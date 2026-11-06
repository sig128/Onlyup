// ONLY UP peg keeper. The token's real price lives on our bonding-curve
// contract; the Meteora DAMM v2 pool is the mirror that Axiom / DexScreener /
// Jupiter actually index. This bot watches both prices and trades the pool back
// toward the contract price whenever they drift apart, so the chart the world
// sees tracks the real curve.
//
// It is a market-maker: it needs inventory on the bot wallet — some SOL (wrapped
// to wSOL) to buy UP when the pool is cheap, and some UP to sell when the pool is
// rich. Run it as a long-lived service (see arb-bot section of the README).
//
// Meteora cp-amm is live on devnet + mainnet, so this is validated on devnet
// first, against a real pool, before it ever touches mainnet.
//
// Usage: npx tsx arb-bot.ts arb.config.json
import { readFileSync, existsSync } from "node:fs";
import { Connection, Keypair, LAMPORTS_PER_SOL, PublicKey, sendAndConfirmTransaction } from "@solana/web3.js";
import { NATIVE_MINT, getAssociatedTokenAddressSync } from "@solana/spl-token";
import BN from "bn.js";
import {
  CpAmm,
  SwapMode,
  getCurrentPoint,
  getPriceFromSqrtPrice,
  getTokenProgram,
  type ActivationType,
} from "@meteora-ag/cp-amm-sdk";
import * as lib from "./lib.js";

interface ArbConfig {
  rpcUrl: string;
  keypairPath: string;
  /** UP mint. Defaults to launch-output.json. */
  mint?: string;
  /** DAMM v2 pool address. Defaults to pool-output.json. */
  pool?: string;
  /** Re-peg when |pool - contract| / contract exceeds this (bps). Default 100 = 1%. */
  thresholdBps?: number;
  /** Max SOL-equivalent to move in a single correction. Default 0.25. */
  maxTradeSol?: number;
  /** Poll interval, ms. Default 5000. */
  intervalMs?: number;
  /** Swap slippage, bps. Default 100 = 1%. */
  slippageBps?: number;
}

const expand = (p: string) => p.replace(/^~/, process.env.HOME ?? "");
const cfg: ArbConfig = JSON.parse(readFileSync(process.argv[2] ?? "arb.config.json", "utf8"));
const wallet = Keypair.fromSecretKey(Uint8Array.from(JSON.parse(readFileSync(expand(cfg.keypairPath), "utf8"))));
const connection = new Connection(cfg.rpcUrl, "confirmed");

function resolve(file: string, key: string, fallback?: string): PublicKey {
  if (fallback) return new PublicKey(fallback);
  if (existsSync(file)) return new PublicKey(JSON.parse(readFileSync(file, "utf8"))[key]);
  throw new Error(`need ${key}: pass it in the config or run the launch/pool script first`);
}
const up = resolve("launch-output.json", "mint", cfg.mint);
const poolAddr = resolve("pool-output.json", "pool", cfg.pool);
const thresholdBps = cfg.thresholdBps ?? 100;
const maxTradeSol = cfg.maxTradeSol ?? 0.25;
const intervalMs = cfg.intervalMs ?? 5000;
const slippageBps = cfg.slippageBps ?? 100;

const cpAmm = new CpAmm(connection);
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

/** Contract curve price, SOL per UP. */
async function contractPrice(): Promise<number> {
  const m = await lib.fetchMarket(connection, up);
  const supply = BigInt((await connection.getTokenSupply(up)).value.amount);
  return lib.curvePrice(m.curveSol, m.totalSupply - supply);
}

