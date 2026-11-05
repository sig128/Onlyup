//! Pure pricing math for ONLY UP v2 — a constant-product curve on top of an
//! unpierceable, ratcheting floor. No Solana types, so it is unit-tested
//! directly. This is the exact integer logic validated by the standalone
//! 2,000-market / 600,000-trade stress test (floor monotonic, always solvent,
//! no sandwich/arbitrage, upside intact).
//!
//! Layout: `rc` = curve SOL reserve, `tc` = virtual token reserve
//! (total_supply - supply), `rf` = floor reserve, `s` = circulating supply.
//! Curve price = rc/tc. Floor (always redeemable) = rf/s, and it can only rise.
//! All rounding favors the protocol.

use crate::constants::{BPS, CREATOR_FEE_BPS, FLOOR_BPS};

#[inline]
fn ceil_div(a: u128, b: u128) -> u128 {
    (a + b - 1) / b
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BuyQuote {
    /// Creator share of `sol`, booked in the vault until claimed.
    pub creator_fee: u64,
    /// Lamports added to the floor reserve.
    pub to_floor: u64,
    /// Lamports added to the curve reserve.
    pub to_curve: u64,
    /// Tokens minted to the buyer.
    pub tokens_out: u64,
    /// Curve SOL reserve after the buy.
    pub new_curve_sol: u64,
}

/// Quote a buy of `sol` lamports.
///
/// 25% of the post-fee SOL funds the floor, 75% drives the curve — except the
/// floor share is raised whenever that is what it takes to keep `rf/s` from
/// falling as new tokens are minted. That single rule makes the floor monotonic
/// by construction and also stops anyone minting below the floor to redeem for
/// a profit.
pub fn quote_buy(sol: u64, rc: u64, tc: u64, rf: u64, s: u64) -> Option<BuyQuote> {
    if rc == 0 || tc == 0 || sol == 0 {
        return None;
    }
    let creator_fee = u64::try_from((sol as u128) * CREATOR_FEE_BPS / BPS).ok()?;
    let avail = sol.checked_sub(creator_fee)?;
    let base_floor = u64::try_from((avail as u128) * FLOOR_BPS / BPS).ok()?;
    let k = (rc as u128) * (tc as u128);

    // Tentative mint with the default 75% on the curve.
    let to_curve0 = avail.checked_sub(base_floor)?;
    if to_curve0 == 0 {
        return None;
    }
    let rc_a = rc.checked_add(to_curve0)?;
    let tc_a = ceil_div(k, rc_a as u128);
    let out0 = (tc as u128).checked_sub(tc_a)? as u64;

    // Anti-dilution: require to_floor >= ceil(rf * out0 / s) so the floor rf/s
    // cannot fall when the new tokens join supply. Using out0 (the max possible
    // out) over-funds slightly, which is the safe direction.
    let required = if s > 0 {
        u64::try_from(ceil_div((rf as u128) * (out0 as u128), s as u128)).ok()?
    } else {
        0
    };
    let to_floor = base_floor.max(required).min(avail);
    let to_curve = avail - to_floor;

    if to_curve == 0 {
        // The curve is at or below the floor. Don't hand out cheap tokens: mint
        // at the floor price F = rf/s with all SOL into the floor reserve, which
        // keeps rf/s exactly flat and nudges the curve price back up.
        if s == 0 || rf == 0 {
            return None;
        }
        let out = u64::try_from((avail as u128) * (s as u128) / (rf as u128)).ok()?;
        if out == 0 || (out as u128) >= tc as u128 {
            return None;
        }
        return Some(BuyQuote {
            creator_fee,
            to_floor: avail,
            to_curve: 0,
            tokens_out: out,
            new_curve_sol: rc,
        });
    }

    let rc2 = rc.checked_add(to_curve)?;
    let tc2 = ceil_div(k, rc2 as u128); // round the token reserve UP -> mint fewer tokens (favor holders)
    if tc2 >= tc as u128 {
        return None;
    }
    let out = tc - tc2 as u64;
    if out == 0 {
        return None;
    }
    Some(BuyQuote {
        creator_fee,
        to_floor,
        to_curve,
        tokens_out: out,
        new_curve_sol: rc2,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SellQuote {
    /// True when the payout came from the floor reserve, not the curve.
    pub from_floor: bool,
    /// Gross value of the tokens at the better of curve/floor, before the fee.
    pub gross: u64,
    /// Creator share of `gross`, booked in the vault until claimed.
    pub creator_fee: u64,
    /// Lamports paid to the seller.
    pub user_out: u64,
    /// Curve SOL reserve after the sell.
    pub new_curve_sol: u64,
    /// Floor reserve after the sell.
    pub new_floor_reserve: u64,
}

/// Quote a sale of `tok` base units. The seller gets whichever is larger: the
/// constant-product curve payout, or redeeming at the floor `rf/s`. The floor
/// payout rounds down, so the floor reserve always covers it, and a floor
/// redemption leaves `rf/s` unchanged — never lower.
pub fn quote_sell(tok: u64, rc: u64, tc: u64, rf: u64, s: u64) -> Option<SellQuote> {
    if s == 0 || tok == 0 || tok >= s {
        return None;
    }
    let floor_out = u64::try_from((rf as u128) * (tok as u128) / (s as u128)).ok()?;
    let k = (rc as u128) * (tc as u128);
    let tc_new = tc as u128 + tok as u128;
    let rc2_curve = ceil_div(k, tc_new); // round the reserve-left UP -> pay less (favor pool)
    let curve_out = rc.saturating_sub(rc2_curve as u64);

    let (from_floor, gross, new_curve_sol, new_floor_reserve) = if curve_out >= floor_out {
        (false, curve_out, rc2_curve as u64, rf) // normal curve sell
    } else {
        (true, floor_out, rc, rf.checked_sub(floor_out)?) // redeem at the floor
    };
    let creator_fee = u64::try_from((gross as u128) * CREATOR_FEE_BPS / BPS).ok()?;
    let user_out = gross.checked_sub(creator_fee)?;
    Some(SellQuote {
        from_floor,
        gross,
        creator_fee,
        user_out,
        new_curve_sol,
        new_floor_reserve,
    })
}

/// True if `new_rf / new_s >= old_rf / old_s` — the floor did not fall. Exact,
/// cross-multiplied, no rounding. With no holders left (`new_s == 0`) it holds.
pub fn floor_not_lower(old_rf: u64, old_s: u64, new_rf: u64, new_s: u64) -> bool {
    if new_s == 0 {
        return true;
    }
    (new_rf as u128) * (old_s as u128) >= (old_rf as u128) * (new_s as u128)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Rng(u64);
    impl Rng {
        fn new(seed: u64) -> Self {
            Rng(seed ^ 0x9E37_79B9_7F4A_7C15)
        }
        fn u(&mut self) -> u64 {
            let mut x = self.0;
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            self.0 = x;
            x
        }
        fn f(&mut self) -> f64 {
            (self.u() >> 11) as f64 / ((1u64 << 53) as f64)
        }
    }

    const SOL: u64 = 1_000_000_000;
    const TOTAL: u64 = 1_000_000_000_000_000; // 1e9 tokens @ 6 decimals (asymptotic cap)

    /// The core guarantee: across thousands of random markets and full exits,
    /// the floor never falls, both reserves stay solvent, and payouts are covered.
    #[test]
    fn floor_is_monotonic_and_always_solvent() {
        for seed in 1..=1500u64 {
            let mut rng = Rng::new(seed.wrapping_mul(2_654_435_761) | 1);
            let mut rc = ((1.0 + rng.f() * 20.0) * SOL as f64) as u64;
            let mut rf = 0u64;
            let mut s = 0u64;
            let mut holders: Vec<u64> = Vec::new();

            for _ in 0..300 {
                let tc = TOTAL - s;
                if holders.is_empty() || rng.f() < 0.5 {
                    let sol = ((0.01 + rng.f() * 30.0) * SOL as f64) as u64;
                    if let Some(q) = quote_buy(sol, rc, tc, rf, s) {
                        let ns = s + q.tokens_out;
                        let nrf = rf + q.to_floor;
                        assert!(floor_not_lower(rf, s, nrf, ns), "buy lowered floor, seed {seed}");
                        assert!(q.new_curve_sol >= rc, "curve reserve fell on buy, seed {seed}");
                        rc = q.new_curve_sol;
                        rf = nrf;
                        s = ns;
                        holders.push(q.tokens_out);
                    }
                } else {
                    let i = (rng.u() % holders.len() as u64) as usize;
                    let amt = (((holders[i] as f64) * (0.1 + 0.9 * rng.f())) as u64)
                        .max(1)
                        .min(holders[i]);
                    if amt >= s {
                        continue;
                    }
                    if let Some(q) = quote_sell(amt, rc, tc, rf, s) {
                        if q.from_floor {
                            assert!(q.gross <= rf, "floor can't cover payout, seed {seed}");
                        } else {
                            assert!(q.gross <= rc, "curve can't cover payout, seed {seed}");
                        }
                        let ns = s - amt;
                        assert!(
                            floor_not_lower(rf, s, q.new_floor_reserve, ns),
                            "sell lowered floor, seed {seed}"
                        );
                        rc = q.new_curve_sol;
                        rf = q.new_floor_reserve;
                        s = ns;
                        holders[i] -= amt;
                        if holders[i] == 0 {
                            holders.remove(i);
                        }
                    }
                }
            }
        }
    }

    /// An early holder peaks with real upside and can never do worse than a
    /// floor above their entry, even after the crowd dumps 70% of supply.
    #[test]
    fn early_holder_keeps_a_rising_floor_through_a_dump() {
        let mut rc = 5 * SOL;
        let mut rf = 0u64;
        let mut s = 0u64;
        fn buy(rc: &mut u64, rf: &mut u64, s: &mut u64, sol: u64) -> u64 {
            let tc = TOTAL - *s;
            let q = quote_buy(sol, *rc, tc, *rf, *s).unwrap();
            *rc = q.new_curve_sol;
            *rf += q.to_floor;
            *s += q.tokens_out;
            q.tokens_out
        }
        for _ in 0..15 {
            buy(&mut rc, &mut rf, &mut s, SOL);
        }
        let my = buy(&mut rc, &mut rf, &mut s, 10 * SOL);
        for i in 0..120u64 {
            buy(&mut rc, &mut rf, &mut s, (2 + i % 6) * SOL);
        }
        let denom = (10 * SOL) as f64;
        let peak = my as f64 * (rc as f64 / (TOTAL - s) as f64) / denom;
        let floor_at_peak = my as f64 * (rf as f64 / s as f64) / denom;
        assert!(peak > 10.0, "early holder should see real upside, got {peak}x");
        assert!(floor_at_peak > 1.0, "floor should exceed entry, got {floor_at_peak}x");

        let target = (s as u128 * 70 / 100) as u64;
        let mut sold = 0u64;
        while sold < target {
            let c = (s * 3 / 100).max(1);
            if c >= s {
                break;
            }
            let tc = TOTAL - s;
            if let Some(q) = quote_sell(c, rc, tc, rf, s) {
                rc = q.new_curve_sol;
                rf = q.new_floor_reserve;
                s -= c;
                sold += c;
            } else {
                break;
            }
        }
        let realize =
            my as f64 * (rc as f64 / (TOTAL - s) as f64).max(rf as f64 / s as f64) / denom;
        assert!(
            realize >= floor_at_peak * 0.99,
            "after a dump the holder should still realize at least their floor, got {realize}x"
        );
    }

    #[test]
    fn cannot_sell_the_whole_supply() {
        assert!(quote_sell(100, 5 * SOL, TOTAL, 0, 100).is_none());
        assert!(quote_sell(99, 5 * SOL, TOTAL - 100, 1_000_000, 100).is_some());
    }
}
