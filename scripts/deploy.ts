// One-shot launcher for ONLY UP v2 on an already-deployed program.
//
// In a single run it:
//   1. creates the token mint — the vanity mint that ends in "...up"
//      (keys/v2-mint-keypair.json), 6 decimals, no freeze authority
//   2. adds name / symbol / image through Metaplex token metadata
//      (immutable by default, so the identity can never be changed)
//   3. creates the market, which takes over the mint authority in the same
//      instruction — from then on tokens are only minted on a paid buy and
//      burned on a sell, and nobody can ever mint for free
//   4. optionally makes a first buy from the deployer wallet, same transaction
//
// Only the mint authority can create the market, so nobody can slip in between
// steps 1 and 3 with their own settings. The CA (contract address) is the mint,
// printed up front.
//
// v2 economics are fixed in the program, not here: 1% creator fee + 40% of each
// buy routed into the floor reserve. There are no fee dials in this config.
//
// Usage: npx tsx deploy.ts launch.config.json
import { readFileSync, writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import {
  ComputeBudgetProgram,
  Connection,
  Keypair,
  LAMPORTS_PER_SOL,
  PublicKey,
  SystemProgram,
  Transaction,
  sendAndConfirmTransaction,
} from "@solana/web3.js";
import {
  MINT_SIZE,
  TOKEN_PROGRAM_ID,
  createAssociatedTokenAccountIdempotentInstruction,
  createInitializeMint2Instruction,
  getAssociatedTokenAddressSync,
  getMinimumBalanceForRentExemptMint,
} from "@solana/spl-token";
import { createUmi } from "@metaplex-foundation/umi-bundle-defaults";
import { keypairIdentity, none, publicKey as umiPublicKey } from "@metaplex-foundation/umi";
import { createMetadataAccountV3, mplTokenMetadata } from "@metaplex-foundation/mpl-token-metadata";
import { fromWeb3JsKeypair } from "@metaplex-foundation/umi-web3js-adapters";
import * as lib from "./lib.js";

interface LaunchConfig {
  rpcUrl: string;
  /** Deployer: pays for everything and is the temporary mint authority. */
  keypairPath: string;
  /** The token mint keypair. Defaults to the vanity "...up" mint in keys/. */
  mintKeypairPath?: string;
  name: string;
  symbol: string;
  /** URL of the token's off-chain metadata JSON (name, symbol, image, socials). */
  metadataUri: string;
  /** false = name / symbol / image can never be changed (a trust signal). */
  metadataMutable: boolean;
  /** Wallet that may claim the creator fee share. Defaults to the deployer. */
  treasury?: string;
  /** Real SOL seeded into the curve reserve at launch (lamports stay locked). */
  seedSol: number;
  /** Virtual token reserve / asymptotic supply cap, in whole tokens.
   *  The starting price is seedSol / seedTokens. Must satisfy
   *  seedTokens <= seedSol * 1_000_000_000 (the program's StartPriceTooLow guard). */
  seedTokens: number;
  /** Optional first buy by the deployer, in SOL. These tokens are NOT locked. */
  launchBuySol: number;
}

// ---- config + keys ----
const configPath = process.argv[2] ?? "launch.config.json";
const cfg: LaunchConfig = JSON.parse(readFileSync(configPath, "utf8"));
const expand = (p: string) => p.replace(/^~/, process.env.HOME ?? "");
const loadKeypair = (p: string) => Keypair.fromSecretKey(Uint8Array.from(JSON.parse(readFileSync(expand(p), "utf8"))));

const deployer = loadKeypair(cfg.keypairPath);
// Default to the vanity mint shipped in the repo (resolved from this file, so it
// works no matter which directory the script is run from).
const defaultMintPath = fileURLToPath(new URL("../keys/v2-mint-keypair.json", import.meta.url));
const mintKp = loadKeypair(cfg.mintKeypairPath ?? defaultMintPath);
const mint = mintKp.publicKey;

const connection = new Connection(cfg.rpcUrl, "confirmed");

// ---- seed params (mirror the program's initialize guards) ----
const seedLamports = BigInt(Math.round(cfg.seedSol * LAMPORTS_PER_SOL));
const seedTokens = BigInt(Math.round(cfg.seedTokens)) * 10n ** BigInt(lib.DECIMALS);
const MIN_SEED_LAMPORTS = 20_000_000n; // 0.02 SOL
const MIN_SEED_TOKENS = 1_000_000n; // 1 whole token
const MAX_SEED_TOKENS = 1_000_000_000_000_000_000n; // 1e18 base units
const MAX_TOKENS_PER_LAMPORT = 1_000_000n;
if (seedLamports < MIN_SEED_LAMPORTS) throw new Error("seedSol must be at least 0.02");
if (seedTokens < MIN_SEED_TOKENS) throw new Error("seedTokens must be at least 1");
if (seedTokens > MAX_SEED_TOKENS) throw new Error("seedTokens is too large");
if (seedTokens / seedLamports > MAX_TOKENS_PER_LAMPORT)
  throw new Error(`seedTokens too high for this seedSol — must be <= seedSol * 1e9 (= ${cfg.seedSol * 1e9})`);

const treasury = new PublicKey(cfg.treasury && cfg.treasury.length > 0 ? cfg.treasury : deployer.publicKey.toBase58());
const market = lib.marketPda(mint);

async function main() {
  console.log(`\n🚀 ONLY UP launch`);
  console.log(`   Contract address (CA): ${mint.toBase58()}`);
  console.log(`   Market:                ${market.toBase58()}`);
  console.log(`   Deployer:              ${deployer.publicKey.toBase58()}\n`);

  const balance = await connection.getBalance(deployer.publicKey);
  console.log(`Deployer balance: ${balance / LAMPORTS_PER_SOL} SOL`);
  const programInfo = await connection.getAccountInfo(lib.PROGRAM_ID);
  if (!programInfo?.executable) throw new Error(`Program ${lib.PROGRAM_ID.toBase58()} is not deployed on this cluster`);
  const existing = await connection.getAccountInfo(mint);
  if (existing) throw new Error(`Mint ${mint.toBase58()} already exists on this cluster — already launched?`);

  // 1. Mint: deployer is the temporary mint authority, no freeze authority.
  const rent = await getMinimumBalanceForRentExemptMint(connection);
  await sendAndConfirmTransaction(
    connection,
    new Transaction().add(
      SystemProgram.createAccount({
        fromPubkey: deployer.publicKey,
        newAccountPubkey: mint,
        lamports: rent,
        space: MINT_SIZE,
        programId: TOKEN_PROGRAM_ID,
      }),
      createInitializeMint2Instruction(mint, lib.DECIMALS, deployer.publicKey, null),
    ),
    [deployer, mintKp],
  );
  console.log("1/3 mint created");

  // 2. Name, symbol, image — signed by the deployer while it still holds the
  //    mint authority. isMutable:false makes the identity permanent.
  const umi = createUmi(cfg.rpcUrl).use(mplTokenMetadata()).use(keypairIdentity(fromWeb3JsKeypair(deployer)));
  await createMetadataAccountV3(umi, {
    mint: umiPublicKey(mint.toBase58()),
    mintAuthority: umi.identity,
    data: {
      name: cfg.name,
      symbol: cfg.symbol,
      uri: cfg.metadataUri,
      sellerFeeBasisPoints: 0,
      creators: none(),
      collection: none(),
      uses: none(),
    },
    isMutable: cfg.metadataMutable,
    collectionDetails: none(),
  }).sendAndConfirm(umi);
  console.log(`2/3 metadata created (${cfg.metadataMutable ? "mutable" : "immutable"})`);

  // 3. Create the market (hands the mint authority to the market PDA for good),
  //    plus the optional first buy in the same transaction.
  const tx = new Transaction().add(
    ComputeBudgetProgram.setComputeUnitLimit({ units: 400_000 }),
    lib.initializeIx(deployer.publicKey, mint, treasury, { seedLamports, seedTokens }),
  );
  if (cfg.launchBuySol > 0) {
    const solIn = BigInt(Math.round(cfg.launchBuySol * LAMPORTS_PER_SOL));
    const ata = getAssociatedTokenAddressSync(mint, deployer.publicKey);
    // At launch: curve reserve = seedLamports, virtual tokens = seedTokens,
    // floor reserve = 0, circulating supply = 0.
    const q = lib.quoteBuy(solIn, seedLamports, seedTokens, 0n, 0n);
    if (!q || q.tokensOut === 0n) throw new Error("launch buy cannot be quoted — raise launchBuySol");
    console.log(`Launch buy: ~${Number(q.tokensOut) / 10 ** lib.DECIMALS} UP for ${cfg.launchBuySol} SOL`);
    tx.add(
      createAssociatedTokenAccountIdempotentInstruction(deployer.publicKey, ata, deployer.publicKey, mint),
      lib.buyIx(deployer.publicKey, mint, ata, solIn, (q.tokensOut * 99n) / 100n),
    );
  }
  const sig = await sendAndConfirmTransaction(connection, tx, [deployer]);
  console.log(`3/3 market created: ${sig}`);

  const m = await lib.fetchMarket(connection, mint);
  const supply = BigInt((await connection.getTokenSupply(mint)).value.amount);
  const tc = m.totalSupply - supply;
  const out = {
    cluster: cfg.rpcUrl,
    programId: lib.PROGRAM_ID.toBase58(),
    mint: mint.toBase58(),
    market: market.toBase58(),
    vault: lib.vaultPda(market).toBase58(),
    treasury: treasury.toBase58(),
    name: cfg.name,
    symbol: cfg.symbol,
    metadataUri: cfg.metadataUri,
    metadataMutable: cfg.metadataMutable,
    curveSol: Number(m.curveSol) / LAMPORTS_PER_SOL,
    floorReserveSol: Number(m.floorReserve) / LAMPORTS_PER_SOL,
    circulatingSupply: Number(supply) / 10 ** lib.DECIMALS,
    startCurvePriceSolPerToken: lib.curvePrice(m.curveSol, tc),
    floorPriceSolPerToken: lib.floorPrice(m.floorReserve, supply),
  };
  writeFileSync("launch-output.json", JSON.stringify(out, null, 2));
  console.log("\n✅ Launched. Details written to launch-output.json:\n");
  console.log(out);
}

main().catch((e) => {
  console.error(e);
  process.exit(1);
});
