// Prints the live state of an ONLY UP v2 market.
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
const tc = m.totalSupply - supply;
const sol = (l: bigint) => (Number(l) / LAMPORTS_PER_SOL).toFixed(6);

console.log(`ONLY UP market ${lib.marketPda(mint).toBase58()} (v${m.version})`);
console.log(`  Mint (CA):            ${m.mint.toBase58()}`);
console.log(`  Curve reserve:        ${sol(m.curveSol)} SOL`);
console.log(`  Floor reserve:        ${sol(m.floorReserve)} SOL   (redeemable, only ever rises)`);
console.log(`  Circulating supply:   ${(Number(supply) / 10 ** lib.DECIMALS).toLocaleString()} UP`);
console.log(`  Curve price:          ${lib.curvePrice(m.curveSol, tc).toExponential(6)} SOL / UP`);
console.log(`  Floor price:          ${lib.floorPrice(m.floorReserve, supply).toExponential(6)} SOL / UP   (always redeemable)`);
console.log(`  Fees:                 3% each side (0.5% creator + 2.5% floor); ${Number(lib.FLOOR_BPS) / 100}% of every buy also funds the floor`);
console.log(`  Trades:               ${m.tradeCount}, volume ${sol(m.volumeLamports)} SOL`);
console.log(`  Creator fees waiting: ${sol(m.creatorFeesOwed)} SOL`);
console.log(`  Buying paused:        ${m.buysPaused}`);
console.log(`  Admin:                ${m.admin.equals(PublicKey.default) ? "renounced (trustless)" : m.admin.toBase58()}`);
console.log(`  Treasury:             ${m.treasury.toBase58()}`);
