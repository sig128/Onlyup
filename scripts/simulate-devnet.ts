// Live devnet load test for ONLY UP: funds a swarm of wallets and fires a few
// hundred to a few thousand real buys and sells at the deployed program, then
// proves — on-chain, every single trade — that the floor never dropped and the
// vault stayed solvent. This is the dry run before mainnet.
//
// Built to survive the free public devnet RPC, which rate-limits hard (429):
//   - every RPC call is throttled + retried with exponential backoff
//   - transactions are confirmed by HTTP polling, never the websocket (the
//     public endpoint 429s the ws subscription almost immediately)
//   - a transient failure is counted and skipped, never fatal; the floor
//     invariant is transitive, so comparing a later state to the last good
//     baseline across a skipped read is still a valid monotonicity check
//
// Prereqs: the program is deployed on devnet and a market is launched (deploy.ts),
// and <bankKeypair> is a devnet wallet with enough SOL to fund the traders.
//
// Usage:
//   npx tsx simulate-devnet.ts <rpcUrl> <bankKeypair.json> <mint> [trades] [traders] [fundSolEach]
import { readFileSync } from "node:fs";
import {
  Connection,
  Keypair,
  LAMPORTS_PER_SOL,
  PublicKey,
  SystemProgram,
  Transaction,
  TransactionInstruction,
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

// No websocket: the public devnet RPC rate-limits ws subscriptions instantly.
const connection = new Connection(rpcUrl, { commitment: "confirmed" });
const bank = Keypair.fromSecretKey(Uint8Array.from(JSON.parse(readFileSync(bankPath.replace(/^~/, process.env.HOME ?? ""), "utf8"))));
const mint = new PublicKey(mintArg);

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));
const rndInt = (n: number) => Math.floor(Math.random() * n);
const sol = (l: bigint) => (Number(l) / LAMPORTS_PER_SOL).toFixed(6);
// Exact floor ratio compare (rf/s), no floats: newRf/newS >= oldRf/oldS.
const floorNotLower = (oldRf: bigint, oldS: bigint, newRf: bigint, newS: bigint) =>
  newS === 0n ? true : newRf * oldS >= oldRf * newS;

// ---- rate-limit-resilient RPC plumbing ----
const MIN_SPACING = 200; // ms between any two RPC calls
let lastCall = 0;
async function throttle() {
  const wait = MIN_SPACING - (Date.now() - lastCall);
  if (wait > 0) await sleep(wait);
  lastCall = Date.now();
}
const isRateLimit = (e: unknown) => {
  const m = String((e as Error)?.message ?? e);
  return m.includes("429") || /too many requests/i.test(m) || /rate limit/i.test(m);
};
async function withRetry<T>(fn: () => Promise<T>, max = 8): Promise<T> {
  let delay = 800;
  for (let attempt = 1; ; attempt++) {
    await throttle();
    try {
      return await fn();
    } catch (e) {
      if (attempt >= max || !isRateLimit(e)) throw e;
      await sleep(delay + rndInt(400));
      delay = Math.min(delay * 2, 8000);
    }
  }
}

/** Send + confirm by HTTP polling (no websocket). Rebuilds on blockhash expiry. */
async function sendTx(ixs: TransactionInstruction[], signers: Keypair[]): Promise<string> {
  for (let attempt = 1; attempt <= 5; attempt++) {
    const { blockhash } = await withRetry(() => connection.getLatestBlockhash("confirmed"));
    const tx = new Transaction().add(...ixs);
    tx.recentBlockhash = blockhash;
    tx.feePayer = signers[0].publicKey;
    tx.sign(...signers);
    let sig: string;
    try {
      sig = await withRetry(() => connection.sendRawTransaction(tx.serialize(), { skipPreflight: false, maxRetries: 3 }));
    } catch (e) {
      if (attempt >= 5 || isRateLimit(e)) { if (attempt >= 5) throw e; await sleep(800); continue; }
      throw e; // real program error — surface it
    }
    const deadline = Date.now() + 45_000;
    for (;;) {
      await sleep(1000);
      const st = await withRetry(() => connection.getSignatureStatuses([sig]));
      const s = st.value[0];
      if (s?.err) throw new Error("tx failed: " + JSON.stringify(s.err));
      if (s && (s.confirmationStatus === "confirmed" || s.confirmationStatus === "finalized")) return sig;
      if (Date.now() > deadline) break; // likely blockhash expiry → rebuild & resend
    }
  }
  throw new Error("could not confirm tx after retries");
}

interface Trader {
  kp: Keypair;
  ata: PublicKey;
  tokens: bigint;
}

async function fundTraders(): Promise<Trader[]> {
  const bal = await withRetry(() => connection.getBalance(bank.publicKey));
  const need = FUND * NTRADERS + 0.5;
  console.log(`Bank ${bank.publicKey.toBase58()} has ${bal / LAMPORTS_PER_SOL} SOL (need ~${need}).`);
  if (bal / LAMPORTS_PER_SOL < need) throw new Error(`Bank underfunded: fund it with ~${need} devnet SOL first.`);

  const traders: Trader[] = [];
  for (let i = 0; i < NTRADERS; i++) {
    const kp = Keypair.generate();
    await sendTx(
      [SystemProgram.transfer({ fromPubkey: bank.publicKey, toPubkey: kp.publicKey, lamports: Math.round(FUND * LAMPORTS_PER_SOL) })],
      [bank],
    );
    traders.push({ kp, ata: getAssociatedTokenAddressSync(mint, kp.publicKey), tokens: 0n });
    console.log(`Funded ${i + 1}/${NTRADERS} traders`);
  }
  return traders;
}

