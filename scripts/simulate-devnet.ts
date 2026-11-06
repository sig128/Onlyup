// Live devnet load test for ONLY UP: funds a swarm of wallets and fires a few
// hundred to a few thousand real buys and sells at the deployed program, then
// proves — on-chain, every single trade — that the floor never dropped and the
// vault stayed solvent. This is the dry run before mainnet.
//
// Prereqs: the program is deployed on devnet and a market is launched (deploy.ts),
// and <bankKeypair> is a devnet wallet with enough SOL to fund the traders.
//
// Usage:
//   npx tsx simulate-devnet.ts <rpcUrl> <bankKeypair.json> <mint> [trades] [traders] [fundSolEach]
// e.g.
//   npx tsx simulate-devnet.ts https://api.devnet.solana.com ~/.config/solana/id.json <MINT> 1000 20 2
import { readFileSync } from "node:fs";
import {
  Connection,
  Keypair,
  LAMPORTS_PER_SOL,
  PublicKey,
  SystemProgram,
  Transaction,
  sendAndConfirmTransaction,
} from "@solana/web3.js";
import {
  createAssociatedTokenAccountIdempotentInstruction,
  getAssociatedTokenAddressSync,
} from "@solana/spl-token";
import * as lib from "./lib.js";

const [rpcUrl, bankPath, mintArg, tradesArg, tradersArg, fundArg] = process.argv.slice(2);
if (!rpcUrl || !bankPath || !mintArg) {
  console.error("Usage: npx tsx simulate-devnet.ts <rpcUrl> <bankKeypair.json> <mint> [trades] [traders] [fundSolEach]");
  process.exit(1);
}
const TRADES = parseInt(tradesArg ?? "1000", 10);
const NTRADERS = parseInt(tradersArg ?? "20", 10);
const FUND = Math.max(0.2, parseFloat(fundArg ?? "2"));

const connection = new Connection(rpcUrl, "confirmed");
const bank = Keypair.fromSecretKey(Uint8Array.from(JSON.parse(readFileSync(bankPath.replace(/^~/, process.env.HOME ?? ""), "utf8"))));
const mint = new PublicKey(mintArg);

const rndInt = (n: number) => Math.floor(Math.random() * n);
const sol = (l: bigint) => (Number(l) / LAMPORTS_PER_SOL).toFixed(6);
// Exact floor ratio compare (rf/s), no floats: newRf/newS >= oldRf/oldS.
const floorNotLower = (oldRf: bigint, oldS: bigint, newRf: bigint, newS: bigint) =>
  newS === 0n ? true : newRf * oldS >= oldRf * newS;

interface Trader {
  kp: Keypair;
  ata: PublicKey;
  tokens: bigint; // our local view; refreshed from chain before selling
}

async function fundTraders(): Promise<Trader[]> {
  const bal = await connection.getBalance(bank.publicKey);
  const need = FUND * NTRADERS + 0.5;
  console.log(`Bank ${bank.publicKey.toBase58()} has ${bal / LAMPORTS_PER_SOL} SOL (need ~${need}).`);
  if (bal / LAMPORTS_PER_SOL < need) throw new Error(`Bank underfunded: fund it with ~${need} devnet SOL first.`);

  const traders: Trader[] = [];
  for (let i = 0; i < NTRADERS; i++) {
    const kp = Keypair.generate();
    await sendAndConfirmTransaction(
      connection,
      new Transaction().add(
        SystemProgram.transfer({ fromPubkey: bank.publicKey, toPubkey: kp.publicKey, lamports: Math.round(FUND * LAMPORTS_PER_SOL) }),
      ),
      [bank],
    );
    traders.push({ kp, ata: getAssociatedTokenAddressSync(mint, kp.publicKey), tokens: 0n });
    process.stdout.write(`\rFunded ${i + 1}/${NTRADERS} traders`);
  }
  console.log("");
  return traders;
}

