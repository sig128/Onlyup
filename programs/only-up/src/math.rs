//! Pure pricing math, with no Solana types, so it can be tested directly.
//!
//! Price = reserve / supply, where reserve is in lamports and supply in token
//! base units. Fees never land in the reserve at once: they go to `pending`
//! and drip into the reserve over `drip_secs`. So the price climbs smoothly
//! between trades, and nobody can buy just before a big trade and sell just
//! after it to grab its fees. All rounding favours the vault, which is what
//! keeps the price from ever going down.

pub const BPS: u128 = 10_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BuyQuote {
    /// Team share of `sol_in`, kept in the vault until claimed.
    pub team_fee: u64,
    /// Everything else (`sol_in - team_fee`); all of it stays for holders.
    pub to_reserve: u64,
    /// The part that backs the new tokens at today's price; joins the reserve now.
    pub fair: u64,
    /// The fee and hype part; drips into the reserve over the drip window.
    pub to_pending: u64,
    /// Tokens minted to the buyer.
    pub tokens_out: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SellQuote {
    /// Value of the tokens at today's price; leaves the reserve.
    pub gross: u64,
    /// Team share, kept in the vault until claimed.
    pub team_fee: u64,
    /// Lamports paid to the seller.
    pub user_out: u64,
    /// The sell fee; drips back into the reserve over the drip window.
    pub to_pending: u64,
}

/// Quote a buy of `sol_in` lamports at a given hype fee.
///
/// The buyer gets tokens at `price * (1 + fee + hype)`. The value of those
/// tokens at today's price joins the reserve now (so the price does not move),
/// and the rest drips in later (so the price rises).
pub fn quote_buy(
    sol_in: u64,
    reserve: u64,
    supply: u64,
    buy_fee_bps: u16,
    team_fee_bps: u16,
    hype_bps: u16,
) -> Option<BuyQuote> {
    if reserve == 0 || supply == 0 {
        return None;
    }
    let team_fee = u64::try_from(sol_in as u128 * team_fee_bps as u128 / BPS).ok()?;
    let to_reserve = sol_in.checked_sub(team_fee)?;
    // Tokens worth `to_reserve` at today's price, rounded down...
    let at_price = (to_reserve as u128) * (supply as u128) / (reserve as u128);
    // ...then the fee and hype charged on top, rounded down again.
    let markup = BPS + buy_fee_bps as u128 + hype_bps as u128;
    let tokens_out = u64::try_from(at_price.checked_mul(BPS)? / markup).ok()?;
    // Their value at today's price, rounded up so the reserve never falls short.
    let fair_num = tokens_out as u128 * reserve as u128;
    let fair = u64::try_from(fair_num.div_ceil(supply as u128)).ok()?;
    let to_pending = to_reserve.checked_sub(fair)?;
    Some(BuyQuote {
        team_fee,
        to_reserve,
        fair,
        to_pending,
        tokens_out,
    })
}

/// Quote a sale of `tokens_in` base units.
///
/// The seller receives `price * (1 - fee - hype - team)`. The fee and hype
/// part drips back into the reserve. The program always passes a hype of zero
/// for sells; the parameter only keeps the math general.
pub fn quote_sell(
    tokens_in: u64,
    reserve: u64,
    supply: u64,
    sell_fee_bps: u16,
    team_fee_bps: u16,
    hype_bps: u16,
) -> Option<SellQuote> {
    if supply == 0 || tokens_in >= supply {
        return None;
    }
    let kept = sell_fee_bps as u128 + hype_bps as u128 + team_fee_bps as u128;
    if kept >= BPS {
        return None;
    }
    let gross = u64::try_from(tokens_in as u128 * reserve as u128 / supply as u128).ok()?;
    let team_fee = u64::try_from(gross as u128 * team_fee_bps as u128 / BPS).ok()?;
    let user_out = u64::try_from(gross as u128 * (BPS - kept) / BPS).ok()?;
    let to_pending = gross - user_out - team_fee;
    Some(SellQuote {
        gross,
        team_fee,
        user_out,
        to_pending,
    })
}

/// Lamports of `pending` that have dripped into the reserve after `elapsed`
/// seconds. Each update releases `elapsed / drip_secs` of what is pending,
/// rounded up, so even tiny amounts finish dripping.
pub fn vest(pending: u64, elapsed: i64, drip_secs: u32) -> u64 {
    if pending == 0 || elapsed <= 0 || drip_secs == 0 {
        return 0;
    }
    let elapsed = elapsed as u64;
    if elapsed >= drip_secs as u64 {
        return pending;
    }
    let num = pending as u128 * elapsed as u128;
    num.div_ceil(drip_secs as u128) as u64
}

/// True if `new_reserve / new_supply >= old_reserve / old_supply`.
/// Uses cross-multiplication, so there is no rounding at all.
pub fn price_not_lower(old_reserve: u64, old_supply: u64, new_reserve: u64, new_supply: u64) -> bool {
    (new_reserve as u128) * (old_supply as u128) >= (old_reserve as u128) * (new_supply as u128)
}

/// ln(2) and 1.0, both scaled by 1e12.
const LN2_E12: u128 = 693_147_180_560;
const E12: u128 = 1_000_000_000_000;

/// 2^(-rem / half_life) for 0 <= rem < half_life, scaled by 1e12.
/// Taylor series of e^-u with u = (rem / half_life) * ln 2, up to u^6.
/// Ending on a positive term keeps the result at or just above the true value
/// (error below 0.002%), so heat never ticks up as it crosses a half-life.
fn exp2_neg_frac_e12(rem: u64, half_life: u64) -> u128 {
    let u = rem as u128 * LN2_E12 / half_life as u128;
    let u2 = u * u / E12;
    let u3 = u2 * u / E12;
    let u4 = u3 * u / E12;
    let u5 = u4 * u / E12;
    let u6 = u5 * u / E12;
    let plus = E12 + u2 / 2 + u4 / 24 + u6 / 720;
    let minus = u + u3 / 6 + u5 / 120;
    plus - minus
}

/// Heat after `elapsed` seconds with half-life `half_life`: a true
/// exponential decay, so decaying in many small steps gives the same result
/// as one big step.
pub fn decay_heat(heat: u64, elapsed: i64, half_life: u32) -> u64 {
    if heat == 0 || elapsed <= 0 || half_life == 0 {
        return heat;
    }
    let half_life = half_life as u64;
    let elapsed = elapsed as u64;
    let halvings = elapsed / half_life;
    if halvings >= 64 {
        return 0;
    }
    let h = heat >> halvings;
    let factor = exp2_neg_frac_e12(elapsed % half_life, half_life);
    (h as u128 * factor / E12) as u64
}

/// Hype level in basis points for a given heat: zero with no recent buying,
/// rising in a straight line to `max_hype_bps` once recent buying reaches
/// `hype_full_bps` of the reserve.
pub fn hype_bps(heat: u64, reserve: u64, max_hype_bps: u16, hype_full_bps: u16) -> u16 {
    if max_hype_bps == 0 || heat == 0 {
        return 0;
    }
    let full = reserve as u128 * hype_full_bps as u128;
    let scaled_heat = heat as u128 * BPS;
    if full == 0 || scaled_heat >= full {
        return max_hype_bps;
    }
    (scaled_heat * max_hype_bps as u128 / full) as u16
}

/// Number of points sampled along a buy to price its hype fee.
pub const HYPE_STEPS: u64 = 8;

/// Hype fee for a buy that adds `to_reserve` lamports, in basis points.
///
/// The hype rises while the buy itself adds heat, so the fee is the average
/// hype across the buy, sampled at the end of each of 8 equal slices and
/// rounded up. Sampling slice ends charges at least the true average, so a
/// buyer always pays for the heat they create and cannot push the next buyer
/// into a high hype fee for free.
pub fn buy_hype_bps(heat: u64, reserve: u64, to_reserve: u64, max_hype_bps: u16, hype_full_bps: u16) -> u16 {
    if max_hype_bps == 0 {
        return 0;
    }
    let mut sum: u64 = 0;
    for i in 1..=HYPE_STEPS {
        let added = (to_reserve as u128 * i as u128 / HYPE_STEPS as u128) as u64;
        sum += hype_bps(
            heat.saturating_add(added),
            reserve.saturating_add(added),
            max_hype_bps,
            hype_full_bps,
        ) as u64;
    }
    sum.div_ceil(HYPE_STEPS) as u16
}

/// Full buy quote as the program computes it: team share, the hype fee for
/// this buy, then the tokens. Returns the quote and the hype fee charged.
#[allow(clippy::too_many_arguments)]
pub fn quote_buy_full(
    sol_in: u64,
    reserve: u64,
    supply: u64,
    heat: u64,
    buy_fee_bps: u16,
    team_fee_bps: u16,
    max_hype_bps: u16,
    hype_full_bps: u16,
) -> Option<(BuyQuote, u16)> {
    let team_fee = u64::try_from(sol_in as u128 * team_fee_bps as u128 / BPS).ok()?;
    let to_reserve = sol_in.checked_sub(team_fee)?;
    let hype = buy_hype_bps(heat, reserve, to_reserve, max_hype_bps, hype_full_bps);
    let quote = quote_buy(sol_in, reserve, supply, buy_fee_bps, team_fee_bps, hype)?;
    Some((quote, hype))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SOL: u64 = 1_000_000_000;
    const TOKEN: u64 = 1_000_000; // 6 decimals

    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            let mut x = self.0;
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            self.0 = x;
            x
        }
        fn range(&mut self, lo: u64, hi: u64) -> u64 {
            lo + self.next() % (hi - lo + 1)
        }
    }

    fn price(reserve: u64, supply: u64) -> f64 {
        reserve as f64 / supply as f64
    }

    /// A tiny market model: reserve, pending fees, supply.
    #[derive(Clone, Copy)]
    struct M {
        r: u64,
        p: u64,
        s: u64,
    }
    impl M {
        fn drip_all(&mut self) {
            self.r += self.p;
            self.p = 0;
        }
        fn buy(&mut self, q: BuyQuote) {
            self.r += q.fair;
            self.p += q.to_pending;
            self.s += q.tokens_out;
        }
        fn sell(&mut self, tokens: u64, q: SellQuote) {
            self.r -= q.gross;
            self.p += q.to_pending;
            self.s -= tokens;
        }
    }

    #[test]
    fn buy_with_no_fees_keeps_price_flat() {
        let q = quote_buy(10 * SOL, 100 * SOL, 100_000 * TOKEN, 0, 0, 0).unwrap();
        assert_eq!(q.tokens_out, 10_000 * TOKEN);
        assert_eq!(q.fair, 10 * SOL);
        assert_eq!(q.to_pending, 0);
    }

    #[test]
    fn plain_words_example_matches() {
        // Safe 100 SOL, 100,000 tokens. Alice buys for 10 SOL at a 1% fee.
        let mut m = M { r: 100 * SOL, p: 0, s: 100_000 * TOKEN };
        let p0 = price(m.r, m.s);
        let a = quote_buy(10 * SOL, m.r, m.s, 100, 0, 0).unwrap();
        let tokens = a.tokens_out as f64 / TOKEN as f64;
        assert!((tokens - 9_900.99).abs() < 0.01, "alice got {tokens}");
        m.buy(a);
        // Right after the buy the price has not moved; the fee is still dripping in.
        assert!((price(m.r, m.s) / p0 - 1.0).abs() < 1e-9);
        m.drip_all();
        let p1 = price(m.r, m.s);
        assert!((p1 * TOKEN as f64 / SOL as f64 - 0.0010009).abs() < 0.0000001);
        // Bob sells 5,000 tokens at a 1% fee.
        let b = quote_sell(5_000 * TOKEN, m.r, m.s, 100, 0, 0).unwrap();
        let sol = b.user_out as f64 / SOL as f64;
        assert!((sol - 4.9545).abs() < 0.0001, "bob got {sol}");
        m.sell(5_000 * TOKEN, b);
        m.drip_all();
        let p2 = price(m.r, m.s);
        assert!((p2 * TOKEN as f64 / SOL as f64 - 0.0010014).abs() < 0.0000001);
        assert!(p2 > p1 && p1 > p0);
    }

    #[test]
    fn fees_raise_price_on_buy_and_sell_once_dripped() {
        let mut m = M { r: 50 * SOL, p: 0, s: 1_000_000 * TOKEN };
        let p0 = price(m.r, m.s);
        let b = quote_buy(3 * SOL, m.r, m.s, 150, 50, 0).unwrap();
        m.buy(b);
        assert!(price(m.r, m.s) >= p0);
        m.drip_all();
        let p1 = price(m.r, m.s);
        assert!(p1 > p0);
        let q = quote_sell(b.tokens_out, m.r, m.s, 150, 50, 0).unwrap();
        m.sell(b.tokens_out, q);
        assert!(price(m.r, m.s) >= p1);
        m.drip_all();
        assert!(price(m.r, m.s) > p1);
        // A round trip costs about the fees: 2% in, 2% out.
        let lost = 1.0 - q.user_out as f64 / (3 * SOL) as f64;
        assert!(lost > 0.035 && lost < 0.045, "round trip lost {lost}");
    }

    #[test]
    fn cannot_sell_whole_supply() {
        assert!(quote_sell(100, 1_000, 100, 10, 0, 0).is_none());
        assert!(quote_sell(101, 1_000, 100, 10, 0, 0).is_none());
        assert!(quote_sell(99, 1_000, 100, 10, 0, 0).is_some());
    }

    #[test]
    fn rejects_fee_settings_that_eat_everything() {
        assert!(quote_sell(10, 1_000, 100, 5_000, 0, 5_000).is_none());
    }

    #[test]
    fn vesting_releases_everything_within_the_window() {
        assert_eq!(vest(0, 100, 3_600), 0);
        assert_eq!(vest(1_000, 0, 3_600), 0);
        assert_eq!(vest(1_000, -5, 3_600), 0);
        assert_eq!(vest(1_000, 3_600, 3_600), 1_000);
        assert_eq!(vest(1_000, 99_999, 3_600), 1_000);
        assert_eq!(vest(3_600, 1, 3_600), 1);
        assert_eq!(vest(1, 1, 3_600), 1, "rounds up so dust drains");
        // Dripping in small steps always ends with nothing left.
        let mut pending = 1_000_000_000u64;
        for _ in 0..100_000 {
            pending -= vest(pending, 5, 3_600);
            if pending == 0 {
                break;
            }
        }
        assert_eq!(pending, 0);
    }

    #[test]
    fn decay_halves_each_half_life() {
        assert_eq!(decay_heat(1_000, 0, 3_600), 1_000);
        assert_eq!(decay_heat(1_000, 3_600, 3_600), 500);
        assert_eq!(decay_heat(1_000, 7_200, 3_600), 250);
        assert_eq!(decay_heat(1_000, 1_800, 3_600), 707); // 1000 / sqrt(2)
        assert_eq!(decay_heat(u64::MAX, 3_600 * 64, 3_600), 0);
        assert_eq!(decay_heat(1_000, -5, 3_600), 1_000);
        let mut last = u64::MAX;
        for t in (0..20_000).step_by(37) {
            let h = decay_heat(1_000_000, t, 3_600);
            assert!(h <= last, "decay went up at t={t}");
            last = h;
        }
    }

    #[test]
    fn decay_is_the_same_in_small_steps_or_one_big_step() {
        let start = 1_000_000_000_000u64;
        let mut stepped = start;
        for _ in 0..360 {
            stepped = decay_heat(stepped, 10, 3_600);
        }
        let once = decay_heat(start, 3_600, 3_600);
        assert_eq!(once, start / 2);
        let diff = (stepped as f64 - once as f64).abs() / once as f64;
        assert!(diff < 0.0001, "stepped {stepped} vs once {once}");
        // Tiny heat still fades away instead of getting stuck.
        let mut tiny = 50u64;
        for _ in 0..1_000 {
            tiny = decay_heat(tiny, 60, 3_600);
        }
        assert_eq!(tiny, 0);
    }

    #[test]
    fn hype_scales_with_heat() {
        let r = 100 * SOL;
        assert_eq!(hype_bps(0, r, 2_000, 2_000), 0);
        assert_eq!(hype_bps(10 * SOL, r, 2_000, 2_000), 1_000); // heat 10% of reserve: half the max
        assert_eq!(hype_bps(20 * SOL, r, 2_000, 2_000), 2_000); // heat 20%: the max
        assert_eq!(hype_bps(500 * SOL, r, 2_000, 2_000), 2_000); // capped
        assert_eq!(hype_bps(10 * SOL, r, 0, 2_000), 0);
    }

    #[test]
    fn a_buy_pays_for_the_heat_it_creates() {
        let r = 100 * SOL;
        let small = buy_hype_bps(0, r, SOL, 2_000, 2_000);
        assert!(small > 0 && small < 100, "small {small}");
        let big = buy_hype_bps(0, r, 100 * SOL, 2_000, 2_000);
        assert!(big > 1_700 && big <= 2_000, "big {big}");
        for amount in [1, SOL / 10, SOL, 10 * SOL, 1_000 * SOL] {
            let h = buy_hype_bps(5 * SOL, r, amount, 2_000, 2_000);
            assert!(h >= hype_bps(5 * SOL, r, 2_000, 2_000) && h <= 2_000);
        }
        let mut last = 0;
        for k in 1..200u64 {
            let h = buy_hype_bps(0, r, k * SOL / 2, 2_000, 2_000);
            assert!(h >= last, "bigger buys never pay a lower rate");
            last = h;
        }
    }

    /// The attack the review found: buy big right before someone else, push
    /// them into a high hype fee, sell straight after. Because fees drip in
    /// over time and each buy pays for its own heat, this always loses money.
    #[test]
    fn front_running_a_buyer_and_selling_straight_after_always_loses() {
        let mut rng = Rng(0xF00D_BEEF_1234_5678);
        for _ in 0..20_000 {
            let buy_fee = rng.range(10, 1_000) as u16;
            let sell_fee = rng.range(10, 500) as u16;
            let team = rng.range(0, 200) as u16;
            let max_hype = rng.range(0, 2_000) as u16;
            let hype_full = rng.range(100, 10_000) as u16;
            let r0 = rng.range(SOL / 100, 10_000 * SOL);
            let s0 = rng.range(r0 / 1_000 + 1, r0.saturating_mul(1_000));
            let heat0 = rng.range(0, r0);
            let attacker_sol = rng.range(1_000, 5 * r0 + 1_000);
            let victim_sol = rng.range(1_000, 5 * r0 + 1_000);
            let Some((a, _)) = quote_buy_full(attacker_sol, r0, s0, heat0, buy_fee, team, max_hype, hype_full) else {
                continue;
            };
            if a.tokens_out == 0 {
                continue;
            }
            let mut m = M { r: r0, p: 0, s: s0 };
            m.buy(a);
            let heat1 = heat0 + a.to_reserve;
            let Some((v, _)) = quote_buy_full(victim_sol, m.r, m.s, heat1, buy_fee, team, max_hype, hype_full) else {
                continue;
            };
            m.buy(v);
            // Same moment: nothing has dripped yet.
            let out = quote_sell(a.tokens_out, m.r, m.s, sell_fee, team, 0).unwrap().user_out;
            assert!(out < attacker_sol, "front-runner made money: in {attacker_sol}, out {out}");
        }
    }

    /// With the shortest drip (one minute) and the launch fees, a bot that
    /// buys right before someone and sells a few seconds after still loses:
    /// only a sliver of the fees has dripped in by then. Includes buys up to
    /// 50 times the whole reserve.
    #[test]
    fn with_a_one_minute_drip_selling_seconds_later_still_loses() {
        let mut rng = Rng(0xD21F_0060_ABCD_0001);
        let (buy_fee, sell_fee, team, max_hype, hype_full) = (150, 150, 50, 2_000, 2_000);
        let mut checked = 0;
        for _ in 0..20_000 {
            let r0 = rng.range(SOL / 100, 10_000 * SOL);
            let s0 = rng.range(r0 / 1_000 + 1, r0.saturating_mul(1_000));
            let heat0 = rng.range(0, r0);
            let attacker_sol = rng.range(1_000, 5 * r0 + 1_000);
            let victim_sol = rng.range(1_000, 50 * r0 + 1_000);
            let wait = rng.range(1, 5) as i64;
            let Some((a, _)) = quote_buy_full(attacker_sol, r0, s0, heat0, buy_fee, team, max_hype, hype_full) else {
                continue;
            };
            if a.tokens_out == 0 {
                continue;
            }
            let mut m = M { r: r0, p: 0, s: s0 };
            m.buy(a);
            let heat1 = heat0 + a.to_reserve;
            let Some((v, _)) = quote_buy_full(victim_sol, m.r, m.s, heat1, buy_fee, team, max_hype, hype_full) else {
                continue;
            };
            m.buy(v);
            let vested = vest(m.p, wait, 60);
            m.r += vested;
            m.p -= vested;
            let out = quote_sell(a.tokens_out, m.r, m.s, sell_fee, team, 0).unwrap().user_out;
            assert!(out < attacker_sol, "made money: in {attacker_sol}, out {out}, waited {wait}s");
            checked += 1;
        }
        assert!(checked > 15_000);
    }

    #[test]
    fn growth_formula_matches_table() {
        // With every buy paying premium k, the price grows as
        // (market cap growth) ^ (k / (1 + k)). Check the 10% row: 2x -> about +6.5%.
        let mut m = M { r: 100 * SOL, p: 0, s: 100_000 * TOKEN };
        let p0 = price(m.r, m.s);
        while m.r < 200 * SOL {
            let q = quote_buy(SOL / 10, m.r, m.s, 1_000, 0, 0).unwrap();
            m.buy(q);
            m.drip_all();
        }
        let growth = price(m.r, m.s) / p0;
        assert!((growth - 1.065).abs() < 0.005, "growth {growth}");
    }

    #[test]
    fn random_trading_never_lowers_price_and_vault_can_always_pay() {
        for seed in 1..=300u64 {
            let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
            let buy_fee = rng.range(10, 1_000) as u16;
            let sell_fee = rng.range(10, 500) as u16;
            let team = rng.range(0, 200) as u16;
            let max_hype = rng.range(0, 2_000) as u16;
            let hype_full = rng.range(100, 10_000) as u16;
            let half_life = rng.range(60, 86_400) as u32;
            let drip = rng.range(60, 86_400) as u32;
            let reserve0 = rng.range(10_000_000, 50 * SOL);
            // Same limit as the program: at most 1,000 base units per seed lamport.
            let locked = rng.range(1_000_000, reserve0.saturating_mul(1_000));
            let mut m = M { r: reserve0, p: 0, s: locked };
            let (mut heat, mut heat_at, mut drip_at, mut now) = (0u64, 0i64, 0i64, 0i64);
            let start_price = price(m.r, m.s);
            let mut holders: Vec<u64> = Vec::new();
            let mut team_owed = 0u64;
            let mut last = (m.r, m.s);
            for _ in 0..400 {
                now += rng.range(0, 3_000) as i64;
                // Every instruction first lets pending fees drip in.
                let v = vest(m.p, now - drip_at, drip);
                m.r += v;
                m.p -= v;
                drip_at = now;
                assert!(price_not_lower(last.0, last.1, m.r, m.s), "dripping lowered the price, seed {seed}");
                last = (m.r, m.s);
                if holders.is_empty() || rng.next() % 2 == 0 {
                    let sol_in = rng.range(1, 200 * SOL);
                    let h = decay_heat(heat, now - heat_at, half_life);
                    let Some((q, _)) = quote_buy_full(sol_in, m.r, m.s, h, buy_fee, team, max_hype, hype_full) else {
                        continue;
                    };
                    if q.tokens_out == 0 || m.s.checked_add(q.tokens_out).is_none() {
                        continue; // the program refuses these
                    }
                    let before = m;
                    m.buy(q);
                    assert!(price_not_lower(before.r, before.s, m.r, m.s), "buy lowered price, seed {seed}");
                    assert_eq!(q.team_fee + q.fair + q.to_pending, sol_in, "every lamport accounted for");
                    heat = h + q.to_reserve;
                    heat_at = now;
                    team_owed += q.team_fee;
                    holders.push(q.tokens_out);
                } else {
                    let i = (rng.next() % holders.len() as u64) as usize;
                    let amount = rng.range(1, holders[i]);
                    let q = quote_sell(amount, m.r, m.s, sell_fee, team, 0).unwrap();
                    assert_eq!(q.user_out + q.team_fee + q.to_pending, q.gross);
                    let before = m;
                    m.sell(amount, q);
                    assert!(price_not_lower(before.r, before.s, m.r, m.s), "sell lowered price, seed {seed}");
                    team_owed += q.team_fee;
                    holders[i] -= amount;
                    if holders[i] == 0 {
                        holders.swap_remove(i);
                    }
                }
                last = (m.r, m.s);
            }
            // Everyone sells everything, one by one: the reserve always covers it.
            while let Some(amount) = holders.pop() {
                let q = quote_sell(amount, m.r, m.s, sell_fee, team, 0).unwrap();
                assert!(q.gross <= m.r, "reserve short, seed {seed}");
                let before = m;
                m.sell(amount, q);
                assert!(price_not_lower(before.r, before.s, m.r, m.s));
            }
            assert_eq!(m.s, locked);
            m.drip_all();
            assert!(price(m.r, m.s) >= start_price, "ended below start, seed {seed}");
            let _ = team_owed;
        }
    }
}
