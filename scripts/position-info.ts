// Read-only: finds where an NFT (e.g. a Meteora position NFT) actually lives —
// its mint's token program, and the account + wallet holding it — so a transfer
// can be built against the real holder. Usage:
//   npx tsx position-info.ts <rpc> <nftMint> [expectedOwner]
import { Connection, PublicKey } from "@solana/web3.js";

const [rpc, nftArg, ownerArg] = process.argv.slice(2);
if (!rpc || !nftArg) {
  console.error("Usage: npx tsx position-info.ts <rpc> <nftMint> [expectedOwner]");
  process.exit(1);
}
const connection = new Connection(rpc, "confirmed");
const nft = new PublicKey(nftArg);

const mintInfo = await connection.getAccountInfo(nft);
console.log("NFT mint:", nft.toBase58());
console.log("  mint owner (token program):", mintInfo?.owner.toBase58() ?? "NOT FOUND");
console.log("  mint data length:", mintInfo?.data.length ?? 0, "(82 = classic SPL, >82 = Token-2022 with extensions)");

const largest = await connection.getTokenLargestAccounts(nft);
console.log("holder accounts:", largest.value.length);
for (const a of largest.value) {
  if (a.uiAmount === 0) continue;
  const acc = await connection.getParsedAccountInfo(a.address);
  const parsed = acc.value?.data as { parsed?: { info?: { owner?: string } } } | undefined;
  console.log("  account:", a.address.toBase58());
  console.log("    amount:", a.amount);
  console.log("    held-by token program:", acc.value?.owner.toBase58());
  console.log("    wallet owner:", parsed?.parsed?.info?.owner ?? "(unparsed)");
}
if (ownerArg) console.log("expected owner (deployer):", ownerArg);
