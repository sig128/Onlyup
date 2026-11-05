// Prints the live state of an ONLY UP market.
// Usage: npx tsx status.ts <rpcUrl> <mint>
import { Connection, LAMPORTS_PER_SOL, PublicKey } from "@solana/web3.js";
import * as lib from "./lib.js";

const [rpcUrl, mintArg] = process.argv.slice(2);
if (!rpcUrl || !mintArg) {
  console.error("Usage: npx tsx status.ts <rpcUrl> <mint>");
  process.exit(1);
}
const connection = new Connection(rpcUrl, "confirmed");
const mint = new PublicKey(mintArg);

const m = await lib.fetchMarket(connection, mint);
const supply = BigInt((await connection.getTokenSupply(mint)).value.amount);
const sol = (l: bigint) => (Number(l) / LAMPORTS_PER_SOL).toFixed(4);
const pct = (bps: number) => `${(bps / 100).toFixed(2)}%`;

console.log(`ONLY UP market ${lib.marketPda(mint).toBase58()}`);
console.log(`  SOL in the safe:     ${sol(m.reserve)} SOL (= market cap)`);
console.log(`  Tokens:              ${(Number(supply) / 10 ** lib.DECIMALS).toLocaleString()}`);
console.log(`  Price:               ${lib.priceSolPerToken(m.reserve, supply).toExponential(6)} SOL per token`);
console.log(`  Fees:                buy ${pct(m.buyFeeBps)}, sell ${pct(m.sellFeeBps)}, creator ${pct(m.teamFeeBps)} each side`);
console.log(`  Trades:              ${m.tradeCount}, volume ${sol(m.volumeLamports)} SOL`);
console.log(`  Creator fees waiting: ${sol(m.teamFeesOwed)} SOL`);
console.log(`  Buying paused:       ${m.buysPaused}`);
console.log(`  Admin:               ${m.admin.equals(PublicKey.default) ? "renounced" : m.admin.toBase58()}`);