async function main() {
  const programInfo = await connection.getAccountInfo(lib.PROGRAM_ID);
  if (!programInfo?.executable) throw new Error(`Program not deployed on ${rpcUrl}`);
  const market = lib.marketPda(mint);
  const vault = lib.vaultPda(market);

  const traders = await fundTraders();

  let m = await lib.fetchMarket(connection, mint);
  let supply = BigInt((await connection.getTokenSupply(mint)).value.amount);
  let prevRf = m.floorReserve;
  let prevS = supply;
  const startFloor = supply > 0n ? lib.floorPrice(m.floorReserve, supply) : 0;

  let buys = 0, sells = 0, fails = 0, floorDrops = 0, solvencyFails = 0;
  let minFloorRatioOk = true;

  for (let t = 0; t < TRADES; t++) {
    const trader = traders[rndInt(traders.length)];
    // refresh this trader's token balance
    try {
      trader.tokens = BigInt((await connection.getTokenAccountBalance(trader.ata)).value.amount);
    } catch {
      trader.tokens = 0n;
    }
    const wantSell = trader.tokens > 0n && Math.random() < 0.45;

    try {
      if (wantSell) {
        const tokensIn = trader.tokens > 1n ? trader.tokens / BigInt(1 + rndInt(3)) : trader.tokens;
        const q = lib.quoteSellMarket(m, supply, tokensIn);
        if (!q || q.userOut === 0n) { continue; }
        const minOut = (q.userOut * 95n) / 100n;
        await sendAndConfirmTransaction(
          connection,
          new Transaction().add(lib.sellIx(trader.kp.publicKey, mint, trader.ata, tokensIn, minOut)),
          [trader.kp],
        );
        sells++;
      } else {
        const bal = await connection.getBalance(trader.kp.publicKey);
        const spend = BigInt(Math.min(bal - 5_000_000, Math.round((0.01 + Math.random() * 0.3) * LAMPORTS_PER_SOL)));
        if (spend <= 0n) { continue; }
        const q = lib.quoteBuyMarket(m, supply, spend);
        if (!q || q.tokensOut === 0n) { continue; }
        const minOut = (q.tokensOut * 95n) / 100n;
        await sendAndConfirmTransaction(
          connection,
          new Transaction().add(
            createAssociatedTokenAccountIdempotentInstruction(trader.kp.publicKey, trader.ata, trader.kp.publicKey, mint),
            lib.buyIx(trader.kp.publicKey, mint, trader.ata, spend, minOut),
          ),
          [trader.kp],
        );
        buys++;
      }
    } catch {
      fails++;
      continue;
    }

    // Re-read state and verify the two invariants that must hold forever.
    m = await lib.fetchMarket(connection, mint);
    supply = BigInt((await connection.getTokenSupply(mint)).value.amount);
    if (!floorNotLower(prevRf, prevS, m.floorReserve, supply)) {
      floorDrops++;
      minFloorRatioOk = false;
      console.error(`\n❌ FLOOR DROPPED at trade ${t}: ${prevRf}/${prevS} -> ${m.floorReserve}/${supply}`);
    }
    const vaultLamports = BigInt(await connection.getBalance(vault));
    if (vaultLamports < m.curveSol + m.floorReserve + m.creatorFeesOwed) {
      solvencyFails++;
      console.error(`\n❌ VAULT SHORT at trade ${t}: ${vaultLamports} < ${m.curveSol + m.floorReserve + m.creatorFeesOwed}`);
    }
    prevRf = m.floorReserve;
    prevS = supply;

    if ((t + 1) % 25 === 0) {
      const fp = supply > 0n ? lib.floorPrice(m.floorReserve, supply) : 0;
      process.stdout.write(`\r${t + 1}/${TRADES} trades · buys ${buys} sells ${sells} fails ${fails} · floor ${fp.toExponential(4)} SOL/UP · drops ${floorDrops}`);
    }
  }

  const endFloor = supply > 0n ? lib.floorPrice(m.floorReserve, supply) : 0;
  console.log("\n\n===== ONLY UP devnet load test =====");
  console.log(`Trades sent:          ${buys + sells} (buys ${buys}, sells ${sells}; ${fails} failed/skipped)`);
  console.log(`Floor start -> end:   ${startFloor.toExponential(6)} -> ${endFloor.toExponential(6)} SOL/UP`);
  console.log(`Floor reserve:        ${sol(m.floorReserve)} SOL`);
  console.log(`Curve reserve:        ${sol(m.curveSol)} SOL`);
  console.log(`Circulating supply:   ${(Number(supply) / 10 ** lib.DECIMALS).toLocaleString()} UP`);
  console.log(`Floor drops:          ${floorDrops}  ${floorDrops === 0 ? "✅" : "❌"}`);
  console.log(`Solvency failures:    ${solvencyFails}  ${solvencyFails === 0 ? "✅" : "❌"}`);
  console.log(`Floor only went up:   ${minFloorRatioOk && endFloor >= startFloor ? "✅ YES" : "❌ NO"}`);
  if (floorDrops || solvencyFails) process.exit(1);
}

main().catch((e) => {
  console.error(e);
  process.exit(1);
});
