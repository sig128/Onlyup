// Creates the Meteora DAMM v2 "mirror" pool for ONLY UP, seeded at the exact
// price our bonding-curve contract is at right now. This is the pool that
// Axiom / DexScreener / Jupiter actually index — the arb bot (arb-bot.ts) then
// keeps its price glued to the contract.
//
// Sequence (run right after deploy.ts):
//   1. read the live contract price from our market
//   2. make sure the deployer holds enough UP to seed the pool — buy the
//      shortfall from the contract if needed
//   3. wrap the SOL side into wSOL
//   4. create + seed the DAMM v2 pool at the contract price
//
// Meteora's cp-amm program is live on BOTH devnet and mainnet (same id), so the
// whole thing dry-runs on devnet for free before any mainnet spend.
//
// Usage: npx tsx pool.ts pool.config.json
import { readFileSync, writeFileSync, existsSync } from "node:fs";
import { fileURLToPath } from "node:url";
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
  NATIVE_MINT,
  TOKEN_PROGRAM_ID,
  createAssociatedTokenAccountIdempotentInstruction,
  createSyncNativeInstruction,
  getAssociatedTokenAddressSync,
} from "@solana/spl-token";
import BN from "bn.js";
import {
  ActivationType,
  BaseFeeMode,
  CollectFeeMode,
  CpAmm,
  MAX_SQRT_PRICE,
  MIN_SQRT_PRICE,
  getBaseFeeParams,
  getSqrtPriceFromPrice,
  type PoolFeesParams,
} from "@meteora-ag/cp-amm-sdk";
import * as lib from "./lib.js";

interface PoolConfig {
  rpcUrl: string;
  keypairPath: string;
  /** The UP mint. Defaults to the mint in launch-output.json, else the vanity key. */
  mint?: string;
  /** SOL to put on the pool's SOL side. */
  poolSol: number;
  /** Pool swap fee, basis points (LP revenue for the deployer). Default 100 = 1%. */
  poolFeeBps?: number;
}

const expand = (p: string) => p.replace(/^~/, process.env.HOME ?? "");
const cfg: PoolConfig = JSON.parse(readFileSync(process.argv[2] ?? "pool.config.json", "utf8"));
const deployer = Keypair.fromSecretKey(Uint8Array.from(JSON.parse(readFileSync(expand(cfg.keypairPath), "utf8"))));
const connection = new Connection(cfg.rpcUrl, "confirmed");

// Resolve the UP mint: explicit config → launch-output.json → vanity keypair.
function resolveMint(): PublicKey {
  if (cfg.mint) return new PublicKey(cfg.mint);
  if (existsSync("launch-output.json")) return new PublicKey(JSON.parse(readFileSync("launch-output.json", "utf8")).mint);
  const kp = fileURLToPath(new URL("../keys/v2-mint-keypair.json", import.meta.url));
  return Keypair.fromSecretKey(Uint8Array.from(JSON.parse(readFileSync(kp, "utf8")))).publicKey;
}
const up = resolveMint();
const poolLamports = BigInt(Math.round(cfg.poolSol * LAMPORTS_PER_SOL));
const poolFeeBps = cfg.poolFeeBps ?? 100;

async function ensureUpBalance(need: bigint) {
  const ata = getAssociatedTokenAddressSync(up, deployer.publicKey);
  let held = 0n;
  try {
    held = BigInt((await connection.getTokenAccountBalance(ata)).value.amount);
  } catch {
    /* no ATA yet */
  }
  if (held >= need) return;
  // Buy the shortfall from the contract. Over-buy slightly (2%) to cover rounding/price drift.
  const m = await lib.fetchMarket(connection, up);
  const supply = BigInt((await connection.getTokenSupply(up)).value.amount);
  const shortfall = need - held;
  const price = lib.curvePrice(m.curveSol, m.totalSupply - supply); // SOL per UP
  const solGuess = BigInt(Math.ceil((Number(shortfall) / 10 ** lib.DECIMALS) * price * 1.05 * LAMPORTS_PER_SOL));
  const q = lib.quoteBuyMarket(m, supply, solGuess);
  if (!q || q.tokensOut < shortfall) throw new Error("could not buy enough UP to seed the pool — raise poolSol headroom");
  console.log(`Buying ${Number(q.tokensOut) / 10 ** lib.DECIMALS} UP from the contract to seed the pool`);
  await sendAndConfirmTransaction(
    connection,
    new Transaction().add(
      createAssociatedTokenAccountIdempotentInstruction(deployer.publicKey, ata, deployer.publicKey, up),
      lib.buyIx(deployer.publicKey, up, ata, solGuess, (q.tokensOut * 98n) / 100n),
    ),
    [deployer],
  );
}

