//! Writes byte-level fixtures that the TypeScript client checks against, so the
//! scripts and website encode instructions, decode accounts and quote trades
//! exactly like the program does.

use anchor_lang::prelude::Pubkey;
use anchor_lang::{AccountSerialize, InstructionData};
use only_up::math;
use only_up::state::Market;
use only_up::InitParams;

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

#[test]
fn write_layout_fixtures() {
    let params = InitParams {
        seed_lamports: 123_456_789,
        seed_tokens: 987_654_321_000,
        buy_fee_bps: 150,
        sell_fee_bps: 140,
        team_fee_bps: 50,
        max_hype_bps: 2_000,
        hype_full_bps: 1_900,
        hype_half_life_secs: 3_600,
        drip_secs: 1_800,
    };
    let market = Market {
        version: 1,
        bump: 254,
        vault_bump: 253,
        locked_bump: 252,
        mint: Pubkey::new_from_array([1; 32]),
        admin: Pubkey::new_from_array([2; 32]),
        treasury: Pubkey::new_from_array([3; 32]),
        reserve: 11,
        pending: 12,
        team_fees_owed: 13,
        heat: 14,
        heat_updated_at: -15,
        last_drip_at: 16,
        buy_fee_bps: 17,
        sell_fee_bps: 18,
        team_fee_bps: 19,
        max_hype_bps: 20,
        hype_full_bps: 21,
        hype_half_life_secs: 22,
        drip_secs: 23,
        buys_paused: true,
        created_at: 24,
        trade_count: 25,
        volume_lamports: 26,
    };
    let mut account = Vec::new();
    market.try_serialize(&mut account).unwrap();
    assert_eq!(account.len(), 8 + 191, "Market layout size changed: update the client");

    // A spread of quote inputs, from tiny to large.
    let mut quotes = Vec::new();
    let mut x: u64 = 0x1234_5678_9ABC_DEF1;
    let mut next = move || {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        x
    };
    for i in 0..300 {
        let reserve = 10_000_000 + next() % 5_000_000_000_000;
        let supply = 1_000_000 + next() % 1_000_000_000_000_000;
        let amount = 1 + next() % 1_000_000_000_000;
        let buy_fee = (10 + next() % 991) as u16;
        let sell_fee = (10 + next() % 491) as u16;
        let team = (next() % 201) as u16;
        let hype = (next() % 2_001) as u16;
        let max_hype = if i % 7 == 0 { 0 } else { (next() % 2_001) as u16 };
        let hype_full = (100 + next() % 9_901) as u16;
        // Some quotes with no heat, some calm, some in a rush.
        let heat = match i % 3 {
            0 => 0,
            1 => next() % (reserve / 10 + 1),
            _ => next() % (reserve * 3),
        };
        let elapsed = (next() % 500_000) as i64 - 1_000;
        let half_life = (60 + next() % 600_000) as u32;
        let drip = (60 + next() % 604_741) as u32;
        let pending = next() % 1_000_000_000_000;

        let b = math::quote_buy(amount, reserve, supply, buy_fee, team, hype);
        let full = math::quote_buy_full(amount, reserve, supply, heat, buy_fee, team, max_hype, hype_full);
        let s = math::quote_sell(amount % (supply - 1) + 1, reserve, supply, sell_fee, team, 0);
        let to_reserve = amount - amount * team as u64 / 10_000;
        quotes.push(format!(
            concat!(
                "{{\"reserve\":\"{}\",\"supply\":\"{}\",\"amount\":\"{}\",\"buyFee\":{},\"sellFee\":{},",
                "\"team\":{},\"hype\":{},\"maxHype\":{},\"hypeFull\":{},\"heat\":\"{}\",\"elapsed\":\"{}\",",
                "\"halfLife\":{},\"drip\":{},\"pending\":\"{}\",",
                "\"buy\":{},\"buyFull\":{},\"fullHype\":{},\"sell\":{},",
                "\"decayed\":\"{}\",\"hypeFromHeat\":{},\"buyHype\":{},\"vested\":\"{}\"}}"
            ),
            reserve,
            supply,
            amount,
            buy_fee,
            sell_fee,
            team,
            hype,
            max_hype,
            hype_full,
            heat,
            elapsed,
            half_life,
            drip,
            pending,
            b.map(|q| format!(
                "[\"{}\",\"{}\",\"{}\",\"{}\",\"{}\"]",
                q.team_fee, q.to_reserve, q.fair, q.to_pending, q.tokens_out
            ))
            .unwrap_or_else(|| "null".into()),
            full.map(|(q, _)| format!(
                "[\"{}\",\"{}\",\"{}\",\"{}\",\"{}\"]",
                q.team_fee, q.to_reserve, q.fair, q.to_pending, q.tokens_out
            ))
            .unwrap_or_else(|| "null".into()),
            full.map(|(_, h)| h as i64).unwrap_or(-1),
            s.map(|q| format!("[\"{}\",\"{}\",\"{}\",\"{}\"]", q.gross, q.team_fee, q.user_out, q.to_pending))
                .unwrap_or_else(|| "null".into()),
            math::decay_heat(heat, elapsed, half_life),
            math::hype_bps(heat, reserve, max_hype, hype_full),
            math::buy_hype_bps(heat, reserve, to_reserve, max_hype, hype_full),
            math::vest(pending, elapsed, drip),
        ));
    }

    let json = format!(
        concat!(
            "{{\n\"initialize\":\"{}\",\n\"buy\":\"{}\",\n\"sell\":\"{}\",\n\"donate\":\"{}\",\n",
            "\"claimTeamFees\":\"{}\",\n\"setBuysPaused\":\"{}\",\n\"setTreasury\":\"{}\",\n",
            "\"renounceAdmin\":\"{}\",\n\"market\":\"{}\",\n\"quotes\":[\n{}\n]\n}}\n"
        ),
        hex(&only_up::instruction::Initialize { params }.data()),
        hex(&only_up::instruction::Buy { sol_in: 5, min_tokens_out: 7 }.data()),
        hex(&only_up::instruction::Sell { tokens_in: 9, min_sol_out: 11 }.data()),
        hex(&only_up::instruction::Donate { lamports: 13 }.data()),
        hex(&only_up::instruction::ClaimTeamFees {}.data()),
        hex(&only_up::instruction::SetBuysPaused { paused: true }.data()),
        hex(&only_up::instruction::SetTreasury {}.data()),
        hex(&only_up::instruction::RenounceAdmin {}.data()),
        hex(&account),
        quotes.join(",\n"),
    );
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../scripts/fixtures");
    std::fs::create_dir_all(dir).unwrap();
    std::fs::write(format!("{dir}/layout.json"), json).unwrap();
}
