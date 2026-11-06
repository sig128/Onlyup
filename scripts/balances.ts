// Read-only inventory of every ONLY UP account: SOL + UP in the deployer, the
// treasury, the contract (market), and the Meteora pool — plus a clear split of
// what is withdrawable vs locked as the token's backing. Spends nothing.
// Usage: npx tsx balances.ts <rpc> <mint> <deployer> <treasury> [pool]
import { Connection, LAMPORTS_PER_SOL, PublicKey } from "@solana/web3.js";
import { getAssociatedTokenAddressSync } from "@solana/spl-token";
import { CpAmm } from "@meteora-ag/cp-amm-sdk";
import * as lib from "./lib.js";

const [rpc, mintArg, deployerArg, treasuryArg, poolArg] = process.argv.slice(2);
if (!rpc || !mintArg || !deployerArg || !treasuryArg) {
  console.error("Usage: npx tsx balances.ts <rpc> <mint> <deployer> <treasury> [pool]");
  process.exit(1);
}
const connection = new Connection(rpc, "confirmed");
const mint = new PublicKey(mintArg);
const sol = (l: bigint) => (Number(l) / LAMPORTS_PER_SOL).toFixed(6);
const upf = (b: bigint) => (Number(b) / 10 ** lib.DECIMALS).toLocaleString();

async function solBal(addr: string): Promise<bigint> {
  return BigInt(await connection.getBalance(new PublicKey(addr)));
}
async function upBal(owner: string): Promise<bigint> {
  try {
    return BigInt((await connection.getTokenAccountBalance(getAssociatedTokenAddressSync(mint, new PublicKey(owner)))).value.amount);
  } catch {
    return 0n;
  }
}

const m = await lib.fetchMarket(connection, mint);
const supply = BigInt((await connection.getTokenSupply(mint)).value.amount);

const depSol = await solBal(deployerArg);
const depUp = await upBal(deployerArg);
const treSol = await solBal(treasuryArg);
const treUp = await upBal(treasuryArg);

console.log("===== ONLY UP — full on-chain inventory =====\n");
console.log(`DEPLOYER  ${deployerArg}`);
console.log(`  SOL: ${sol(depSol)}`);
console.log(`  UP:  ${upf(depUp)}\n`);
console.log(`TREASURY / fee wallet  ${treasuryArg}`);
console.log(`  SOL: ${sol(treSol)}`);
console.log(`  UP:  ${upf(treUp)}\n`);
console.log("MARKET (held by the contract)");
console.log(`  creator fees owed (CLAIMABLE to treasury): ${sol(m.creatorFeesOwed)} SOL`);
console.log(`  floor reserve (LOCKED - redeemable by sellers): ${sol(m.floorReserve)} SOL`);
console.log(`  curve reserve (LOCKED - backs the price):       ${sol(m.curveSol)} SOL`);
console.log(`  circulating supply: ${upf(supply)} UP`);
console.log(`  trades: ${m.tradeCount}, volume ${sol(m.volumeLamports)} SOL\n`);

let poolSol = 0n, poolUp = 0n, poolOk = false;
if (poolArg) {
  try {
    const cp = new CpAmm(connection);
    const ps = await cp.fetchPoolState(new PublicKey(poolArg));
    const aIsUp = ps.tokenAMint.equals(mint);
    const aBal = BigInt((await connection.getTokenAccountBalance(ps.tokenAVault)).value.amount);
    const bBal = BigInt((await connection.getTokenAccountBalance(ps.tokenBVault)).value.amount);
    poolUp = aIsUp ? aBal : bBal;
    poolSol = aIsUp ? bBal : aBal;
    poolOk = true;
    console.log(`POOL (your LP - withdrawable, but it IS the tradable liquidity)  ${poolArg}`);
    console.log(`  SOL in pool: ${sol(poolSol)}`);
    console.log(`  UP in pool:  ${upf(poolUp)}\n`);
  } catch (e) {
    console.log(`POOL: could not read - ${(e as Error).message}\n`);
  }
}

const liquidToYou = depSol + treSol + m.creatorFeesOwed;
console.log(`===== WHAT CAN GO TO ${treasuryArg} =====`);
console.log(`  deployer SOL (drain):        ${sol(depSol)} SOL`);
console.log(`  deployer UP (transfer):      ${upf(depUp)} UP`);
console.log(`  creator fees (claim):        ${sol(m.creatorFeesOwed)} SOL`);
console.log(`  already in treasury:         ${sol(treSol)} SOL + ${upf(treUp)} UP`);
console.log(`  --> liquid SOL once swept:   ${sol(liquidToYou)} SOL`);
if (poolOk) console.log(`  + your LP if ever withdrawn: ${sol(poolSol)} SOL + ${upf(poolUp)} UP (removing it pulls pool liquidity)`);
console.log("");
console.log("LOCKED - NOT withdrawable (this is the token's backing / the whole point):");
console.log(`  floor reserve: ${sol(m.floorReserve)} SOL   curve reserve: ${sol(m.curveSol)} SOL`);
