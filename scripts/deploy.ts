// Launches ONLY UP on an already deployed program:
//   1. creates the token mint (6 decimals, no freeze authority)
//   2. adds name, symbol and image through Metaplex token metadata
//   3. creates the market, which takes over the mint authority in the same
//      instruction and locks the seed SOL and seed tokens forever
//   4. optionally makes a first buy from the deployer wallet, in that same transaction
//
// Only the mint authority can create the market, so nobody can jump in
// between steps 1 and 3 with their own settings.
//
// Usage: npx tsx deploy.ts launch.config.json
import { readFileSync, writeFileSync } from "node:fs";
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
  keypairPath: string;
  name: string;
  symbol: string;
  /** URL of the token's metadata JSON (name, symbol, image). */
  metadataUri: string;
  /** false = name, symbol and image can never be changed (a trust signal). */
  metadataMutable: boolean;
  /** Wallet for the creator's fee share. Defaults to the deployer. */
  treasury?: string;
  /** SOL locked in the safe forever at launch. */
  seedSol: number;
  /** Starting price in SOL per token. Must be at least 0.000001. */
  startPriceSol: number;
  buyFeeBps: number;
  sellFeeBps: number;
  teamFeeBps: number;
  /** Optional first buy by the deployer, in SOL. These tokens are not locked. */
  launchBuySol: number;
}

const configPath = process.argv[2] ?? "launch.config.json";
const cfg: LaunchConfig = JSON.parse(readFileSync(configPath, "utf8"));
const deployer = Keypair.fromSecretKey(
  Uint8Array.from(JSON.parse(readFileSync(cfg.keypairPath.replace(/^~/, process.env.HOME ?? ""), "utf8"))),
);
const connection = new Connection(cfg.rpcUrl, "confirmed");

const seedLamports = BigInt(Math.round(cfg.seedSol * LAMPORTS_PER_SOL));
const seedTokens = BigInt(Math.round((cfg.seedSol / cfg.startPriceSol) * 10 ** lib.DECIMALS));
if (seedTokens / seedLamports > 1_000n) throw new Error("startPriceSol must be at least 0.000001");
const treasury = new PublicKey(cfg.treasury ?? deployer.publicKey.toBase58());

async function main() {
  const balance = await connection.getBalance(deployer.publicKey);
  console.log(`Deployer ${deployer.publicKey.toBase58()} has ${balance / LAMPORTS_PER_SOL} SOL`);
  const programInfo = await connection.getAccountInfo(lib.PROGRAM_ID);
  if (!programInfo?.executable) throw new Error(`Program ${lib.PROGRAM_ID.toBase58()} is not deployed on this cluster`);

  // 1. Mint, with the deployer as temporary mint authority and no freeze authority.
  const mintKp = Keypair.generate();
  const mint = mintKp.publicKey;
  const market = lib.marketPda(mint);
  console.log(`Mint   ${mint.toBase58()}\nMarket ${market.toBase58()}`);
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

  // 2. Name, symbol and image.
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
  console.log("2/3 metadata created");

  // 3. Create the market (it takes over the mint authority), plus the optional first buy.
  const params: lib.InitParams = {
    seedLamports,
    seedTokens,
    buyFeeBps: cfg.buyFeeBps,
    sellFeeBps: cfg.sellFeeBps,
    teamFeeBps: cfg.teamFeeBps,
  };
  const tx = new Transaction().add(
    ComputeBudgetProgram.setComputeUnitLimit({ units: 400_000 }),
    lib.initializeIx(deployer.publicKey, mint, treasury, params),
  );
  if (cfg.launchBuySol > 0) {
    const solIn = BigInt(Math.round(cfg.launchBuySol * LAMPORTS_PER_SOL));
    const ata = getAssociatedTokenAddressSync(mint, deployer.publicKey);
    const q = lib.quoteBuy(solIn, seedLamports, seedTokens, cfg.buyFeeBps, cfg.teamFeeBps);
    if (!q || q.tokensOut === 0n) throw new Error("launch buy cannot be quoted");
    console.log(`Launch buy: ${Number(q.tokensOut) / 10 ** lib.DECIMALS} tokens`);
    tx.add(
      createAssociatedTokenAccountIdempotentInstruction(deployer.publicKey, ata, deployer.publicKey, mint),
      lib.buyIx(deployer.publicKey, mint, ata, solIn, (q.tokensOut * 99n) / 100n),
    );
  }
  const sig = await sendAndConfirmTransaction(connection, tx, [deployer]);
  console.log(`3/3 market created: ${sig}`);

  const m = await lib.fetchMarket(connection, mint);
  const supply = BigInt((await connection.getTokenSupply(mint)).value.amount);
  const out = {
    cluster: cfg.rpcUrl,
    programId: lib.PROGRAM_ID.toBase58(),
    mint: mint.toBase58(),
    market: market.toBase58(),
    vault: lib.vaultPda(market).toBase58(),
    lockedTokens: lib.lockedPda(market).toBase58(),
    treasury: treasury.toBase58(),
    safeSol: Number(m.reserve) / LAMPORTS_PER_SOL,
    priceSolPerToken: lib.priceSolPerToken(m.reserve, supply),
  };
  writeFileSync("launch-output.json", JSON.stringify(out, null, 2));
  console.log(out);
}

main().catch((e) => {
  console.error(e);
  process.exit(1);
});
