// Transfers a Token-2022 position NFT from the deployer to `dest` (resolving any
// transfer hook), then drains the deployer's remaining SOL to `dest`. Finds the
// NFT's real holder account rather than assuming a standard ATA.
// Usage: npx tsx move-position.ts <config.json>
//   { rpcUrl, keypairPath, nftMint, dest }
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
  TOKEN_PROGRAM_ID,
  TOKEN_2022_PROGRAM_ID,
  getAssociatedTokenAddressSync,
  createAssociatedTokenAccountIdempotentInstruction,
  createTransferCheckedWithTransferHookInstruction,
} from "@solana/spl-token";

interface Cfg { rpcUrl: string; keypairPath: string; nftMint: string; dest: string; }
const cfg: Cfg = JSON.parse(readFileSync(process.argv[2] ?? "move-position.config.json", "utf8"));
const expand = (p: string) => p.replace(/^~/, process.env.HOME ?? "");
const connection = new Connection(cfg.rpcUrl, { commitment: "confirmed" });
const deployer = Keypair.fromSecretKey(Uint8Array.from(JSON.parse(readFileSync(expand(cfg.keypairPath), "utf8"))));
const nft = new PublicKey(cfg.nftMint);
const dest = new PublicKey(cfg.dest);

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));
const isRL = (e: unknown) => /429|too many requests|rate limit/i.test(String((e as Error)?.message ?? e));
async function withRetry<T>(fn: () => Promise<T>, max = 8): Promise<T> {
  let d = 800;
  for (let a = 1; ; a++) {
    try { return await fn(); }
    catch (e) { if (a >= max || !isRL(e)) throw e; await sleep(d); d = Math.min(d * 2, 8000); }
  }
}
async function sendTx(ixs: TransactionInstruction[], label: string): Promise<string> {
  for (let attempt = 1; attempt <= 5; attempt++) {
    const { blockhash } = await withRetry(() => connection.getLatestBlockhash("confirmed"));
    const tx = new Transaction().add(...ixs);
    tx.recentBlockhash = blockhash;
    tx.feePayer = deployer.publicKey;
    tx.sign(deployer);
    let sig: string;
    try { sig = await withRetry(() => connection.sendRawTransaction(tx.serialize(), { skipPreflight: false, maxRetries: 3 })); }
    catch (e) { if (isRL(e) && attempt < 5) { await sleep(800); continue; } throw e; }
    const deadline = Date.now() + 45_000;
    for (;;) {
      await sleep(1000);
      const st = await withRetry(() => connection.getSignatureStatuses([sig]));
      const s = st.value[0];
      if (s?.err) throw new Error(`${label} on-chain error: ${JSON.stringify(s.err)}`);
      if (s && (s.confirmationStatus === "confirmed" || s.confirmationStatus === "finalized")) return sig;
      if (Date.now() > deadline) break;
    }
  }
  throw new Error(`${label}: could not confirm`);
}

const mintInfo = await withRetry(() => connection.getAccountInfo(nft));
const prog = mintInfo?.owner.equals(TOKEN_2022_PROGRAM_ID) ? TOKEN_2022_PROGRAM_ID : TOKEN_PROGRAM_ID;
console.log(`position NFT ${nft.toBase58()} — token program ${prog.toBase58()}`);

const largest = await withRetry(() => connection.getTokenLargestAccounts(nft));
const holder = largest.value.find((a) => a.amount === "1") ?? largest.value[0];
if (!holder) throw new Error("no holder account found for the position NFT");
const src = holder.address;
const srcInfo = await withRetry(() => connection.getParsedAccountInfo(src));
const owner = (srcInfo.value?.data as { parsed?: { info?: { owner?: string } } } | undefined)?.parsed?.info?.owner;
console.log(`holder account ${src.toBase58()} owned by ${owner}`);
if (owner !== deployer.publicKey.toBase58()) throw new Error(`deployer does not own the NFT (owner=${owner}) — nothing to move`);

const destAta = getAssociatedTokenAddressSync(nft, dest, false, prog);
const createAta = createAssociatedTokenAccountIdempotentInstruction(deployer.publicKey, destAta, dest, nft, prog);
const transferIx = await createTransferCheckedWithTransferHookInstruction(
  connection, src, nft, destAta, deployer.publicKey, 1n, 0, [], "confirmed", prog,
);
const sig = await sendTx([createAta, transferIx], "transfer_position_NFT");
console.log(`OK   position NFT -> ${dest.toBase58()} · ${sig}`);

// Drain the deployer's remaining SOL to dest (fully close out).
const bal = BigInt(await withRetry(() => connection.getBalance(deployer.publicKey)));
const amount = bal - 5_000n;
if (amount > 0n) {
  const sig2 = await sendTx([SystemProgram.transfer({ fromPubkey: deployer.publicKey, toPubkey: dest, lamports: Number(amount) })], "drain_SOL");
  console.log(`OK   drain ${(Number(amount) / LAMPORTS_PER_SOL).toFixed(6)} SOL -> ${dest.toBase58()} · ${sig2}`);
} else {
  console.log("nothing left to drain");
}
console.log("\nDONE");
