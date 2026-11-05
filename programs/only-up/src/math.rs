//! Pure pricing math, with no Solana types, so it can be tested directly.
//!
//! Price = reserve / supply, where reserve is in lamports and supply in token
//! base units. The holder fee on every trade goes straight into the reserve,
//! so the price rises the instant a trade lands. All rounding favours the
//! vault, which is what keeps the price from ever going down.

pub const BPS: u128 = 10_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BuyQuote {
    /// Creator share of `sol_in`, kept in the vault until claimed.
    pub team_fee: u64,
    /// Everything else (`sol_in - team_fee`); all of it joins the reserve now.
    pub to_reserve: u64,
    /// Tokens minted to the buyer.
    pub tokens_out: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SellQuote {
    /// Value of the tokens at today's price; the starting point for the split.
    pub gross: u64,
    /// Creator share, kept in the vault until claimed.
    pub team_fee: u64,
    /// Lamports paid to the seller.
    pub user_out: u64,
    /// The holder fee; stays in the reserve, so the price rises.
    pub holder_fee: u64,
}

/// Quote a buy of `sol_in` lamports.
///
/// The buyer gets tokens at `price * (1 + buy_fee)`. The whole non-creator part
/// of their SOL joins the reserve now, while they receive fewer tokens than that
/// SOL is worth, so the price rises immediately.
pub fn quote_buy(sol_in: u64, reserve: u64, supply: u64, buy_fee_bps: u16, team_fee_bps: u16) -> Option<BuyQuote> {
    if reserve == 0 || supply == 0 {
        return None;
    }
    let team_fee = u64::try_from(sol_in as u128 * team_fee_bps as u128 / BPS).ok()?;
    let to_reserve = sol_in.checked_sub(team_fee)?;
    // Tokens worth `to_reserve` at today's price, rounded down...
    let at_price = (to_reserve as u128) * (supply as u128) / (reserve as u128);
    // ...then the fee charged on top, rounded down again. Both favour the vault.
    let tokens_out = u64::try_from(at_price.checked_mul(BPS)? / (BPS + buy_fee_bps as u128)).ok()?;
    Some(BuyQuote { team_fee, to_reserve, tokens_out })
}

/// Quote a sale of `tokens_in` base units.
///
/// The seller receives `price * (1 - sell_fee - team)`. The holder-fee part
/// stays in the reserve, so the price rises.
pub fn quote_sell(tokens_in: u64, reserve: u64, supply: u64, sell_fee_bps: u16, team_fee_bps: u16) -> Option<SellQuote> {
    if supply == 0 || tokens_in >= supply {
        return None;
    }
    let kept = sell_fee_bps as u128 + team_fee_bps as u128;
    if kept >= BPS {
        return None;
    }
    let gross = u64::try_from(tokens_in as u128 * reserve as u128 / supply as u128).ok()?;
    let team_fee = u64::try_from(gross as u128 * team_fee_bps as u128 / BPS).ok()?;
    let user_out = u64::try_from(gross as u128 * (BPS - kept) / BPS).ok()?;
    let holder_fee = gross - user_out - team_fee;
    Some(SellQuote { gross, team_fee, user_out, holder_fee })
}

/// True if `new_reserve / new_supply >= old_reserve / old_supply`.
/// Uses cross-multiplication, so there is no rounding at all.
pub fn price_not_lower(old_reserve: u64, old_supply: u64, new_reserve: u64, new_supply: u64) -> bool {
    (new_reserve as u128) * (old_supply as u128) >= (old_reserve as u128) * (new_supply as u128)
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

    /// A tiny market model: reserve and supply. Fees land instantly.
    #[derive(Clone, Copy)]
    struct M {
        r: u64,
        s: u64,
    }
    impl M {
        fn buy(&mut self, q: BuyQuote) {
            self.r += q.to_reserve;
            self.s += q.tokens_out;
        }
        fn sell(&mut self, tokens: u64, q: SellQuote) {
            self.r -= q.user_out + q.team_fee;
            self.s -= tokens;
        }
    }

    #[test]
    fn buy_with_no_fees_keeps_price_flat() {
        let q = quote_buy(10 * SOL, 100 * SOL, 100_000 * TOKEN, 0, 0).unwrap();
        assert_eq!(q.tokens_out, 10_000 * TOKEN);
        assert_eq!(q.to_reserve, 10 * SOL);
    }

    #[test]
    fn plain_words_example_matches() {
        // Safe 100 SOL, 100,000 tokens. Alice buys for 10 SOL at a 1% fee.
        let mut m = M { r: 100 * SOL, s: 100_000 * TOKEN };
        let p0 = price(m.r, m.s);
        let a = quote_buy(10 * SOL, m.r, m.s, 100, 0).unwrap();
        let tokens = a.tokens_out as f64 / TOKEN as f64;
        assert!((tokens - 9_900.99).abs() < 0.01, "alice got {tokens}");
        m.buy(a);
        // The price has already moved; there is no drip.
        let p1 = price(m.r, m.s);
        assert!((p1 * TOKEN as f64 / SOL as f64 - 0.0010009).abs() < 0.0000001, "p1 {p1}");
        assert!(p1 > p0);
        // Bob sells 5,000 tokens at a 1% fee.
        let b = quote_sell(5_000 * TOKEN, m.r, m.s, 100, 0).unwrap();
        let sol = b.user_out as f64 / SOL as f64;
        assert!((sol - 4.9545).abs() < 0.0001, "bob got {sol}");
        m.sell(5_000 * TOKEN, b);
        let p2 = price(m.r, m.s);
        assert!((p2 * TOKEN as f64 / SOL as f64 - 0.0010014).abs() < 0.0000001, "p2 {p2}");
        assert!(p2 > p1 && p1 > p0);
    }

    #[test]
    fn fees_raise_price_on_every_buy_and_sell() {
        let mut m = M { r: 50 * SOL, s: 1_000_000 * TOKEN };
        let mut last = price(m.r, m.s);
        let b = quote_buy(3 * SOL, m.r, m.s, 250, 50).unwrap();
        m.buy(b);
        assert!(price(m.r, m.s) > last);
        last = price(m.r, m.s);
        let q = quote_sell(b.tokens_out, m.r, m.s, 250, 50).unwrap();
        m.sell(b.tokens_out, q);
        assert!(price(m.r, m.s) > last);
        // A round trip costs about the fees: 3% in, 3% out.
        let lost = 1.0 - q.user_out as f64 / (3 * SOL) as f64;
        assert!(lost > 0.055 && lost < 0.065, "round trip lost {lost}");
    }

    #[test]
    fn cannot_sell_whole_supply() {
        assert!(quote_sell(100, 1_000, 100, 10, 0).is_none());
        assert!(quote_sell(101, 1_000, 100, 10, 0).is_none());
        assert!(quote_sell(99, 1_000, 100, 10, 0).is_some());
    }

    #[test]
    fn rejects_fee_settings_that_eat_everything() {
        assert!(quote_sell(10, 1_000, 100, 9_000, 1_000).is_none());
    }

    /// Buy right before someone else's buy, then sell straight after. The
    /// program requires the sell fee to be at least the buy fee, and a buy can
    /// lift the price by at most the buy fee, so the exit always costs at least
    /// the most the front-runner's tokens could have gained. They lose every
    /// time, at any size and any allowed settings. (This is why fees can land
    /// instantly without a drip.) The 50,000-case range below includes buys of
    /// up to fifty times the whole reserve.
    #[test]
    fn front_running_a_buyer_and_selling_straight_after_always_loses() {
        let mut rng = Rng(0xF00D_BEEF_1234_5678);
        for _ in 0..50_000 {
            let buy_fee = rng.range(10, 500) as u16;
            let sell_fee = rng.range(buy_fee as u64, 500) as u16; // program enforces sell >= buy
            let team = rng.range(0, 200) as u16;
            let r0 = rng.range(SOL / 100, 10_000 * SOL);
            let s0 = rng.range(r0 / 1_000 + 1, r0.saturating_mul(1_000));
            let attacker_sol = rng.range(1_000, 5 * r0 + 1_000);
            let victim_sol = rng.range(1_000, 50 * r0 + 1_000);
            let Some(a) = quote_buy(attacker_sol, r0, s0, buy_fee, team) else { continue };
            if a.tokens_out == 0 {
                continue;
            }
            let mut m = M { r: r0, s: s0 };
            m.buy(a);
            let Some(v) = quote_buy(victim_sol, m.r, m.s, buy_fee, team) else { continue };
            m.buy(v);
            let out = quote_sell(a.tokens_out, m.r, m.s, sell_fee, team).unwrap().user_out;
            assert!(out < attacker_sol, "front-runner made money: in {attacker_sol}, out {out}");
        }
    }

    #[test]
    fn growth_formula_matches_table() {
        // With every buy paying premium k, the price grows as
        // (market cap growth) ^ (k / (1 + k)). Check the 10% row: 2x -> about +6.5%.
        let mut m = M { r: 100 * SOL, s: 100_000 * TOKEN };
        let p0 = price(m.r, m.s);
        while m.r < 200 * SOL {
            let q = quote_buy(SOL / 10, m.r, m.s, 1_000, 0).unwrap();
            m.buy(q);
        }
        let growth = price(m.r, m.s) / p0;
        assert!((growth - 1.065).abs() < 0.005, "growth {growth}");
    }

    #[test]
    fn random_trading_never_lowers_price_and_vault_can_always_pay() {
        for seed in 1..=500u64 {
            let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
            let buy_fee = rng.range(10, 500) as u16;
            let sell_fee = rng.range(10, 500) as u16;
            let team = rng.range(0, 200) as u16;
            let reserve0 = rng.range(10_000_000, 50 * SOL);
            let locked = rng.range(1_000_000, reserve0.saturating_mul(1_000));
            let mut m = M { r: reserve0, s: locked };
            let start_price = price(m.r, m.s);
            let mut holders: Vec<u64> = Vec::new();
            let mut team_owed = 0u64;
            let mut last = (m.r, m.s);
            for _ in 0..400 {
                if holders.is_empty() || rng.next() % 2 == 0 {
                    let sol_in = rng.range(1, 200 * SOL);
                    let Some(q) = quote_buy(sol_in, m.r, m.s, buy_fee, team) else { continue };
                    if q.tokens_out == 0 || m.s.checked_add(q.tokens_out).is_none() {
                        continue; // the program refuses these
                    }
                    let before = m;
                    m.buy(q);
                    assert!(price_not_lower(before.r, before.s, m.r, m.s), "buy lowered price, seed {seed}");
                    assert_eq!(q.team_fee + q.to_reserve, sol_in, "every lamport accounted for");
                    team_owed += q.team_fee;
                    holders.push(q.tokens_out);
                } else {
                    let i = (rng.next() % holders.len() as u64) as usize;
                    let amount = rng.range(1, holders[i]);
                    let q = quote_sell(amount, m.r, m.s, sell_fee, team).unwrap();
                    assert_eq!(q.user_out + q.team_fee + q.holder_fee, q.gross);
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
                let q = quote_sell(amount, m.r, m.s, sell_fee, team).unwrap();
                assert!(q.user_out + q.team_fee <= m.r, "reserve short, seed {seed}");
                let before = m;
                m.sell(amount, q);
                assert!(price_not_lower(before.r, before.s, m.r, m.s));
            }
            assert_eq!(m.s, locked);
            assert!(price(m.r, m.s) >= start_price, "ended below start, seed {seed}");
            let _ = (team_owed, last);
        }
    }
}
