// Read-only: lists the top UP holders and flags which are you, the pool, or
// external buyers — so you know whether pulling pool liquidity would strand
// anyone. Usage: npx tsx holders.ts <rpc> <mint> <pool> <you>
import { Connection, PublicKey } from "@solana/web3.js";
import { CpAmm } from "@meteora-ag/cp-amm-sdk";

const [rpc, mintArg, poolArg, youArg] = process.argv.slice(2);
if (!rpc || !mintArg) {
  console.error("Usage: npx tsx holders.ts <rpc> <mint> [pool] [you]");
  process.exit(1);
}
const connection = new Connection(rpc, "confirmed");
const mint = new PublicKey(mintArg);

const supply = (await connection.getTokenSupply(mint)).value;
console.log(`UP supply: ${supply.uiAmountString}`);

let upVault = "";
if (poolArg) {
  try {
    const cp = new CpAmm(connection);
    const ps = await cp.fetchPoolState(new PublicKey(poolArg));
    upVault = (ps.tokenAMint.equals(mint) ? ps.tokenAVault : ps.tokenBVault).toBase58();
  } catch { /* ignore */ }
}

const largest = await connection.getTokenLargestAccounts(mint);
console.log(`top ${largest.value.length} holder accounts:`);
let external = 0;
let externalCount = 0;
for (const a of largest.value) {
  if (!a.uiAmount) continue;
  const acc = await connection.getParsedAccountInfo(a.address);
  const owner = (acc.value?.data as { parsed?: { info?: { owner?: string } } } | undefined)?.parsed?.info?.owner ?? "?";
  let label = "EXTERNAL (someone else)";
  if (a.address.toBase58() === upVault) label = "POOL vault";
  else if (youArg && owner === youArg) label = "YOU (dest wallet)";
  if (label.startsWith("EXTERNAL")) { external += a.uiAmount; externalCount++; }
  console.log(`  ${a.uiAmountString} UP · acct ${a.address.toBase58()} · owner ${owner} · ${label}`);
}
console.log(`\nEXTERNAL holders (not you, not the pool): ${externalCount} account(s), ${external.toLocaleString()} UP`);
console.log(external > 0 ? "=> Someone else is holding UP." : "=> No external holders detected in the top accounts.");
