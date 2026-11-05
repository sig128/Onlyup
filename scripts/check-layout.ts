// Checks that this client encodes instructions, decodes the market account and
// quotes trades exactly like the program. Fixtures come from
// programs/only-up/tests/layout.rs (run `cargo test` first).
import { readFileSync } from "node:fs";
import { PublicKey } from "@solana/web3.js";
import * as lib from "./lib.js";

const fx = JSON.parse(readFileSync(new URL("./fixtures/layout.json", import.meta.url), "utf8"));
let failures = 0;
let checks = 0;
function check(name: string, ok: boolean, detail = "") {
  checks++;
  if (!ok) {
    failures++;
    console.error(`FAIL ${name} ${detail}`);
  }
}
const key = (b: number) => new PublicKey(Buffer.alloc(32, b));
const hex = (b: Buffer) => b.toString("hex");

const params: lib.InitParams = {
  seedLamports: 123_456_789n,
  seedTokens: 987_654_321_000n,
  buyFeeBps: 150,
  sellFeeBps: 140,
  teamFeeBps: 50,
  maxHypeBps: 2_000,
  hypeFullBps: 1_900,
  hypeHalfLifeSecs: 3_600,
  dripSecs: 1_800,
};
const mint = key(1);
check("initialize", hex(lib.initializeIx(key(9), mint, key(3), params).data) === fx.initialize);
check("buy", hex(lib.buyIx(key(9), mint, key(8), 5n, 7n).data) === fx.buy);
check("sell", hex(lib.sellIx(key(9), mint, key(8), 9n, 11n).data) === fx.sell);
check("donate", hex(lib.donateIx(key(9), mint, 13n).data) === fx.donate);
check("claimTeamFees", hex(lib.claimTeamFeesIx(mint, key(3)).data) === fx.claimTeamFees);
check("setBuysPaused", hex(lib.setBuysPausedIx(key(2), mint, true).data) === fx.setBuysPaused);
check("setTreasury", hex(lib.setTreasuryIx(key(2), mint, key(4)).data) === fx.setTreasury);
check("renounceAdmin", hex(lib.renounceAdminIx(key(2), mint).data) === fx.renounceAdmin);

const marketBytes = Buffer.from(fx.market, "hex");
check("market size", marketBytes.length === lib.MARKET_SIZE, `${marketBytes.length} != ${lib.MARKET_SIZE}`);
const m = lib.decodeMarket(marketBytes);
const want: Record<string, unknown> = {
  version: 1, bump: 254, vaultBump: 253, lockedBump: 252,
  reserve: 11n, pending: 12n, teamFeesOwed: 13n, heat: 14n, heatUpdatedAt: -15n, lastDripAt: 16n,
  buyFeeBps: 17, sellFeeBps: 18, teamFeeBps: 19, maxHypeBps: 20, hypeFullBps: 21,
  hypeHalfLifeSecs: 22, dripSecs: 23, buysPaused: true, createdAt: 24n, tradeCount: 25n, volumeLamports: 26n,
};
const got = m as unknown as Record<string, unknown>;
for (const [k, v] of Object.entries(want)) {
  check(`market.${k}`, got[k] === v, `${got[k]} != ${v}`);
}
check("market.mint", m.mint.equals(key(1)));
check("market.admin", m.admin.equals(key(2)));
check("market.treasury", m.treasury.equals(key(3)));

const buyTuple = (q: lib.BuyQuote | null | undefined) =>
  q ? JSON.stringify([q.teamFee, q.toReserve, q.fair, q.toPending, q.tokensOut].map(String)) : "null";
const sellTuple = (q: lib.SellQuote | null) =>
  q ? JSON.stringify([q.gross, q.teamFee, q.userOut, q.toPending].map(String)) : "null";

for (const [i, q] of (fx.quotes as Record<string, unknown>[]).entries()) {
  const reserve = BigInt(q.reserve as string);
  const supply = BigInt(q.supply as string);
  const amount = BigInt(q.amount as string);
  const heat = BigInt(q.heat as string);
  const elapsed = BigInt(q.elapsed as string);
  const buyFee = q.buyFee as number, sellFee = q.sellFee as number, team = q.team as number;
  const maxHype = q.maxHype as number, hypeFull = q.hypeFull as number;

  const b = lib.quoteBuy(amount, reserve, supply, buyFee, team, q.hype as number);
  check(`quote ${i} buy`, buyTuple(b) === JSON.stringify(q.buy), `${buyTuple(b)} != ${JSON.stringify(q.buy)}`);

  const full = lib.quoteBuyFull(amount, reserve, supply, heat, buyFee, team, maxHype, hypeFull);
  check(`quote ${i} buyFull`, buyTuple(full?.quote) === JSON.stringify(q.buyFull), `${buyTuple(full?.quote)} != ${JSON.stringify(q.buyFull)}`);
  check(`quote ${i} fullHype`, (full ? full.hype : -1) === q.fullHype, `${full?.hype} != ${q.fullHype}`);

  const s = lib.quoteSell((amount % (supply - 1n)) + 1n, reserve, supply, sellFee, team, 0);
  check(`quote ${i} sell`, sellTuple(s) === JSON.stringify(q.sell), `${sellTuple(s)} != ${JSON.stringify(q.sell)}`);

  const d = lib.decayHeat(heat, elapsed, q.halfLife as number);
  check(`quote ${i} decay`, d === BigInt(q.decayed as string), `${d} != ${q.decayed}`);
  const h = lib.hypeBps(heat, reserve, maxHype, hypeFull);
  check(`quote ${i} hype`, h === q.hypeFromHeat, `${h} != ${q.hypeFromHeat}`);
  const toReserve = amount - (amount * BigInt(team)) / lib.BPS;
  const bh = lib.buyHypeBps(heat, reserve, toReserve, maxHype, hypeFull);
  check(`quote ${i} buyHype`, bh === q.buyHype, `${bh} != ${q.buyHype}`);
  const v = lib.vest(BigInt(q.pending as string), elapsed, q.drip as number);
  check(`quote ${i} vest`, v === BigInt(q.vested as string), `${v} != ${q.vested}`);
}

if (failures) {
  console.error(`${failures} of ${checks} layout checks failed`);
  process.exit(1);
}
console.log(`Client matches the program: 8 instructions, market layout, ${fx.quotes.length} quote cases (${checks} checks).`);
