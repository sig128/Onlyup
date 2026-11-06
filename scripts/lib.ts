// Shared client code for the ONLY UP v2 program: addresses, instruction
// builders, account decoding and the same quote math the program uses, to the
// lamport. Checked against Rust-generated fixtures by check-layout.ts.
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
export const CREATOR_FEE_BPS = 50n; // 0.5%, fixed in the program
export const FLOOR_FEE_BPS = 250n; // 2.5% into the floor, fixed in the program
export const FLOOR_BPS = 2_000n; // 20%, fixed in the program
const U64_MAX = (1n << 64n) - 1n;

// ---------- addresses ----------

export function marketPda(mint: PublicKey): PublicKey {
  return PublicKey.findProgramAddressSync([Buffer.from("market"), mint.toBuffer()], PROGRAM_ID)[0];
}
export function vaultPda(market: PublicKey): PublicKey {
  return PublicKey.findProgramAddressSync([Buffer.from("vault"), market.toBuffer()], PROGRAM_ID)[0];
}

// ---------- encoding ----------

export function discriminator(name: string): Buffer {
  return createHash("sha256").update(`global:${name}`).digest().subarray(0, 8);
}

function u64(v: bigint): Buffer {
  const b = Buffer.alloc(8);
  b.writeBigUInt64LE(v);
  return b;
}

export interface InitParams {
  seedLamports: bigint;
  seedTokens: bigint;
}

