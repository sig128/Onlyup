// Shared client code for the ONLY UP program: addresses, instruction builders,
// account decoding and the same quote math the program uses, to the lamport.
import { createHash } from "node:crypto";
import {
  PublicKey,
  SYSVAR_CLOCK_PUBKEY,
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
function u32(v: number): Buffer {
  const b = Buffer.alloc(4);
  b.writeUInt32LE(v);
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
  maxHypeBps: number;
  hypeFullBps: number;
  hypeHalfLifeSecs: number;
  dripSecs: number;
}

export function encodeInitParams(p: InitParams): Buffer {
  return Buffer.concat([
    u64(p.seedLamports),
    u64(p.seedTokens),
    u16(p.buyFeeBps),
    u16(p.sellFeeBps),
    u16(p.teamFeeBps),
    u16(p.maxHypeBps),
    u16(p.hypeFullBps),
    u32(p.hypeHalfLifeSecs),
    u32(p.dripSecs),
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
  /** Lamports backing the price right after the last instruction. */
  reserve: bigint;
  /** Fees paid but not yet dripped into the reserve. */
  pending: bigint;
  teamFeesOwed: bigint;
  heat: bigint;
  heatUpdatedAt: bigint;
  lastDripAt: bigint;
  buyFeeBps: number;
  sellFeeBps: number;
  teamFeeBps: number;
  maxHypeBps: number;
  hypeFullBps: number;
  hypeHalfLifeSecs: number;
  dripSecs: number;
  buysPaused: boolean;
  createdAt: bigint;
  tradeCount: bigint;
  volumeLamports: bigint;
}

export const MARKET_SIZE = 8 + 191;

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
  const rd32 = () => {
    const v = data.readUInt32LE(o);
    o += 4;
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
    pending: rd64(),
    teamFeesOwed: rd64(),
    heat: rd64(),
    heatUpdatedAt: ri64(),
    lastDripAt: ri64(),
    buyFeeBps: rd16(),
    sellFeeBps: rd16(),
    teamFeeBps: rd16(),
    maxHypeBps: rd16(),
    hypeFullBps: rd16(),
    hypeHalfLifeSecs: rd32(),
    dripSecs: rd32(),
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

/** The chain's own clock (unix seconds), which is what the program uses. */
export async function fetchChainTime(connection: Connection): Promise<bigint> {
  const info = await connection.getAccountInfo(SYSVAR_CLOCK_PUBKEY);
  if (!info) throw new Error("clock sysvar not found");
  return info.data.readBigInt64LE(32);
}

// ---------- quote math (mirrors programs/only-up/src/math.rs exactly) ----------

/** Lamports of `pending` that have dripped into the reserve after `elapsed` seconds. */
export function vest(pending: bigint, elapsed: bigint, dripSecs: number): bigint {
  if (pending === 0n || elapsed <= 0n || dripSecs === 0) return 0n;
  const drip = BigInt(dripSecs);
  if (elapsed >= drip) return pending;
  return (pending * elapsed + drip - 1n) / drip;
}

export function priceNotLower(oldReserve: bigint, oldSupply: bigint, newReserve: bigint, newSupply: bigint): boolean {
  return newReserve * oldSupply >= oldReserve * newSupply;
}

const LN2_E12 = 693_147_180_560n;
const E12 = 1_000_000_000_000n;

function exp2NegFracE12(rem: bigint, halfLife: bigint): bigint {
  const u = (rem * LN2_E12) / halfLife;
  const u2 = (u * u) / E12;
  const u3 = (u2 * u) / E12;
  const u4 = (u3 * u) / E12;
  const u5 = (u4 * u) / E12;
  const u6 = (u5 * u) / E12;
  return E12 + u2 / 2n + u4 / 24n + u6 / 720n - (u + u3 / 6n + u5 / 120n);
}

/** Heat after `elapsed` seconds: halves every half-life, smoothly. */
export function decayHeat(heat: bigint, elapsed: bigint, halfLife: number): bigint {
  if (heat === 0n || elapsed <= 0n || halfLife === 0) return heat;
  const hl = BigInt(halfLife);
  const halvings = elapsed / hl;
  if (halvings >= 64n) return 0n;
  const h = heat >> halvings;
  return (h * exp2NegFracE12(elapsed % hl, hl)) / E12;
}

/** Hype level in basis points for a given heat. */
export function hypeBps(heat: bigint, reserve: bigint, maxHype: number, hypeFull: number): number {
  if (maxHype === 0 || heat === 0n) return 0;
  const full = reserve * BigInt(hypeFull);
  const scaled = heat * BPS;
  if (full === 0n || scaled >= full) return maxHype;
  return Number((scaled * BigInt(maxHype)) / full);
}

export const HYPE_STEPS = 8n;

/** Hype fee a buy adding `toReserve` lamports pays: the average over its own heat. */
export function buyHypeBps(heat: bigint, reserve: bigint, toReserve: bigint, maxHype: number, hypeFull: number): number {
  if (maxHype === 0) return 0;
  const sat = (a: bigint, b: bigint) => (a + b > U64_MAX ? U64_MAX : a + b);
  let sum = 0n;
  for (let i = 1n; i <= HYPE_STEPS; i++) {
    const added = (toReserve * i) / HYPE_STEPS;
    sum += BigInt(hypeBps(sat(heat, added), sat(reserve, added), maxHype, hypeFull));
  }
  return Number((sum + HYPE_STEPS - 1n) / HYPE_STEPS);
}

export interface BuyQuote {
  teamFee: bigint;
  toReserve: bigint;
  /** Joins the reserve at once. */
  fair: bigint;
  /** Drips into the reserve over the drip window. */
  toPending: bigint;
  tokensOut: bigint;
}

export interface SellQuote {
  gross: bigint;
  teamFee: bigint;
  userOut: bigint;
  toPending: bigint;
}

/** Same as the program's quote_buy. Returns null where the program would refuse. */
export function quoteBuy(
  solIn: bigint,
  reserve: bigint,
  supply: bigint,
  buyFee: number,
  teamFee: number,
  hype: number,
): BuyQuote | null {
  if (reserve === 0n || supply === 0n) return null;
  const team = (solIn * BigInt(teamFee)) / BPS;
  const toReserve = solIn - team;
  const atPrice = (toReserve * supply) / reserve;
  if (atPrice * BPS > U128_MAX) return null;
  const tokensOut = (atPrice * BPS) / (BPS + BigInt(buyFee) + BigInt(hype));
  if (tokensOut > U64_MAX) return null;
  const fair = (tokensOut * reserve + supply - 1n) / supply;
  if (fair > U64_MAX || fair > toReserve) return null;
  return { teamFee: team, toReserve, fair, toPending: toReserve - fair, tokensOut };
}

/** Same as the program's quote_buy_full: works out this buy's hype fee, then quotes it. */
export function quoteBuyFull(
  solIn: bigint,
  reserve: bigint,
  supply: bigint,
  heat: bigint,
  buyFee: number,
  teamFee: number,
  maxHype: number,
  hypeFull: number,
): { quote: BuyQuote; hype: number } | null {
  const team = (solIn * BigInt(teamFee)) / BPS;
  const hype = buyHypeBps(heat, reserve, solIn - team, maxHype, hypeFull);
  const quote = quoteBuy(solIn, reserve, supply, buyFee, teamFee, hype);
  return quote ? { quote, hype } : null;
}

/** Same as the program's quote_sell. Sells always pass a hype of 0. */
export function quoteSell(
  tokensIn: bigint,
  reserve: bigint,
  supply: bigint,
  sellFee: number,
  teamFee: number,
  hype = 0,
): SellQuote | null {
  if (supply === 0n || tokensIn >= supply) return null;
  const kept = BigInt(sellFee) + BigInt(hype) + BigInt(teamFee);
  if (kept >= BPS) return null;
  const gross = (tokensIn * reserve) / supply;
  const team = (gross * BigInt(teamFee)) / BPS;
  const userOut = (gross * (BPS - kept)) / BPS;
  return { gross, teamFee: team, userOut, toPending: gross - userOut - team };
}

// ---------- the market right now ----------

export interface MarketNow {
  /** Reserve including everything that has dripped in by `now`. */
  reserve: bigint;
  /** Fees still waiting to drip in. */
  pending: bigint;
  /** Heat decayed to `now`. */
  heat: bigint;
  supply: bigint;
  now: bigint;
}

/** What the program will see if an instruction runs at `now`. */
export function marketNow(m: Market, supply: bigint, now: bigint): MarketNow {
  const vested = vest(m.pending, now - m.lastDripAt, m.dripSecs);
  return {
    reserve: m.reserve + vested,
    pending: m.pending - vested,
    heat: decayHeat(m.heat, now - m.heatUpdatedAt, m.hypeHalfLifeSecs),
    supply,
    now,
  };
}

export function quoteBuyNow(m: Market, s: MarketNow, solIn: bigint) {
  return quoteBuyFull(solIn, s.reserve, s.supply, s.heat, m.buyFeeBps, m.teamFeeBps, m.maxHypeBps, m.hypeFullBps);
}

export function quoteSellNow(m: Market, s: MarketNow, tokensIn: bigint) {
  return quoteSell(tokensIn, s.reserve, s.supply, m.sellFeeBps, m.teamFeeBps, 0);
}

/** Hype fee a tiny buy would pay right now, in basis points. */
export function currentHype(m: Market, s: MarketNow): number {
  return hypeBps(s.heat, s.reserve, m.maxHypeBps, m.hypeFullBps);
}

/** SOL per whole token, as a float for display. */
export function priceSolPerToken(reserve: bigint, supply: bigint): number {
  return Number(reserve) / 1e9 / (Number(supply) / 10 ** DECIMALS);
}
