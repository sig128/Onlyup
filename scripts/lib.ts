// Shared client code for the ONLY UP program: addresses, instruction builders,
// account decoding and the same quote math the program uses, to the lamport.
import { createHash } from "node:crypto";
import {
  PublicKey,
  SystemProgram,
  TransactionInstruction,
  type Connection,
} from "@solana/web3.js";
import { TOKEN_PROGRAM_ID } from "@solana/spl-token";

export const PROGRAM_ID = new PublicKey("3ooGGJTLiadrijsQPW9AfTQLrgawhSkEYJPrPQpjnGxS");
export const DECIMALS = 6;
export const BPS = 10_000n;
const U64_MAX = (1n << 64n) - 1n;
const U128_MAX = (1n << 128n) - 1n;

// ---------- addresses ----------

export function marketPda(mint: PublicKey): PublicKey {
  return PublicKey.findProgramAddressSync([Buffer.from("market"), mint.toBuffer()], PROGRAM_ID)[0];
}
export function vaultPda(market: PublicKey): PublicKey {
  return PublicKey.findProgramAddressSync([Buffer.from("vault"), market.toBuffer()], PROGRAM_ID)[0];
}
export function lockedPda(market: PublicKey): PublicKey {
  return PublicKey.findProgramAddressSync([Buffer.from("locked"), market.toBuffer()], PROGRAM_ID)[0];
}

// ---------- encoding ----------

export function discriminator(name: string): Buffer {
  return createHash("sha256").update(`global:${name}`).digest().subarray(0, 8);
}

function u16(v: number): Buffer {
  const b = Buffer.alloc(2);
  b.writeUInt16LE(v);
  return b;
}
function u64(v: bigint): Buffer {
  const b = Buffer.alloc(8);
  b.writeBigUInt64LE(v);
  return b;
}

export interface InitParams {
  seedLamports: bigint;
  seedTokens: bigint;
  buyFeeBps: number;
  sellFeeBps: number;
  teamFeeBps: number;
}

export function encodeInitParams(p: InitParams): Buffer {
  return Buffer.concat([
    u64(p.seedLamports),
    u64(p.seedTokens),
    u16(p.buyFeeBps),
    u16(p.sellFeeBps),
    u16(p.teamFeeBps),
  ]);
}

const w = (pubkey: PublicKey, isSigner = false) => ({ pubkey, isSigner, isWritable: true });
const r = (pubkey: PublicKey, isSigner = false) => ({ pubkey, isSigner, isWritable: false });

// ---------- instructions (account order matches the program) ----------

/** Creates the market. `admin` must be the mint's current mint authority; the
 *  program hands the mint authority to the market in the same instruction. */
export function initializeIx(admin: PublicKey, mint: PublicKey, treasury: PublicKey, p: InitParams) {
  const market = marketPda(mint);
  return new TransactionInstruction({
    programId: PROGRAM_ID,
    keys: [
      w(admin, true),
      w(mint),
      w(market),
      w(vaultPda(market)),
      w(lockedPda(market)),
      r(treasury),
      r(TOKEN_PROGRAM_ID),
      r(SystemProgram.programId),
    ],
    data: Buffer.concat([discriminator("initialize"), encodeInitParams(p)]),
  });
}

export function buyIx(buyer: PublicKey, mint: PublicKey, buyerTokens: PublicKey, solIn: bigint, minTokensOut: bigint) {
  const market = marketPda(mint);
  return new TransactionInstruction({
    programId: PROGRAM_ID,
    keys: [
      w(buyer, true),
      w(mint),
      w(market),
      w(vaultPda(market)),
      w(buyerTokens),
      r(TOKEN_PROGRAM_ID),
      r(SystemProgram.programId),
    ],
    data: Buffer.concat([discriminator("buy"), u64(solIn), u64(minTokensOut)]),
  });
}

export function sellIx(seller: PublicKey, mint: PublicKey, sellerTokens: PublicKey, tokensIn: bigint, minSolOut: bigint) {
  const market = marketPda(mint);
  return new TransactionInstruction({
    programId: PROGRAM_ID,
    keys: [
      w(seller, true),
      w(mint),
      w(market),
      w(vaultPda(market)),
      w(sellerTokens),
      r(TOKEN_PROGRAM_ID),
      r(SystemProgram.programId),
    ],
    data: Buffer.concat([discriminator("sell"), u64(tokensIn), u64(minSolOut)]),
  });
}

export function donateIx(donor: PublicKey, mint: PublicKey, lamports: bigint) {
  const market = marketPda(mint);
  return new TransactionInstruction({
    programId: PROGRAM_ID,
    keys: [w(donor, true), w(market), w(vaultPda(market)), r(SystemProgram.programId)],
    data: Buffer.concat([discriminator("donate"), u64(lamports)]),
  });
}

/** Anyone can send this; the SOL can only go to the treasury stored in the market. */
export function claimTeamFeesIx(mint: PublicKey, treasury: PublicKey) {
  const market = marketPda(mint);
  return new TransactionInstruction({
    programId: PROGRAM_ID,
    keys: [w(market), w(vaultPda(market)), w(treasury), r(SystemProgram.programId)],
    data: discriminator("claim_team_fees"),
  });
}

export function setBuysPausedIx(admin: PublicKey, mint: PublicKey, paused: boolean) {
  return new TransactionInstruction({
    programId: PROGRAM_ID,
    keys: [r(admin, true), w(marketPda(mint))],
    data: Buffer.concat([discriminator("set_buys_paused"), Buffer.from([paused ? 1 : 0])]),
  });
}

