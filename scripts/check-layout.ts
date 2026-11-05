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
  buyFeeBps: 250,
  sellFeeBps: 260,
  teamFeeBps: 50,
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
  reserve: 11n, teamFeesOwed: 12n, buyFeeBps: 13, sellFeeBps: 14, teamFeeBps: 15,
  buysPaused: true, createdAt: 16n, tradeCount: 17n, volumeLamports: 18n,
};
const got = m as unknown as Record<string, unknown>;
for (const [k, v] of Object.entries(want)) {
  check(`market.${k}`, got[k] === v, `${got[k]} != ${v}`);
}
check("market.mint", m.mint.equals(key(1)));
check("market.admin", m.admin.equals(key(2)));
check("market.treasury", m.treasury.equals(key(3)));

const buyTuple = (q: lib.BuyQuote | null) =>
  q ? JSON.stringify([q.teamFee, q.toReserve, q.tokensOut].map(String)) : "null";
const sellTuple = (q: lib.SellQuote | null) =>
  q ? JSON.stringify([q.gross, q.teamFee, q.userOut, q.holderFee].map(String)) : "null";

for (const [i, q] of (fx.quotes as Record<string, unknown>[]).entries()) {
  const reserve = BigInt(q.reserve as string);
  const supply = BigInt(q.supply as string);
  const amount = BigInt(q.amount as string);
  const buyFee = q.buyFee as number, sellFee = q.sellFee as number, team = q.team as number;

  const b = lib.quoteBuy(amount, reserve, supply, buyFee, team);
  check(`quote ${i} buy`, buyTuple(b) === JSON.stringify(q.buy), `${buyTuple(b)} != ${JSON.stringify(q.buy)}`);
  const s = lib.quoteSell((amount % (supply - 1n)) + 1n, reserve, supply, sellFee, team);
  check(`quote ${i} sell`, sellTuple(s) === JSON.stringify(q.sell), `${sellTuple(s)} != ${JSON.stringify(q.sell)}`);
}

if (failures) {
  console.error(`${failures} of ${checks} layout checks failed`);
  process.exit(1);
}
console.log(`Client matches the program: 8 instructions, market layout, ${fx.quotes.length} quote cases (${checks} checks).`);
