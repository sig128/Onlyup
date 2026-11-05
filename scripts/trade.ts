// Buy or sell ONLY UP from a wallet file, with slippage protection (default 1%).
// Usage:
//   npx tsx trade.ts <rpcUrl> <keypair.json> <mint> buy <sol> [slippage%]
//   npx tsx trade.ts <rpcUrl> <keypair.json> <mint> sell <tokens|all> [slippage%]
import { readFileSync } from "node:fs";
import { Connection, Keypair, LAMPORTS_PER_SOL, PublicKey, Transaction, sendAndConfirmTransaction } from "@solana/web3.js";
import { createAssociatedTokenAccountIdempotentInstruction, getAssociatedTokenAddressSync } from "@solana/spl-token";
import * as lib from "./lib.js";

const [rpcUrl, keypairPath, mintArg, side, amountArg, slippageArg] = process.argv.slice(2);
if (!rpcUrl || !keypairPath || !mintArg || !side || !amountArg) {
  console.error("Usage: npx tsx trade.ts <rpcUrl> <keypair.json> <mint> buy <sol> | sell <tokens|all> [slippage%]");
  process.exit(1);
}
const slippageBps = BigInt(Math.round(Number(slippageArg ?? "1") * 100));
if (slippageBps < 0n || slippageBps >= lib.BPS) throw new Error("slippage must be between 0 and 100%");
const withSlippage = (v: bigint) => (v * (lib.BPS - slippageBps)) / lib.BPS;

const connection = new Connection(rpcUrl, "confirmed");
const wallet = Keypair.fromSecretKey(
  Uint8Array.from(JSON.parse(readFileSync(keypairPath.replace(/^~/, process.env.HOME ?? ""), "utf8"))),
);
const mint = new PublicKey(mintArg);
const ata = getAssociatedTokenAddressSync(mint, wallet.publicKey);

const m = await lib.fetchMarket(connection, mint);
const supply = BigInt((await connection.getTokenSupply(mint)).value.amount);
const tx = new Transaction();

if (side === "buy") {
  if (m.buysPaused) throw new Error("buying is paused right now (selling always works)");
  const solIn = BigInt(Math.round(Number(amountArg) * LAMPORTS_PER_SOL));
  const q = lib.quoteBuyMarket(m, supply, solIn);
  if (!q || q.tokensOut === 0n) throw new Error("amount too small");
  console.log(
    `Buying about ${Number(q.tokensOut) / 10 ** lib.DECIMALS} tokens ` +
      `(fee ${m.buyFeeBps / 100}% + creator ${m.teamFeeBps / 100}%)`,
  );
  tx.add(
    createAssociatedTokenAccountIdempotentInstruction(wallet.publicKey, ata, wallet.publicKey, mint),
    lib.buyIx(wallet.publicKey, mint, ata, solIn, withSlippage(q.tokensOut)),
  );
} else if (side === "sell") {
  const held = BigInt((await connection.getTokenAccountBalance(ata)).value.amount);
  const tokensIn = amountArg === "all" ? held : BigInt(Math.round(Number(amountArg) * 10 ** lib.DECIMALS));
  if (tokensIn > held) throw new Error(`you only hold ${Number(held) / 10 ** lib.DECIMALS} tokens`);
  const q = lib.quoteSellMarket(m, supply, tokensIn);
  if (!q || q.userOut === 0n) throw new Error("amount too small");
  console.log(`Selling for about ${Number(q.userOut) / LAMPORTS_PER_SOL} SOL`);
  tx.add(lib.sellIx(wallet.publicKey, mint, ata, tokensIn, withSlippage(q.userOut)));
} else {
  throw new Error("side must be buy or sell");
}

const sig = await sendAndConfirmTransaction(connection, tx, [wallet]);
console.log(`Done: ${sig}`);