/** The new treasury must be an ordinary wallet (owned by the System program). */
export function setTreasuryIx(admin: PublicKey, mint: PublicKey, newTreasury: PublicKey) {
  return new TransactionInstruction({
    programId: PROGRAM_ID,
    keys: [r(admin, true), w(marketPda(mint)), r(newTreasury)],
    data: discriminator("set_treasury"),
  });
}

export function renounceAdminIx(admin: PublicKey, mint: PublicKey) {
  return new TransactionInstruction({
    programId: PROGRAM_ID,
    keys: [r(admin, true), w(marketPda(mint))],
    data: discriminator("renounce_admin"),
  });
}

// ---------- account decoding ----------

export interface Market {
  version: number;
  bump: number;
  vaultBump: number;
  lockedBump: number;
  mint: PublicKey;
  admin: PublicKey;
  treasury: PublicKey;
  /** Lamports backing the price right now. */
  reserve: bigint;
  teamFeesOwed: bigint;
  buyFeeBps: number;
  sellFeeBps: number;
  teamFeeBps: number;
  buysPaused: boolean;
  createdAt: bigint;
  tradeCount: bigint;
  volumeLamports: bigint;
}

export const MARKET_SIZE = 8 + 147;

export function decodeMarket(data: Buffer): Market {
  if (data.length < MARKET_SIZE) throw new Error("not a market account");
  if (!data.subarray(0, 8).equals(createHash("sha256").update("account:Market").digest().subarray(0, 8))) {
    throw new Error("wrong account type");
  }
  let o = 8;
  const u8 = () => data.readUInt8(o++);
  const key = () => {
    const k = new PublicKey(data.subarray(o, o + 32));
    o += 32;
    return k;
  };
  const rd64 = () => {
    const v = data.readBigUInt64LE(o);
    o += 8;
    return v;
  };
  const ri64 = () => {
    const v = data.readBigInt64LE(o);
    o += 8;
    return v;
  };
  const rd16 = () => {
    const v = data.readUInt16LE(o);
    o += 2;
    return v;
  };
  return {
    version: u8(),
    bump: u8(),
    vaultBump: u8(),
    lockedBump: u8(),
    mint: key(),
    admin: key(),
    treasury: key(),
    reserve: rd64(),
    teamFeesOwed: rd64(),
    buyFeeBps: rd16(),
    sellFeeBps: rd16(),
    teamFeeBps: rd16(),
    buysPaused: u8() === 1,
    createdAt: ri64(),
    tradeCount: rd64(),
    volumeLamports: rd64(),
  };
}

export async function fetchMarket(connection: Connection, mint: PublicKey): Promise<Market> {
  const info = await connection.getAccountInfo(marketPda(mint));
  if (!info) throw new Error("market not found");
  if (!info.owner.equals(PROGRAM_ID)) throw new Error("market account is not owned by the ONLY UP program");
  return decodeMarket(info.data);
}

// ---------- quote math (mirrors programs/only-up/src/math.rs exactly) ----------

export interface BuyQuote {
  teamFee: bigint;
  toReserve: bigint;
  tokensOut: bigint;
}

export interface SellQuote {
  gross: bigint;
  teamFee: bigint;
  userOut: bigint;
  holderFee: bigint;
}

/** Same as the program's quote_buy. Returns null where the program would refuse. */
export function quoteBuy(
  solIn: bigint,
  reserve: bigint,
  supply: bigint,
  buyFee: number,
  teamFee: number,
): BuyQuote | null {
  if (reserve === 0n || supply === 0n) return null;
  const team = (solIn * BigInt(teamFee)) / BPS;
  const toReserve = solIn - team;
  const atPrice = (toReserve * supply) / reserve;
  if (atPrice * BPS > U128_MAX) return null;
  const tokensOut = (atPrice * BPS) / (BPS + BigInt(buyFee));
  if (tokensOut > U64_MAX) return null;
  return { teamFee: team, toReserve, tokensOut };
}

/** Same as the program's quote_sell. Returns null where the program would refuse. */
export function quoteSell(
  tokensIn: bigint,
  reserve: bigint,
  supply: bigint,
  sellFee: number,
  teamFee: number,
): SellQuote | null {
  if (supply === 0n || tokensIn >= supply) return null;
  const kept = BigInt(sellFee) + BigInt(teamFee);
  if (kept >= BPS) return null;
  const gross = (tokensIn * reserve) / supply;
  const team = (gross * BigInt(teamFee)) / BPS;
  const userOut = (gross * (BPS - kept)) / BPS;
  return { gross, teamFee: team, userOut, holderFee: gross - userOut - team };
}

export function quoteBuyMarket(m: Market, supply: bigint, solIn: bigint) {
  return quoteBuy(solIn, m.reserve, supply, m.buyFeeBps, m.teamFeeBps);
}

export function quoteSellMarket(m: Market, supply: bigint, tokensIn: bigint) {
  return quoteSell(tokensIn, m.reserve, supply, m.sellFeeBps, m.teamFeeBps);
}

/** SOL per whole token, as a float for display. */
export function priceSolPerToken(reserve: bigint, supply: bigint): number {
  return Number(reserve) / 1e9 / (Number(supply) / 10 ** DECIMALS);
}