export function encodeInitParams(p: InitParams): Buffer {
  return Buffer.concat([u64(p.seedLamports), u64(p.seedTokens)]);
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

/** Adds SOL straight into the floor reserve, raising the floor for everyone. */
export function donateIx(donor: PublicKey, mint: PublicKey, lamports: bigint) {
  const market = marketPda(mint);
  return new TransactionInstruction({
    programId: PROGRAM_ID,
    keys: [w(donor, true), w(market), w(vaultPda(market)), r(SystemProgram.programId)],
    data: Buffer.concat([discriminator("donate"), u64(lamports)]),
  });
}

/** Anyone can send this; the SOL can only go to the treasury stored in the market. */
export function claimCreatorFeesIx(mint: PublicKey, treasury: PublicKey) {
  const market = marketPda(mint);
  return new TransactionInstruction({
    programId: PROGRAM_ID,
    keys: [w(market), w(vaultPda(market)), w(treasury), r(SystemProgram.programId)],
    data: discriminator("claim_creator_fees"),
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
  mint: PublicKey;
  admin: PublicKey;
  treasury: PublicKey;
  /** Real SOL in the constant-product curve reserve (lamports). */
  curveSol: bigint;
  /** Real SOL in the floor reserve (lamports). Redeemable at floorReserve/supply. */
  floorReserve: bigint;
  /** Virtual token reserve at launch and the asymptotic supply cap (base units). */
  totalSupply: bigint;
  creatorFeesOwed: bigint;
  buysPaused: boolean;
  createdAt: bigint;
  tradeCount: bigint;
  volumeLamports: bigint;
}

export const MARKET_SIZE = 8 + 156;

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
  return {
    version: u8(),
    bump: u8(),
    vaultBump: u8(),
    mint: key(),
    admin: key(),
    treasury: key(),
    curveSol: rd64(),
    floorReserve: rd64(),
    totalSupply: rd64(),
    creatorFeesOwed: rd64(),
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

function ceilDiv(a: bigint, b: bigint): bigint {
  return (a + b - 1n) / b;
}

export interface BuyQuote {
  creatorFee: bigint;
  toFloor: bigint;
  toCurve: bigint;
  tokensOut: bigint;
  newCurveSol: bigint;
}

export interface SellQuote {
  fromFloor: boolean;
  gross: bigint;
  creatorFee: bigint;
  floorFee: bigint;
  userOut: bigint;
  newCurveSol: bigint;
  newFloorReserve: bigint;
}

/** Same as the program's quote_buy. `tc` is the virtual token reserve
 *  (totalSupply - supply). Returns null exactly where the program would refuse. */
export function quoteBuy(sol: bigint, rc: bigint, tc: bigint, rf: bigint, s: bigint): BuyQuote | null {
  if (rc === 0n || tc === 0n || sol === 0n) return null;
  const creatorFee = (sol * CREATOR_FEE_BPS) / BPS;
  if (creatorFee > U64_MAX) return null;
  const floorFee = (sol * FLOOR_FEE_BPS) / BPS;
  if (floorFee > U64_MAX) return null;
  const avail = sol - creatorFee - floorFee;
  if (avail <= 0n) return null;
  const baseFloor = (avail * FLOOR_BPS) / BPS;
  if (baseFloor > U64_MAX) return null;
  const k = rc * tc;

  const toCurve0 = avail - baseFloor;
  if (toCurve0 === 0n) return null;
  const rcA = rc + toCurve0;
  const tcA = ceilDiv(k, rcA);
  if (tcA > tc) return null;
  const out0 = tc - tcA;

  // Anti-dilution: the 2.5% floor fee counts toward the no-dilution requirement,
  // so only the remainder has to come out of avail.
  let requiredFromAvail = 0n;
  if (s > 0n) {
    const needed = ceilDiv(rf * out0, s);
    requiredFromAvail = needed > floorFee ? needed - floorFee : 0n;
    if (requiredFromAvail > U64_MAX) return null;
  }
  let splitFloor = baseFloor > requiredFromAvail ? baseFloor : requiredFromAvail;
  if (splitFloor > avail) splitFloor = avail;
  const toCurve = avail - splitFloor;

  if (toCurve === 0n) {
    if (s === 0n || rf === 0n) return null;
    const out = (avail * s) / rf;
    if (out > U64_MAX) return null;
    if (out === 0n || out >= tc) return null;
    return { creatorFee, toFloor: floorFee + avail, toCurve: 0n, tokensOut: out, newCurveSol: rc };
  }

  const rc2 = rc + toCurve;
  const tc2 = ceilDiv(k, rc2);
  if (tc2 >= tc) return null;
  const out = tc - tc2;
  if (out === 0n) return null;
  return { creatorFee, toFloor: floorFee + splitFloor, toCurve, tokensOut: out, newCurveSol: rc2 };
}

/** Same as the program's quote_sell. Returns null exactly where the program would refuse. */
export function quoteSell(tok: bigint, rc: bigint, tc: bigint, rf: bigint, s: bigint): SellQuote | null {
  if (s === 0n || tok === 0n || tok >= s) return null;
  const floorOut = (rf * tok) / s;
  if (floorOut > U64_MAX) return null;
  const k = rc * tc;
  const tcNew = tc + tok;
  const rc2Curve = ceilDiv(k, tcNew);
  const curveOut = rc > rc2Curve ? rc - rc2Curve : 0n;

  let fromFloor: boolean, gross: bigint, newCurveSol: bigint, floorAfterRedeem: bigint;
  if (curveOut >= floorOut) {
    fromFloor = false;
    gross = curveOut;
    newCurveSol = rc2Curve;
    floorAfterRedeem = rf;
  } else {
    fromFloor = true;
    gross = floorOut;
    newCurveSol = rc;
    floorAfterRedeem = rf - floorOut;
  }
  const creatorFee = (gross * CREATOR_FEE_BPS) / BPS;
  const floorFee = (gross * FLOOR_FEE_BPS) / BPS;
  const userOut = gross - creatorFee - floorFee;
  const newFloorReserve = floorAfterRedeem + floorFee;
  return { fromFloor, gross, creatorFee, floorFee, userOut, newCurveSol, newFloorReserve };
}

/** True if new_rf/new_s >= old_rf/old_s — the floor did not fall. */
export function floorNotLower(oldRf: bigint, oldS: bigint, newRf: bigint, newS: bigint): boolean {
  if (newS === 0n) return true;
  return newRf * oldS >= oldRf * newS;
}

export function quoteBuyMarket(m: Market, supply: bigint, solIn: bigint) {
  return quoteBuy(solIn, m.curveSol, m.totalSupply - supply, m.floorReserve, supply);
}
export function quoteSellMarket(m: Market, supply: bigint, tokensIn: bigint) {
  return quoteSell(tokensIn, m.curveSol, m.totalSupply - supply, m.floorReserve, supply);
}

/** Curve price, SOL per whole token, as a float for display. */
export function curvePrice(curveSol: bigint, tc: bigint): number {
  return Number(curveSol) / 1e9 / (Number(tc) / 10 ** DECIMALS);
}
/** Floor price (always redeemable), SOL per whole token. */
export function floorPrice(floorReserve: bigint, supply: bigint): number {
  if (supply === 0n) return 0;
  return Number(floorReserve) / 1e9 / (Number(supply) / 10 ** DECIMALS);
}