async function main() {
  const poolState = await cpAmm.fetchPoolState(poolAddr);
  const upIsA = poolState.tokenAMint.equals(up);
  const decA = upIsA ? lib.DECIMALS : 9;
  const decB = upIsA ? 9 : lib.DECIMALS;
  const solMint = NATIVE_MINT;
  console.log(`Peg keeper up. UP=${up.toBase58()} pool=${poolAddr.toBase58()} (UP is token${upIsA ? "A" : "B"})`);
  console.log(`threshold ${thresholdBps}bps · max ${maxTradeSol} SOL/move · every ${intervalMs}ms`);

  // eslint-disable-next-line no-constant-condition
  for (;;) {
    try {
      const ps = await cpAmm.fetchPoolState(poolAddr);
      // pool price as SOL per UP (getPriceFromSqrtPrice gives tokenB per tokenA)
      const priceAB = Number(getPriceFromSqrtPrice(ps.sqrtPrice, decA, decB).toString());
      const poolPrice = upIsA ? priceAB : 1 / priceAB;
      const curve = await contractPrice();
      const gap = (poolPrice - curve) / curve;
      const tag = `pool ${poolPrice.toExponential(4)} vs curve ${curve.toExponential(4)} (${(gap * 100).toFixed(2)}%)`;

      if (Math.abs(gap) * 10000 < thresholdBps) {
        console.log(`${new Date().toISOString()}  in peg · ${tag}`);
        await sleep(intervalMs);
        continue;
      }

      // Pool too rich -> sell UP into it (input UP). Pool too cheap -> buy UP (input wSOL).
      const sellUp = gap > 0;
      const inputMint = sellUp ? up : solMint;
      const amountIn = sellUp
        ? new BN(Math.round((maxTradeSol / poolPrice) * 10 ** lib.DECIMALS).toString()) // UP base units
        : new BN(Math.round(maxTradeSol * LAMPORTS_PER_SOL).toString()); // lamports of wSOL

      // make sure we actually hold the input
      const inAta = getAssociatedTokenAddressSync(inputMint, wallet.publicKey);
      let held = 0n;
      try {
        held = BigInt((await connection.getTokenAccountBalance(inAta)).value.amount);
      } catch {
        held = 0n;
      }
      if (held < BigInt(amountIn.toString())) {
        console.warn(`${new Date().toISOString()}  low inventory of ${sellUp ? "UP" : "wSOL"} — top up the bot wallet · ${tag}`);
        await sleep(intervalMs);
        continue;
      }

      const currentPoint = await getCurrentPoint(connection, ps.activationType as ActivationType);
      const quote = cpAmm.getQuote2({
        inputTokenMint: inputMint,
        poolState: ps,
        currentPoint,
        amountIn,
        slippage: slippageBps,
        swapMode: SwapMode.ExactIn,
        tokenADecimal: decA,
        tokenBDecimal: decB,
        hasReferral: false,
      });
      const swapTx = await cpAmm.swap2({
        payer: wallet.publicKey,
        pool: poolAddr,
        inputTokenMint: inputMint,
        outputTokenMint: inputMint.equals(ps.tokenAMint) ? ps.tokenBMint : ps.tokenAMint,
        tokenAMint: ps.tokenAMint,
        tokenBMint: ps.tokenBMint,
        tokenAVault: ps.tokenAVault,
        tokenBVault: ps.tokenBVault,
        tokenAProgram: getTokenProgram(ps.tokenAFlag),
        tokenBProgram: getTokenProgram(ps.tokenBFlag),
        referralTokenAccount: null,
        poolState: ps,
        swapMode: SwapMode.ExactIn,
        amountIn,
        minimumAmountOut: quote.minimumAmountOut ?? new BN(0),
      });
      swapTx.feePayer = wallet.publicKey;
      swapTx.recentBlockhash = (await connection.getLatestBlockhash()).blockhash;
      const sig = await sendAndConfirmTransaction(connection, swapTx, [wallet]);
      console.log(`${new Date().toISOString()}  re-pegged: ${sellUp ? "sold UP" : "bought UP"} · ${tag} · ${sig}`);
    } catch (e) {
      console.error(`${new Date().toISOString()}  tick error:`, (e as Error).message);
    }
    await sleep(intervalMs);
  }
}

main().catch((e) => {
  console.error(e);
  process.exit(1);
});