async function main() {
  const programInfo = await withRetry(() => connection.getAccountInfo(lib.PROGRAM_ID));
  if (!programInfo?.executable) throw new Error(`Program not deployed on ${rpcUrl}`);
  const market = lib.marketPda(mint);
  const vault = lib.vaultPda(market);

  const traders = await fundTraders();

  let m = await withRetry(() => lib.fetchMarket(connection, mint));
  let supply = BigInt((await withRetry(() => connection.getTokenSupply(mint))).value.amount);
  let prevRf = m.floorReserve;
  let prevS = supply;
  const startFloor = supply > 0n ? lib.floorPrice(m.floorReserve, supply) : 0;

  let buys = 0, sells = 0, fails = 0, readErrors = 0, floorDrops = 0, solvencyFails = 0;
  let minFloorRatioOk = true;

  for (let t = 0; t < TRADES; t++) {
    const trader = traders[rndInt(traders.length)];
    try {
      trader.tokens = BigInt((await withRetry(() => connection.getTokenAccountBalance(trader.ata))).value.amount);
    } catch {
      trader.tokens = 0n;
    }
    const wantSell = trader.tokens > 0n && Math.random() < 0.45;

    let traded = false;
    try {
      if (wantSell) {
        const tokensIn = trader.tokens > 1n ? trader.tokens / BigInt(1 + rndInt(3)) : trader.tokens;
        const q = lib.quoteSellMarket(m, supply, tokensIn);
        if (!q || q.userOut === 0n) continue;
        const minOut = (q.userOut * 95n) / 100n;
        await sendTx([lib.sellIx(trader.kp.publicKey, mint, trader.ata, tokensIn, minOut)], [trader.kp]);
        sells++; traded = true;
      } else {
        const bal = await withRetry(() => connection.getBalance(trader.kp.publicKey));
        const spend = BigInt(Math.min(bal - 5_000_000, Math.round((0.01 + Math.random() * 0.3) * LAMPORTS_PER_SOL)));
        if (spend <= 0n) continue;
        const q = lib.quoteBuyMarket(m, supply, spend);
        if (!q || q.tokensOut === 0n) continue;
        const minOut = (q.tokensOut * 95n) / 100n;
        await sendTx(
          [
            createAssociatedTokenAccountIdempotentInstruction(trader.kp.publicKey, trader.ata, trader.kp.publicKey, mint),
            lib.buyIx(trader.kp.publicKey, mint, trader.ata, spend, minOut),
          ],
          [trader.kp],
        );
        buys++; traded = true;
      }
    } catch {
      fails++;
    }
    if (!traded) continue;

    // Re-read state and verify the two invariants that must hold forever. Retried
    // hard; if the RPC still won't answer, keep the last good baseline (the ratio
    // check stays valid across the gap) rather than crashing the run.
    try {
      const nm = await withRetry(() => lib.fetchMarket(connection, mint));
      const ns = BigInt((await withRetry(() => connection.getTokenSupply(mint))).value.amount);
      if (!floorNotLower(prevRf, prevS, nm.floorReserve, ns)) {
        floorDrops++; minFloorRatioOk = false;
        console.error(`\n❌ FLOOR DROPPED at trade ${t}: ${prevRf}/${prevS} -> ${nm.floorReserve}/${ns}`);
      }
      const vaultLamports = BigInt(await withRetry(() => connection.getBalance(vault)));
      if (vaultLamports < nm.curveSol + nm.floorReserve + nm.creatorFeesOwed) {
        solvencyFails++;
        console.error(`\n❌ VAULT SHORT at trade ${t}: ${vaultLamports} < ${nm.curveSol + nm.floorReserve + nm.creatorFeesOwed}`);
      }
      m = nm; supply = ns; prevRf = nm.floorReserve; prevS = ns;
    } catch {
      readErrors++;
    }

    if ((t + 1) % 25 === 0) {
      const fp = supply > 0n ? lib.floorPrice(m.floorReserve, supply) : 0;
      console.log(`${t + 1}/${TRADES} trades · buys ${buys} sells ${sells} fails ${fails} readErr ${readErrors} · floor ${fp.toExponential(4)} SOL/UP · drops ${floorDrops}`);
    }
  }

  const endFloor = supply > 0n ? lib.floorPrice(m.floorReserve, supply) : 0;
  console.log("\n\n===== ONLY UP devnet load test =====");
  console.log(`Trades sent:          ${buys + sells} (buys ${buys}, sells ${sells}; ${fails} failed, ${readErrors} read-skips)`);
  console.log(`Floor start -> end:   ${startFloor.toExponential(6)} -> ${endFloor.toExponential(6)} SOL/UP`);
  console.log(`Floor reserve:        ${sol(m.floorReserve)} SOL`);
  console.log(`Curve reserve:        ${sol(m.curveSol)} SOL`);
  console.log(`Circulating supply:   ${(Number(supply) / 10 ** lib.DECIMALS).toLocaleString()} UP`);
  console.log(`Floor drops:          ${floorDrops}  ${floorDrops === 0 ? "✅" : "❌"}`);
  console.log(`Solvency failures:    ${solvencyFails}  ${solvencyFails === 0 ? "✅" : "❌"}`);
  console.log(`Floor only went up:   ${minFloorRatioOk && endFloor >= startFloor ? "✅ YES" : "❌ NO"}`);
  if (floorDrops || solvencyFails) process.exit(1);
  if (buys + sells === 0) { console.error("No trades landed — RPC was unusable."); process.exit(1); }
}

main().catch((e) => {
  console.error(e);
  process.exit(1);
});