async function wrapSol(lamports: bigint) {
  const wsolAta = getAssociatedTokenAddressSync(NATIVE_MINT, deployer.publicKey);
  await sendAndConfirmTransaction(
    connection,
    new Transaction().add(
      createAssociatedTokenAccountIdempotentInstruction(deployer.publicKey, wsolAta, deployer.publicKey, NATIVE_MINT),
      SystemProgram.transfer({ fromPubkey: deployer.publicKey, toPubkey: wsolAta, lamports: Number(lamports) }),
      createSyncNativeInstruction(wsolAta),
    ),
    [deployer],
  );
}

async function main() {
  const cpAmm = new CpAmm(connection);

  // 1. Live contract price (SOL per UP) and the matching UP amount for the pool.
  const m = await lib.fetchMarket(connection, up);
  const supply = BigInt((await connection.getTokenSupply(up)).value.amount);
  const priceSolPerUp = lib.curvePrice(m.curveSol, m.totalSupply - supply);
  if (!(priceSolPerUp > 0)) throw new Error("contract price is zero — launch the token first");
  const upWhole = cfg.poolSol / priceSolPerUp; // UP needed to match poolSol at the contract price
  const upBase = BigInt(Math.round(upWhole * 10 ** lib.DECIMALS));
  console.log(`Contract price: ${priceSolPerUp.toExponential(6)} SOL/UP`);
  console.log(`Seeding pool with ${cfg.poolSol} SOL + ~${upWhole.toLocaleString()} UP`);

  // 2 + 3. Make sure we hold the UP, and wrap the SOL.
  await ensureUpBalance(upBase);
  await wrapSol(poolLamports);

  // 4. Build the pool. DAMM v2 sorts the two mints; assign A/B + price accordingly.
  const upFirst = Buffer.compare(up.toBuffer(), NATIVE_MINT.toBuffer()) < 0;
  const tokenAMint = upFirst ? up : NATIVE_MINT;
  const tokenBMint = upFirst ? NATIVE_MINT : up;
  const tokenADec = upFirst ? lib.DECIMALS : 9;
  const tokenBDec = upFirst ? 9 : lib.DECIMALS;
  const tokenAAmount = new BN((upFirst ? upBase : poolLamports).toString());
  const tokenBAmount = new BN((upFirst ? poolLamports : upBase).toString());
  // price is always tokenB-per-tokenA. UP=A → SOL/UP; SOL=A → UP/SOL.
  const priceStr = (upFirst ? priceSolPerUp : 1 / priceSolPerUp).toFixed(18);
  const initSqrtPrice = getSqrtPriceFromPrice(priceStr, tokenADec, tokenBDec);

  const liquidityDelta = cpAmm.getLiquidityDelta({
    maxAmountTokenA: tokenAAmount,
    maxAmountTokenB: tokenBAmount,
    sqrtPrice: initSqrtPrice,
    sqrtMinPrice: MIN_SQRT_PRICE,
    sqrtMaxPrice: MAX_SQRT_PRICE,
    collectFeeMode: CollectFeeMode.BothToken,
  });

  const baseFee = getBaseFeeParams({
    baseFeeMode: BaseFeeMode.FeeTimeSchedulerExponential,
    feeTimeSchedulerParam: { startingFeeBps: poolFeeBps, endingFeeBps: poolFeeBps, numberOfPeriod: 0, totalDuration: 0 },
  });
  const poolFees: PoolFeesParams = { baseFee, compoundingFeeBps: 0, padding: 0, dynamicFee: null };

  const positionNft = Keypair.generate();
  const { tx, pool, position } = await cpAmm.createCustomPool({
    payer: deployer.publicKey,
    creator: deployer.publicKey,
    positionNft: positionNft.publicKey,
    tokenAMint,
    tokenBMint,
    tokenAAmount,
    tokenBAmount,
    sqrtMinPrice: MIN_SQRT_PRICE,
    sqrtMaxPrice: MAX_SQRT_PRICE,
    liquidityDelta,
    initSqrtPrice,
    poolFees,
    hasAlphaVault: false,
    activationType: ActivationType.Timestamp,
    collectFeeMode: CollectFeeMode.BothToken,
    activationPoint: null,
    tokenAProgram: TOKEN_PROGRAM_ID,
    tokenBProgram: TOKEN_PROGRAM_ID,
    isLockLiquidity: false,
  });
  tx.feePayer = deployer.publicKey;
  tx.recentBlockhash = (await connection.getLatestBlockhash()).blockhash;
  const sig = await sendAndConfirmTransaction(connection, tx, [deployer, positionNft]);

  const out = {
    cluster: cfg.rpcUrl,
    mint: up.toBase58(),
    pool: pool.toBase58(),
    position: position.toBase58(),
    positionNft: positionNft.publicKey.toBase58(),
    poolSol: cfg.poolSol,
    seededUp: upWhole,
    priceSolPerUp,
    poolFeeBps,
    sig,
  };
  writeFileSync("pool-output.json", JSON.stringify(out, null, 2));
  console.log("\n✅ Meteora pool created:\n", out);
}

main().catch((e) => {
  console.error(e);
  process.exit(1);
});
