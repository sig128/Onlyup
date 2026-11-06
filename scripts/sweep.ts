// Sweeps the deployer's assets to a destination wallet and repoints the creator-
// fee treasury there. Signs with the deployer keypair. Each action is its own
// confirmed transaction (HTTP-polled, retried on 429), so a partial failure is
// visible and the SOL drain runs last. Spends only what it sends to `dest`.
//
// Usage: npx tsx sweep.ts <config.json>
//   { rpcUrl, keypairPath, mint, dest, setTreasury?, positionNftMint? }
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
  createTransferCheckedInstruction,
} from "@solana/spl-token";
import * as lib from "./lib.js";

interface Cfg {
  rpcUrl: string;
  keypairPath: string;
  mint: string;
  dest: string;
  setTreasury?: boolean;
  positionNftMint?: string;
}
const cfg: Cfg = JSON.parse(readFileSync(process.argv[2] ?? "sweep.config.json", "utf8"));
const expand = (p: string) => p.replace(/^~/, process.env.HOME ?? "");
const connection = new Connection(cfg.rpcUrl, { commitment: "confirmed" });
const deployer = Keypair.fromSecretKey(Uint8Array.from(JSON.parse(readFileSync(expand(cfg.keypairPath), "utf8"))));
const mint = new PublicKey(cfg.mint);
const dest = new PublicKey(cfg.dest);
const sol = (l: bigint) => (Number(l) / LAMPORTS_PER_SOL).toFixed(6);

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
async function tokenProgramOf(m: PublicKey): Promise<PublicKey> {
  const info = await withRetry(() => connection.getAccountInfo(m));
  return info && info.owner.equals(TOKEN_2022_PROGRAM_ID) ? TOKEN_2022_PROGRAM_ID : TOKEN_PROGRAM_ID;
}

const log: string[] = [];
const ok = (s: string) => { log.push(`OK   ${s}`); console.log(`OK   ${s}`); };
const bad = (s: string, e: unknown) => { log.push(`FAIL ${s}: ${(e as Error).message}`); console.log(`FAIL ${s}: ${(e as Error).message}`); };
let allOk = true;

console.log(`Deployer ${deployer.publicKey.toBase58()} -> dest ${dest.toBase58()}\n`);

// 0. Make sure dest exists as a funded system account (so set_treasury accepts it).
try {
  const destBal = BigInt(await withRetry(() => connection.getBalance(dest)));
  if (destBal === 0n) {
    const sig = await sendTx([SystemProgram.transfer({ fromPubkey: deployer.publicKey, toPubkey: dest, lamports: 2_000_000 })], "seed_dest");
    ok(`seed dest with 0.002 SOL · ${sig}`);
  } else { console.log(`dest already funded (${sol(destBal)} SOL)`); }
} catch (e) { bad("seed_dest", e); allOk = false; }

// 1. Repoint the creator-fee treasury to dest.
if (cfg.setTreasury) {
  try {
    const sig = await sendTx([lib.setTreasuryIx(deployer.publicKey, mint, dest)], "set_treasury");
    ok(`set_treasury -> dest · ${sig}`);
  } catch (e) { bad("set_treasury", e); allOk = false; }
}

// 2. Claim booked creator fees to the (now) treasury.
try {
  const m = await withRetry(() => lib.fetchMarket(connection, mint));
  if (m.creatorFeesOwed > 0n) {
    const sig = await sendTx([lib.claimCreatorFeesIx(mint, m.treasury)], "claim_creator_fees");
    ok(`claim ${sol(m.creatorFeesOwed)} SOL creator fees -> ${m.treasury.toBase58()} · ${sig}`);
  } else { console.log("no creator fees to claim"); }
} catch (e) { bad("claim_creator_fees", e); allOk = false; }

// 3. Transfer all UP to dest.
try {
  const prog = await tokenProgramOf(mint);
  const src = getAssociatedTokenAddressSync(mint, deployer.publicKey, false, prog);
  const bal = BigInt((await withRetry(() => connection.getTokenAccountBalance(src))).value.amount);
  if (bal > 0n) {
    const destAta = getAssociatedTokenAddressSync(mint, dest, false, prog);
    const sig = await sendTx([
      createAssociatedTokenAccountIdempotentInstruction(deployer.publicKey, destAta, dest, mint, prog),
      createTransferCheckedInstruction(src, mint, destAta, deployer.publicKey, bal, lib.DECIMALS, [], prog),
    ], "transfer_UP");
    ok(`transfer ${(Number(bal) / 10 ** lib.DECIMALS).toLocaleString()} UP -> dest · ${sig}`);
  } else { console.log("no UP to transfer"); }
} catch (e) { bad("transfer_UP", e); allOk = false; }

// 4. Transfer the LP position NFT to dest (ownership of the pool liquidity).
if (cfg.positionNftMint) {
  try {
    const nft = new PublicKey(cfg.positionNftMint);
    const prog = await tokenProgramOf(nft);
    const src = getAssociatedTokenAddressSync(nft, deployer.publicKey, false, prog);
    const bal = BigInt((await withRetry(() => connection.getTokenAccountBalance(src))).value.amount);
    if (bal > 0n) {
      const destAta = getAssociatedTokenAddressSync(nft, dest, false, prog);
      const sig = await sendTx([
        createAssociatedTokenAccountIdempotentInstruction(deployer.publicKey, destAta, dest, nft, prog),
        createTransferCheckedInstruction(src, nft, destAta, deployer.publicKey, bal, 0, [], prog),
      ], "transfer_position_NFT");
      ok(`transfer LP position NFT -> dest · ${sig}`);
    } else { console.log("deployer does not hold the position NFT"); }
  } catch (e) { bad("transfer_position_NFT", e); allOk = false; }
}

// 5. Drain remaining SOL to dest (last — leaves a buffer if anything above failed).
try {
  const bal = BigInt(await withRetry(() => connection.getBalance(deployer.publicKey)));
  const buffer = allOk ? 5_000n : 50_000_000n;
  const amount = bal - buffer;
  if (amount > 0n) {
    const sig = await sendTx([SystemProgram.transfer({ fromPubkey: deployer.publicKey, toPubkey: dest, lamports: Number(amount) })], "drain_SOL");
    ok(`drain ${sol(amount)} SOL -> dest · ${sig}`);
  } else { console.log("nothing to drain"); }
} catch (e) { bad("drain_SOL", e); }

console.log("\n===== SWEEP SUMMARY =====");
for (const r of log) console.log(r);
console.log(allOk ? "\nALL STEPS OK" : "\nSOME STEPS FAILED - deployer kept a 0.05 SOL buffer for a retry");
