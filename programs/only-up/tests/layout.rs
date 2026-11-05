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
        buy_fee_bps: 250,
        sell_fee_bps: 260,
        team_fee_bps: 50,
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
        team_fees_owed: 12,
        buy_fee_bps: 13,
        sell_fee_bps: 14,
        team_fee_bps: 15,
        buys_paused: true,
        created_at: 16,
        trade_count: 17,
        volume_lamports: 18,
    };
    let mut account = Vec::new();
    market.try_serialize(&mut account).unwrap();
    assert_eq!(account.len(), 8 + 147, "Market layout size changed: update the client");

    // A spread of quote inputs, from tiny to large.
    let mut quotes = Vec::new();
    let mut x: u64 = 0x1234_5678_9ABC_DEF1;
    let mut next = move || {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        x
    };
    for _ in 0..300 {
        let reserve = 10_000_000 + next() % 5_000_000_000_000;
        let supply = 1_000_000 + next() % 1_000_000_000_000_000;
        let amount = 1 + next() % 1_000_000_000_000;
        let buy_fee = (10 + next() % 491) as u16;
        let sell_fee = (10 + next() % 491) as u16;
        let team = (next() % 201) as u16;

        let b = math::quote_buy(amount, reserve, supply, buy_fee, team);
        let s = math::quote_sell(amount % (supply - 1) + 1, reserve, supply, sell_fee, team);
        quotes.push(format!(
            concat!(
                "{{\"reserve\":\"{}\",\"supply\":\"{}\",\"amount\":\"{}\",\"buyFee\":{},\"sellFee\":{},\"team\":{},",
                "\"buy\":{},\"sell\":{}}}"
            ),
            reserve,
            supply,
            amount,
            buy_fee,
            sell_fee,
            team,
            b.map(|q| format!("[\"{}\",\"{}\",\"{}\"]", q.team_fee, q.to_reserve, q.tokens_out))
                .unwrap_or_else(|| "null".into()),
            s.map(|q| format!("[\"{}\",\"{}\",\"{}\",\"{}\"]", q.gross, q.team_fee, q.user_out, q.holder_fee))
                .unwrap_or_else(|| "null".into()),
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
