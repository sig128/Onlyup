//! Writes byte-level fixtures that the TypeScript client checks against, so the
//! scripts and website encode instructions, decode accounts and quote trades
//! exactly like the v2 program does. Run `cargo test` to (re)generate them.

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
    };
    let market = Market {
        version: 2,
        bump: 254,
        vault_bump: 253,
        mint: Pubkey::new_from_array([1; 32]),
        admin: Pubkey::new_from_array([2; 32]),
        treasury: Pubkey::new_from_array([3; 32]),
        curve_sol: 11,
        floor_reserve: 12,
        total_supply: 13,
        creator_fees_owed: 14,
        buys_paused: true,
        created_at: 16,
        trade_count: 17,
        volume_lamports: 18,
    };
    let mut account = Vec::new();
    market.try_serialize(&mut account).unwrap();
    assert_eq!(account.len(), 8 + 156, "Market layout size changed: update the client");

    // A spread of quote inputs, from tiny to large, exercising both the curve
    // and floor-redemption paths.
    let mut quotes = Vec::new();
    let mut x: u64 = 0x1234_5678_9ABC_DEF1;
    let mut next = move || {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        x
    };
    for _ in 0..300 {
        let rc = 10_000_000 + next() % 5_000_000_000_000;
        let tc = 1_000_000_000 + next() % 1_000_000_000_000_000;
        let rf = next() % 20_000_000_000_000;
        let s = 1_000_000 + next() % 1_000_000_000_000_000;
        let buy_amount = 1 + next() % 1_000_000_000_000;
        let sell_tok = buy_amount % (s - 1) + 1;

        let b = math::quote_buy(buy_amount, rc, tc, rf, s);
        let sq = math::quote_sell(sell_tok, rc, tc, rf, s);
        quotes.push(format!(
            concat!(
                "{{\"rc\":\"{}\",\"tc\":\"{}\",\"rf\":\"{}\",\"s\":\"{}\",\"buyAmount\":\"{}\",\"sellTok\":\"{}\",",
                "\"buy\":{},\"sell\":{}}}"
            ),
            rc,
            tc,
            rf,
            s,
            buy_amount,
            sell_tok,
            b.map(|q| format!(
                "[\"{}\",\"{}\",\"{}\",\"{}\",\"{}\"]",
                q.creator_fee, q.to_floor, q.to_curve, q.tokens_out, q.new_curve_sol
            ))
            .unwrap_or_else(|| "null".into()),
            sq.map(|q| format!(
                "[{},\"{}\",\"{}\",\"{}\",\"{}\",\"{}\"]",
                q.from_floor, q.gross, q.creator_fee, q.user_out, q.new_curve_sol, q.new_floor_reserve
            ))
            .unwrap_or_else(|| "null".into()),
        ));
    }

    let json = format!(
        concat!(
            "{{\n\"initialize\":\"{}\",\n\"buy\":\"{}\",\n\"sell\":\"{}\",\n\"donate\":\"{}\",\n",
            "\"claimCreatorFees\":\"{}\",\n\"setBuysPaused\":\"{}\",\n\"setTreasury\":\"{}\",\n",
            "\"renounceAdmin\":\"{}\",\n\"market\":\"{}\",\n\"quotes\":[\n{}\n]\n}}\n"
        ),
        hex(&only_up::instruction::Initialize { params }.data()),
        hex(&only_up::instruction::Buy { sol_in: 5, min_tokens_out: 7 }.data()),
        hex(&only_up::instruction::Sell { tokens_in: 9, min_sol_out: 11 }.data()),
        hex(&only_up::instruction::Donate { lamports: 13 }.data()),
        hex(&only_up::instruction::ClaimCreatorFees {}.data()),
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
