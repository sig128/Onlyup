#![allow(deprecated)] // solana_sdk::system_instruction is fine for tests
//! Full-transaction tests. The ONLY UP program runs natively inside a local
//! Solana bank, together with the real SPL Token and System programs.

use anchor_lang::error::ErrorCode as AnchorError;
use anchor_lang::{AccountDeserialize, InstructionData, ToAccountMetas};
use only_up::constants::*;
use only_up::errors::OnlyUpError;
use only_up::math;
use only_up::state::Market;
use only_up::InitParams;
use solana_program_test::{processor, BanksClientError, ProgramTest, ProgramTestContext};
use solana_sdk::{
    account_info::AccountInfo,
    clock::Clock,
    compute_budget::ComputeBudgetInstruction,
    entrypoint::ProgramResult,
    instruction::{Instruction, InstructionError},
    program_option::COption,
    program_pack::Pack,
    pubkey::Pubkey,
    signature::Keypair,
    signer::Signer,
    system_instruction, system_program,
    transaction::{Transaction, TransactionError},
};
use spl_associated_token_account::{
    get_associated_token_address, instruction::create_associated_token_account_idempotent,
};

const SOL: u64 = 1_000_000_000;

fn process_instruction(program_id: &Pubkey, accounts: &[AccountInfo], data: &[u8]) -> ProgramResult {
    // Anchor's entry wants one lifetime for the slice and its items.
    let accounts = Box::leak(Box::new(accounts.to_vec()));
    only_up::entry(program_id, accounts, data)
}

// ---------- harness ----------

struct Env {
    ctx: ProgramTestContext,
    nonce: u32,
}

#[derive(Clone, Copy)]
struct Mkt {
    mint: Pubkey,
    market: Pubkey,
    vault: Pubkey,
    locked: Pubkey,
}

fn pdas(mint: &Pubkey) -> Mkt {
    let (market, _) = Pubkey::find_program_address(&[MARKET_SEED, mint.as_ref()], &only_up::ID);
    let (vault, _) = Pubkey::find_program_address(&[VAULT_SEED, market.as_ref()], &only_up::ID);
    let (locked, _) = Pubkey::find_program_address(&[LOCKED_SEED, market.as_ref()], &only_up::ID);
    Mkt { mint: *mint, market, vault, locked }
}

/// Test settings: 3% buy fee, 3% sell fee, no creator share.
fn params() -> InitParams {
    InitParams {
        seed_lamports: SOL / 10,
        seed_tokens: SOL / 10 * 1_000,
        buy_fee_bps: 300,
        sell_fee_bps: 300,
        team_fee_bps: 0,
    }
}

/// The launch settings: 2.5% + 0.5% creator on each side = 3% / 3%.
fn launch_defaults() -> InitParams {
    InitParams { buy_fee_bps: 250, sell_fee_bps: 250, team_fee_bps: 50, ..params() }
}

fn custom_code(err: &BanksClientError) -> Option<u32> {
    match err {
        BanksClientError::TransactionError(TransactionError::InstructionError(_, InstructionError::Custom(c)))
        | BanksClientError::SimulationError {
            err: TransactionError::InstructionError(_, InstructionError::Custom(c)),
            ..
        } => Some(*c),
        _ => None,
    }
}

fn ours(e: OnlyUpError) -> u32 {
    e.into()
}

fn anchor(e: AnchorError) -> u32 {
    e.into()
}

impl Env {
    async fn new() -> Self {
        let mut pt = ProgramTest::new("only_up", only_up::ID, processor!(process_instruction));
        pt.prefer_bpf(false);
        let ctx = pt.start_with_context().await;
        Env { ctx, nonce: 0 }
    }

    fn payer(&self) -> Keypair {
        self.ctx.payer.insecure_clone()
    }

    async fn send(&mut self, ixs: &[Instruction], signers: &[&Keypair]) -> Result<(), BanksClientError> {
        self.nonce += 1;
        let mut all = vec![ComputeBudgetInstruction::set_compute_unit_limit(400_000 + self.nonce)];
        all.extend_from_slice(ixs);
        let payer = self.payer();
        let mut keys: Vec<&Keypair> = vec![&payer];
        keys.extend_from_slice(signers);
        let blockhash = self.ctx.banks_client.get_latest_blockhash().await.unwrap();
        let tx = Transaction::new_signed_with_payer(&all, Some(&payer.pubkey()), &keys, blockhash);
        self.ctx.banks_client.process_transaction(tx).await
    }

    async fn fund(&mut self, to: &Pubkey, lamports: u64) {
        let payer = self.payer();
        self.send(&[system_instruction::transfer(&payer.pubkey(), to, lamports)], &[])
            .await
            .unwrap();
    }

    async fn user(&mut self, lamports: u64) -> Keypair {
        let kp = Keypair::new();
        self.fund(&kp.pubkey(), lamports).await;
        kp
    }

    async fn lamports(&mut self, key: &Pubkey) -> u64 {
        self.ctx
            .banks_client
            .get_account(*key)
            .await
            .unwrap()
            .map(|a| a.lamports)
            .unwrap_or(0)
    }

    async fn market(&mut self, key: &Pubkey) -> Market {
        let acc = self.ctx.banks_client.get_account(*key).await.unwrap().unwrap();
        Market::try_deserialize(&mut acc.data.as_slice()).unwrap()
    }

    async fn mint_state(&mut self, mint: &Pubkey) -> spl_token::state::Mint {
        let acc = self.ctx.banks_client.get_account(*mint).await.unwrap().unwrap();
        spl_token::state::Mint::unpack(&acc.data).unwrap()
    }

    async fn supply(&mut self, mint: &Pubkey) -> u64 {
        self.mint_state(mint).await.supply
    }

    async fn balance(&mut self, token_account: &Pubkey) -> u64 {
        let acc = self.ctx.banks_client.get_account(*token_account).await.unwrap().unwrap();
        spl_token::state::Account::unpack(&acc.data).unwrap().amount
    }

    async fn now(&mut self) -> i64 {
        let clock: Clock = self.ctx.banks_client.get_sysvar().await.unwrap();
        clock.unix_timestamp
    }

    async fn rent0(&mut self) -> u64 {
        self.ctx.banks_client.get_rent().await.unwrap().minimum_balance(0)
    }

    /// Price as a fraction (reserve, supply).
    async fn price(&mut self, m: &Mkt) -> (u64, u64) {
        let reserve = self.market(&m.market).await.reserve;
        let supply = self.supply(&m.mint).await;
        (reserve, supply)
    }

    /// The vault holds exactly rent + reserve + creator fees (+ any stray SOL).
    async fn check_books(&mut self, m: &Mkt, stray: u64) {
        let mk = self.market(&m.market).await;
        let rent0 = self.rent0().await;
        assert_eq!(
            self.lamports(&m.vault).await,
            rent0 + mk.reserve + mk.team_fees_owed + stray,
            "vault does not match the books"
        );
    }

    async fn create_mint(&mut self, authority: &Pubkey, freeze: Option<&Pubkey>, decimals: u8) -> Keypair {
        let mint = Keypair::new();
        let payer = self.payer();
        let rent = self.ctx.banks_client.get_rent().await.unwrap();
        let len = spl_token::state::Mint::LEN;
        let ixs = [
            system_instruction::create_account(
                &payer.pubkey(),
                &mint.pubkey(),
                rent.minimum_balance(len),
                len as u64,
                &spl_token::ID,
            ),
            spl_token::instruction::initialize_mint2(&spl_token::ID, &mint.pubkey(), authority, freeze, decimals)
                .unwrap(),
        ];
        self.send(&ixs, &[&mint]).await.unwrap();
        mint
    }

    async fn ata(&mut self, owner: &Pubkey, mint: &Pubkey) -> Pubkey {
        let payer = self.payer();
        self.send(
            &[create_associated_token_account_idempotent(&payer.pubkey(), owner, mint, &spl_token::ID)],
            &[],
        )
        .await
        .unwrap();
        get_associated_token_address(owner, mint)
    }

    /// Creates a fresh mint owned by the payer, then the market for it.
    async fn try_launch(&mut self, p: InitParams, treasury: &Pubkey) -> (Mkt, Result<(), BanksClientError>) {
        let admin = self.payer();
        let mint = self.create_mint(&admin.pubkey(), None, 6).await;
        let m = pdas(&mint.pubkey());
        let res = self.send(&[ix_initialize(&admin.pubkey(), &m.mint, treasury, p)], &[]).await;
        (m, res)
    }

    async fn launch(&mut self, p: InitParams, treasury: &Pubkey) -> Mkt {
        let (m, res) = self.try_launch(p, treasury).await;
        res.unwrap();
        m
    }

    /// Buys and checks every number against the published math.
    async fn buy_exact(&mut self, m: &Mkt, who: &Keypair, ata: &Pubkey, sol_in: u64) -> math::BuyQuote {
        let mk = self.market(&m.market).await;
        let (reserve, supply) = (mk.reserve, self.supply(&m.mint).await);
        let q = math::quote_buy(sol_in, reserve, supply, mk.buy_fee_bps, mk.team_fee_bps).expect("quote");
        let bal = self.balance(ata).await;
        let lam = self.lamports(&who.pubkey()).await;
        self.send(&[ix_buy(&who.pubkey(), m, ata, sol_in, q.tokens_out)], &[who])
            .await
            .unwrap();
        assert_eq!(self.balance(ata).await - bal, q.tokens_out, "tokens received");
        assert_eq!(lam - self.lamports(&who.pubkey()).await, sol_in, "buyer pays exactly sol_in");
        let after = self.market(&m.market).await;
        assert_eq!(after.reserve, reserve + q.to_reserve);
        assert_eq!(after.team_fees_owed, mk.team_fees_owed + q.team_fee);
        assert_eq!(self.supply(&m.mint).await, supply + q.tokens_out);
        q
    }

    /// Sells and checks every number against the published math.
    async fn sell_exact(&mut self, m: &Mkt, who: &Keypair, ata: &Pubkey, tokens: u64) -> math::SellQuote {
        let mk = self.market(&m.market).await;
        let (reserve, supply) = (mk.reserve, self.supply(&m.mint).await);
        let q = math::quote_sell(tokens, reserve, supply, mk.sell_fee_bps, mk.team_fee_bps).expect("quote");
        let lam = self.lamports(&who.pubkey()).await;
        self.send(&[ix_sell(&who.pubkey(), m, ata, tokens, q.user_out)], &[who])
            .await
            .unwrap();
        assert_eq!(self.lamports(&who.pubkey()).await - lam, q.user_out, "SOL received");
        let after = self.market(&m.market).await;
        assert_eq!(after.reserve, reserve - q.user_out - q.team_fee);
        assert_eq!(after.team_fees_owed, mk.team_fees_owed + q.team_fee);
        assert_eq!(self.supply(&m.mint).await, supply - tokens);
        q
    }
}

fn higher_or_equal(old: (u64, u64), new: (u64, u64)) -> bool {
    (new.0 as u128) * (old.1 as u128) >= (old.0 as u128) * (new.1 as u128)
}

fn strictly_higher(old: (u64, u64), new: (u64, u64)) -> bool {
    (new.0 as u128) * (old.1 as u128) > (old.0 as u128) * (new.1 as u128)
}

// ---------- instruction builders ----------

fn ix_initialize(admin: &Pubkey, mint: &Pubkey, treasury: &Pubkey, params: InitParams) -> Instruction {
    let m = pdas(mint);
    Instruction {
        program_id: only_up::ID,
        accounts: only_up::accounts::Initialize {
            admin: *admin,
            mint: *mint,
            market: m.market,
            vault: m.vault,
            locked_tokens: m.locked,
            treasury: *treasury,
            token_program: spl_token::ID,
            system_program: system_program::ID,
        }
        .to_account_metas(None),
        data: only_up::instruction::Initialize { params }.data(),
    }
}

fn ix_buy(buyer: &Pubkey, m: &Mkt, to: &Pubkey, sol_in: u64, min_tokens_out: u64) -> Instruction {
    Instruction {
        program_id: only_up::ID,
        accounts: only_up::accounts::Buy {
            buyer: *buyer,
            mint: m.mint,
            market: m.market,
            vault: m.vault,
            buyer_tokens: *to,
            token_program: spl_token::ID,
            system_program: system_program::ID,
        }
        .to_account_metas(None),
        data: only_up::instruction::Buy { sol_in, min_tokens_out }.data(),
    }
}

fn ix_sell(seller: &Pubkey, m: &Mkt, from: &Pubkey, tokens_in: u64, min_sol_out: u64) -> Instruction {
    Instruction {
        program_id: only_up::ID,
        accounts: only_up::accounts::Sell {
            seller: *seller,
            mint: m.mint,
            market: m.market,
            vault: m.vault,
            seller_tokens: *from,
            token_program: spl_token::ID,
            system_program: system_program::ID,
        }
        .to_account_metas(None),
        data: only_up::instruction::Sell { tokens_in, min_sol_out }.data(),
    }
}

fn ix_donate(donor: &Pubkey, m: &Mkt, lamports: u64) -> Instruction {
    Instruction {
        program_id: only_up::ID,
        accounts: only_up::accounts::Donate {
            donor: *donor,
            market: m.market,
            vault: m.vault,
            system_program: system_program::ID,
        }
        .to_account_metas(None),
        data: only_up::instruction::Donate { lamports }.data(),
    }
}

fn ix_claim(m: &Mkt, treasury: &Pubkey) -> Instruction {
    Instruction {
        program_id: only_up::ID,
        accounts: only_up::accounts::ClaimTeamFees {
            market: m.market,
            vault: m.vault,
            treasury: *treasury,
            system_program: system_program::ID,
        }
        .to_account_metas(None),
        data: only_up::instruction::ClaimTeamFees {}.data(),
    }
}

fn admin_accounts(admin: &Pubkey, m: &Mkt) -> Vec<solana_sdk::instruction::AccountMeta> {
    only_up::accounts::AdminOnly { admin: *admin, market: m.market }.to_account_metas(None)
}

fn ix_pause(admin: &Pubkey, m: &Mkt, paused: bool) -> Instruction {
    Instruction {
        program_id: only_up::ID,
        accounts: admin_accounts(admin, m),
        data: only_up::instruction::SetBuysPaused { paused }.data(),
    }
}

fn ix_renounce(admin: &Pubkey, m: &Mkt) -> Instruction {
    Instruction {
        program_id: only_up::ID,
        accounts: admin_accounts(admin, m),
        data: only_up::instruction::RenounceAdmin {}.data(),
    }
}

fn ix_set_treasury(admin: &Pubkey, m: &Mkt, new_treasury: &Pubkey) -> Instruction {
    Instruction {
        program_id: only_up::ID,
        accounts: only_up::accounts::SetTreasury {
            admin: *admin,
            market: m.market,
            new_treasury: *new_treasury,
        }
        .to_account_metas(None),
        data: only_up::instruction::SetTreasury {}.data(),
    }
}

// ---------- tests ----------

#[tokio::test]
async fn price_rises_on_every_buy_and_sell() {
    let mut env = Env::new().await;
    let treasury = env.user(SOL).await.pubkey();
    let m = env.launch(launch_defaults(), &treasury).await;

    let start = env.price(&m).await;
    assert_eq!(start, (SOL / 10, SOL / 10 * 1_000));
    assert_eq!(env.balance(&m.locked).await, SOL / 10 * 1_000, "seed tokens are locked");

    let alice = env.user(100 * SOL).await;
    let bob = env.user(100 * SOL).await;
    let alice_ata = env.ata(&alice.pubkey(), &m.mint).await;
    let bob_ata = env.ata(&bob.pubkey(), &m.mint).await;

    let mut last = start;
    env.buy_exact(&m, &alice, &alice_ata, 5 * SOL).await;
    let p = env.price(&m).await;
    assert!(strictly_higher(last, p), "buy must raise the price");
    last = p;

    env.buy_exact(&m, &bob, &bob_ata, 10 * SOL).await;
    let p = env.price(&m).await;
    assert!(strictly_higher(last, p));
    last = p;

    let half = env.balance(&alice_ata).await / 2;
    env.sell_exact(&m, &alice, &alice_ata, half).await;
    let p = env.price(&m).await;
    assert!(strictly_higher(last, p), "sell must raise the price");
    last = p;

    let all = env.balance(&bob_ata).await;
    env.sell_exact(&m, &bob, &bob_ata, all).await;
    assert_eq!(env.balance(&bob_ata).await, 0);
    let p = env.price(&m).await;
    assert!(strictly_higher(last, p));

    env.check_books(&m, 0).await;
    assert_eq!(env.market(&m.market).await.trade_count, 4);
}

#[tokio::test]
async fn a_front_runner_loses_money() {
    // Buy right before someone else's buy, then sell straight after, in the
    // same moment. The sell fee is at least the buy fee, so this never pays.
    let cases = [
        (launch_defaults(), 100 * SOL, 100 * SOL, 50 * SOL),
        (launch_defaults(), 20 * SOL, 5 * SOL, 500 * SOL),
        (params(), 100 * SOL, 100 * SOL, 50 * SOL),
        (params(), 10 * SOL, SOL, 1_000 * SOL),
    ];
    for (p, warm, attack, victim_sol) in cases {
        let mut env = Env::new().await;
        let treasury = env.user(SOL).await.pubkey();
        let m = env.launch(p, &treasury).await;
        let whale = env.user(warm + SOL).await;
        let whale_ata = env.ata(&whale.pubkey(), &m.mint).await;
        env.send(&[ix_buy(&whale.pubkey(), &m, &whale_ata, warm, 1)], &[&whale])
            .await
            .unwrap();

        let attacker = env.user(attack + SOL).await;
        let victim = env.user(victim_sol + SOL).await;
        let a_ata = env.ata(&attacker.pubkey(), &m.mint).await;
        let v_ata = env.ata(&victim.pubkey(), &m.mint).await;
        let before = env.lamports(&attacker.pubkey()).await;
        env.send(&[ix_buy(&attacker.pubkey(), &m, &a_ata, attack, 1)], &[&attacker])
            .await
            .unwrap();
        env.send(&[ix_buy(&victim.pubkey(), &m, &v_ata, victim_sol, 1)], &[&victim])
            .await
            .unwrap();
        let held = env.balance(&a_ata).await;
        env.send(&[ix_sell(&attacker.pubkey(), &m, &a_ata, held, 0)], &[&attacker])
            .await
            .unwrap();
        let after = env.lamports(&attacker.pubkey()).await;
        assert!(
            after < before,
            "front-runner made money: warm {warm}, in {attack}, victim {victim_sol}, net {}",
            after as i128 - before as i128
        );
    }
}

#[tokio::test]
async fn selling_works_while_buying_is_paused() {
    let mut env = Env::new().await;
    let admin = env.payer();
    let treasury = env.user(SOL).await.pubkey();
    let m = env.launch(params(), &treasury).await;
    let alice = env.user(10 * SOL).await;
    let ata = env.ata(&alice.pubkey(), &m.mint).await;
    env.send(&[ix_buy(&alice.pubkey(), &m, &ata, SOL, 1)], &[&alice]).await.unwrap();

    let mallory = env.user(SOL).await;
    let err = env.send(&[ix_pause(&mallory.pubkey(), &m, true)], &[&mallory]).await.unwrap_err();
    assert_eq!(custom_code(&err), Some(anchor(AnchorError::ConstraintHasOne)));

    env.send(&[ix_pause(&admin.pubkey(), &m, true)], &[]).await.unwrap();
    let err = env.send(&[ix_buy(&alice.pubkey(), &m, &ata, SOL, 1)], &[&alice]).await.unwrap_err();
    assert_eq!(custom_code(&err), Some(ours(OnlyUpError::BuysPaused)));

    let all = env.balance(&ata).await;
    env.sell_exact(&m, &alice, &ata, all).await;

    env.send(&[ix_pause(&admin.pubkey(), &m, false)], &[]).await.unwrap();
    env.send(&[ix_buy(&alice.pubkey(), &m, &ata, SOL, 1)], &[&alice]).await.unwrap();
}

#[tokio::test]
async fn renounce_admin_is_final_and_unpauses() {
    let mut env = Env::new().await;
    let admin = env.payer();
    let treasury = env.user(SOL).await.pubkey();
    let m = env.launch(launch_defaults(), &treasury).await;
    env.send(&[ix_pause(&admin.pubkey(), &m, true)], &[]).await.unwrap();
    env.send(&[ix_renounce(&admin.pubkey(), &m)], &[]).await.unwrap();
    let mk = env.market(&m.market).await;
    assert_eq!(mk.admin, Pubkey::default());
    assert!(!mk.buys_paused, "renouncing switches buying back on");
    let err = env.send(&[ix_pause(&admin.pubkey(), &m, true)], &[]).await.unwrap_err();
    assert_eq!(custom_code(&err), Some(anchor(AnchorError::ConstraintHasOne)));
    let other = env.user(SOL).await.pubkey();
    let err = env.send(&[ix_set_treasury(&admin.pubkey(), &m, &other)], &[]).await.unwrap_err();
    assert_eq!(custom_code(&err), Some(anchor(AnchorError::ConstraintHasOne)));

    // Trading and creator fees keep working without an admin.
    let alice = env.user(10 * SOL).await;
    let ata = env.ata(&alice.pubkey(), &m.mint).await;
    env.buy_exact(&m, &alice, &ata, 5 * SOL).await;
    let before = env.lamports(&treasury).await;
    env.send(&[ix_claim(&m, &treasury)], &[]).await.unwrap();
    assert_eq!(env.lamports(&treasury).await - before, 5 * SOL / 200);
}

#[tokio::test]
async fn slippage_limits_are_enforced() {
    let mut env = Env::new().await;
    let treasury = env.user(SOL).await.pubkey();
    let m = env.launch(params(), &treasury).await;
    let alice = env.user(10 * SOL).await;
    let ata = env.ata(&alice.pubkey(), &m.mint).await;

    let (r, s) = env.price(&m).await;
    let q = math::quote_buy(SOL, r, s, 300, 0).unwrap();
    let err = env
        .send(&[ix_buy(&alice.pubkey(), &m, &ata, SOL, q.tokens_out + 1)], &[&alice])
        .await
        .unwrap_err();
    assert_eq!(custom_code(&err), Some(ours(OnlyUpError::SlippageExceeded)));
    env.buy_exact(&m, &alice, &ata, SOL).await;

    let bal = env.balance(&ata).await;
    let (r, s) = env.price(&m).await;
    let q = math::quote_sell(bal, r, s, 300, 0).unwrap();
    let err = env
        .send(&[ix_sell(&alice.pubkey(), &m, &ata, bal, q.user_out + 1)], &[&alice])
        .await
        .unwrap_err();
    assert_eq!(custom_code(&err), Some(ours(OnlyUpError::SlippageExceeded)));

    let err = env.send(&[ix_buy(&alice.pubkey(), &m, &ata, 0, 0)], &[&alice]).await.unwrap_err();
    assert_eq!(custom_code(&err), Some(ours(OnlyUpError::ZeroAmount)));
    let err = env.send(&[ix_sell(&alice.pubkey(), &m, &ata, 0, 0)], &[&alice]).await.unwrap_err();
    assert_eq!(custom_code(&err), Some(ours(OnlyUpError::ZeroAmount)));
    // A sale too small to return a single lamport is refused.
    let err = env.send(&[ix_sell(&alice.pubkey(), &m, &ata, 1, 0)], &[&alice]).await.unwrap_err();
    assert_eq!(custom_code(&err), Some(ours(OnlyUpError::ZeroOutput)));
}

#[tokio::test]
async fn nobody_can_sell_tokens_they_do_not_own() {
    let mut env = Env::new().await;
    let treasury = env.user(SOL).await.pubkey();
    let m = env.launch(params(), &treasury).await;
    let alice = env.user(10 * SOL).await;
    let ata = env.ata(&alice.pubkey(), &m.mint).await;
    env.send(&[ix_buy(&alice.pubkey(), &m, &ata, SOL, 1)], &[&alice]).await.unwrap();

    let mallory = env.user(SOL).await;
    let err = env
        .send(&[ix_sell(&mallory.pubkey(), &m, &ata, 1_000_000, 0)], &[&mallory])
        .await
        .unwrap_err();
    assert_eq!(custom_code(&err), Some(anchor(AnchorError::ConstraintTokenOwner)));
    // The locked seed tokens.
    let err = env
        .send(&[ix_sell(&mallory.pubkey(), &m, &m.locked, 1_000_000, 0)], &[&mallory])
        .await
        .unwrap_err();
    assert_eq!(custom_code(&err), Some(anchor(AnchorError::ConstraintTokenOwner)));
    let bal = env.balance(&ata).await;
    assert!(env
        .send(&[ix_sell(&alice.pubkey(), &m, &ata, bal + 1, 0)], &[&alice])
        .await
        .is_err());
}

#[tokio::test]
async fn fake_vault_or_wrong_market_is_rejected() {
    let mut env = Env::new().await;
    let treasury = env.user(SOL).await.pubkey();
    let m = env.launch(params(), &treasury).await;
    let other = env.launch(params(), &treasury).await;
    let alice = env.user(10 * SOL).await;
    let ata = env.ata(&alice.pubkey(), &m.mint).await;
    env.send(&[ix_buy(&alice.pubkey(), &m, &ata, SOL, 1)], &[&alice]).await.unwrap();

    let bad = Mkt { vault: other.vault, ..m };
    let err = env
        .send(&[ix_sell(&alice.pubkey(), &bad, &ata, 1_000_000, 0)], &[&alice])
        .await
        .unwrap_err();
    assert_eq!(custom_code(&err), Some(anchor(AnchorError::ConstraintSeeds)));

    let bad = Mkt { mint: m.mint, ..other };
    assert!(env
        .send(&[ix_sell(&alice.pubkey(), &bad, &ata, 1_000_000, 0)], &[&alice])
        .await
        .is_err());
    let other_ata = env.ata(&alice.pubkey(), &other.mint).await;
    let err = env
        .send(&[ix_buy(&alice.pubkey(), &m, &other_ata, SOL, 1)], &[&alice])
        .await
        .unwrap_err();
    assert_eq!(custom_code(&err), Some(anchor(AnchorError::ConstraintTokenMint)));
}

#[tokio::test]
async fn creator_fees_go_only_to_the_treasury_and_never_touch_the_reserve() {
    let mut env = Env::new().await;
    let treasury = env.user(SOL).await;
    let m = env.launch(launch_defaults(), &treasury.pubkey()).await;
    let alice = env.user(100 * SOL).await;
    let ata = env.ata(&alice.pubkey(), &m.mint).await;
    let b = env.buy_exact(&m, &alice, &ata, 10 * SOL).await;
    assert_eq!(b.team_fee, 10 * SOL / 200, "0.5% of the buy");
    let bal = env.balance(&ata).await;
    let s = env.sell_exact(&m, &alice, &ata, bal / 2).await;
    assert_eq!(s.team_fee, s.gross / 200, "0.5% of the sale");

    let mk = env.market(&m.market).await;
    assert_eq!(mk.team_fees_owed, b.team_fee + s.team_fee);
    let price_before = env.price(&m).await;

    let mallory = env.user(SOL).await;
    let err = env.send(&[ix_claim(&m, &mallory.pubkey())], &[]).await.unwrap_err();
    assert_eq!(custom_code(&err), Some(anchor(AnchorError::ConstraintAddress)));

    let t_before = env.lamports(&treasury.pubkey()).await;
    env.send(&[ix_claim(&m, &treasury.pubkey())], &[]).await.unwrap();
    assert_eq!(env.lamports(&treasury.pubkey()).await - t_before, mk.team_fees_owed);
    let after = env.market(&m.market).await;
    assert_eq!(after.team_fees_owed, 0);
    assert_eq!(after.reserve, mk.reserve, "claiming never touches the reserve");
    assert_eq!(env.price(&m).await, price_before);
    env.check_books(&m, 0).await;
    let err = env.send(&[ix_claim(&m, &treasury.pubkey())], &[]).await.unwrap_err();
    assert_eq!(custom_code(&err), Some(ours(OnlyUpError::NothingToClaim)));
}

#[tokio::test]
async fn the_treasury_must_be_an_ordinary_wallet() {
    let mut env = Env::new().await;
    let admin = env.payer();
    let wallet = env.user(SOL).await.pubkey();
    let some_mint = env.create_mint(&admin.pubkey(), None, 6).await;
    let token_account = env.ata(&wallet, &some_mint.pubkey()).await;

    let (_, res) = env.try_launch(launch_defaults(), &token_account).await;
    assert_eq!(custom_code(&res.unwrap_err()), Some(anchor(AnchorError::AccountNotSystemOwned)));

    let m = env.launch(launch_defaults(), &wallet).await;
    let err = env
        .send(&[ix_set_treasury(&admin.pubkey(), &m, &token_account)], &[])
        .await
        .unwrap_err();
    assert_eq!(custom_code(&err), Some(anchor(AnchorError::AccountNotSystemOwned)));
    let mallory = env.user(SOL).await;
    let err = env
        .send(&[ix_set_treasury(&mallory.pubkey(), &m, &mallory.pubkey())], &[&mallory])
        .await
        .unwrap_err();
    assert_eq!(custom_code(&err), Some(anchor(AnchorError::ConstraintHasOne)));

    let fresh = Keypair::new().pubkey();
    env.send(&[ix_set_treasury(&admin.pubkey(), &m, &fresh)], &[]).await.unwrap();
    assert_eq!(env.market(&m.market).await.treasury, fresh);
    let alice = env.user(10 * SOL).await;
    let ata = env.ata(&alice.pubkey(), &m.mint).await;
    env.buy_exact(&m, &alice, &ata, 5 * SOL).await;
    let err = env.send(&[ix_claim(&m, &wallet)], &[]).await.unwrap_err();
    assert_eq!(custom_code(&err), Some(anchor(AnchorError::ConstraintAddress)));
    env.send(&[ix_claim(&m, &fresh)], &[]).await.unwrap();
    assert_eq!(env.lamports(&fresh).await, 5 * SOL / 200);
}

#[tokio::test]
async fn donations_and_direct_burns_raise_the_price_and_stray_sol_is_harmless() {
    let mut env = Env::new().await;
    let treasury = env.user(SOL).await.pubkey();
    let m = env.launch(params(), &treasury).await;
    let alice = env.user(100 * SOL).await;
    let ata = env.ata(&alice.pubkey(), &m.mint).await;
    env.buy_exact(&m, &alice, &ata, 10 * SOL).await;

    let p0 = env.price(&m).await;
    env.send(&[ix_donate(&alice.pubkey(), &m, SOL)], &[&alice]).await.unwrap();
    let p1 = env.price(&m).await;
    assert!(strictly_higher(p0, p1), "donation raises the price at once");
    assert_eq!(p1.0, p0.0 + SOL);
    let err = env.send(&[ix_donate(&alice.pubkey(), &m, 0)], &[&alice]).await.unwrap_err();
    assert_eq!(custom_code(&err), Some(ours(OnlyUpError::ZeroAmount)));

    // Burning tokens directly in the token program also raises the price.
    let burn = spl_token::instruction::burn(&spl_token::ID, &ata, &m.mint, &alice.pubkey(), &[], 1_000_000)
        .unwrap();
    env.send(&[burn], &[&alice]).await.unwrap();
    let p2 = env.price(&m).await;
    assert!(strictly_higher(p1, p2), "direct burn raises the price");

    env.fund(&m.vault, 3 * SOL).await;
    assert_eq!(env.price(&m).await, p2);
    let bal = env.balance(&ata).await;
    env.sell_exact(&m, &alice, &ata, bal).await;
    assert!(strictly_higher(p2, env.price(&m).await));
    env.check_books(&m, 3 * SOL).await;
}

#[tokio::test]
async fn launching_hands_the_mint_to_the_market_for_good() {
    let mut env = Env::new().await;
    let admin = env.payer();
    let treasury = env.user(SOL).await.pubkey();
    let p = launch_defaults();
    let m = env.launch(p.clone(), &treasury).await;

    let mint = env.mint_state(&m.mint).await;
    assert_eq!(mint.mint_authority, COption::Some(m.market), "only the market can mint");
    assert_eq!(mint.freeze_authority, COption::None, "nobody can freeze tokens");
    assert_eq!(mint.supply, p.seed_tokens);
    assert_eq!(env.balance(&m.locked).await, p.seed_tokens, "seed tokens are locked");

    let mk = env.market(&m.market).await;
    let now = env.now().await;
    assert_eq!(mk.admin, admin.pubkey());
    assert_eq!(mk.treasury, treasury);
    assert_eq!(mk.reserve, p.seed_lamports);
    assert_eq!((mk.team_fees_owed, mk.trade_count, mk.volume_lamports), (0, 0, 0));
    assert_eq!(mk.created_at, now);
    assert_eq!((mk.buy_fee_bps, mk.sell_fee_bps, mk.team_fee_bps), (250, 250, 50));
    env.check_books(&m, 0).await;

    let admin_ata = env.ata(&admin.pubkey(), &m.mint).await;
    let mint_more = spl_token::instruction::mint_to(&spl_token::ID, &m.mint, &admin_ata, &admin.pubkey(), &[], 1)
        .unwrap();
    assert!(env.send(&[mint_more], &[]).await.is_err());
    assert!(env
        .send(&[ix_initialize(&admin.pubkey(), &m.mint, &treasury, launch_defaults())], &[])
        .await
        .is_err());
}

#[tokio::test]
async fn pre_funded_addresses_cannot_block_the_launch() {
    let mut env = Env::new().await;
    let admin = env.payer();
    let treasury = env.user(SOL).await.pubkey();
    let mint = env.create_mint(&admin.pubkey(), None, 6).await;
    let m = pdas(&mint.pubkey());
    env.fund(&m.market, 1_000_000).await;
    env.fund(&m.vault, 2 * SOL).await;
    env.fund(&m.locked, 1_000_000).await;
    env.send(&[ix_initialize(&admin.pubkey(), &m.mint, &treasury, params())], &[])
        .await
        .unwrap();
    let rent0 = env.rent0().await;
    env.check_books(&m, 2 * SOL - rent0).await;
    let alice = env.user(10 * SOL).await;
    let ata = env.ata(&alice.pubkey(), &m.mint).await;
    env.buy_exact(&m, &alice, &ata, SOL).await;
}

#[tokio::test]
async fn initialize_rejects_unsafe_mints_and_settings() {
    let mut env = Env::new().await;
    let admin = env.payer();
    let treasury = env.user(SOL).await.pubkey();

    // Someone else tries to create the market for our fresh mint first.
    let mint = env.create_mint(&admin.pubkey(), None, 6).await;
    let mallory = env.user(SOL).await;
    let evil = InitParams { buy_fee_bps: 250, sell_fee_bps: 250, team_fee_bps: 200, ..params() };
    let err = env
        .send(&[ix_initialize(&mallory.pubkey(), &mint.pubkey(), &mallory.pubkey(), evil)], &[&mallory])
        .await
        .unwrap_err();
    assert_eq!(custom_code(&err), Some(ours(OnlyUpError::BadMintAuthority)));
    env.send(&[ix_initialize(&admin.pubkey(), &mint.pubkey(), &treasury, params())], &[])
        .await
        .unwrap();

    // Freeze authority present.
    let mint = env.create_mint(&admin.pubkey(), Some(&admin.pubkey()), 6).await;
    let err = env
        .send(&[ix_initialize(&admin.pubkey(), &mint.pubkey(), &treasury, params())], &[])
        .await
        .unwrap_err();
    assert_eq!(custom_code(&err), Some(ours(OnlyUpError::FreezeAuthoritySet)));

    // Wrong decimals.
    let mint = env.create_mint(&admin.pubkey(), None, 9).await;
    let err = env
        .send(&[ix_initialize(&admin.pubkey(), &mint.pubkey(), &treasury, params())], &[])
        .await
        .unwrap_err();
    assert_eq!(custom_code(&err), Some(ours(OnlyUpError::WrongDecimals)));

    // Tokens already minted before launch.
    let mint = env.create_mint(&admin.pubkey(), None, 6).await;
    let admin_ata = env.ata(&admin.pubkey(), &mint.pubkey()).await;
    let pre_mint = spl_token::instruction::mint_to(&spl_token::ID, &mint.pubkey(), &admin_ata, &admin.pubkey(), &[], 5)
        .unwrap();
    env.send(&[pre_mint], &[]).await.unwrap();
    let err = env
        .send(&[ix_initialize(&admin.pubkey(), &mint.pubkey(), &treasury, params())], &[])
        .await
        .unwrap_err();
    assert_eq!(custom_code(&err), Some(ours(OnlyUpError::MintNotEmpty)));

    // Settings outside the hard limits.
    let cases: Vec<(InitParams, OnlyUpError)> = vec![
        (InitParams { buy_fee_bps: 0, ..params() }, OnlyUpError::BadFeeSettings),
        (InitParams { buy_fee_bps: 501, sell_fee_bps: 501, ..params() }, OnlyUpError::BadFeeSettings),
        (InitParams { sell_fee_bps: 501, ..params() }, OnlyUpError::BadFeeSettings),
        (InitParams { team_fee_bps: 201, ..params() }, OnlyUpError::BadFeeSettings),
        (InitParams { buy_fee_bps: 300, sell_fee_bps: 200, ..params() }, OnlyUpError::LopsidedFees),
        (InitParams { seed_lamports: 1_000, ..params() }, OnlyUpError::SeedTooSmall),
        (InitParams { seed_tokens: 999_999, ..params() }, OnlyUpError::SeedTooSmall),
        (InitParams { seed_tokens: SOL / 10 * 2_000, ..params() }, OnlyUpError::StartPriceTooLow),
    ];
    for (i, (p, want)) in cases.into_iter().enumerate() {
        let want = ours(want);
        let (_, res) = env.try_launch(p, &treasury).await;
        assert_eq!(custom_code(&res.unwrap_err()), Some(want), "case {i}");
    }
    // The limits themselves are allowed (buy == sell is fine).
    let edge = InitParams {
        buy_fee_bps: 500,
        sell_fee_bps: 500,
        team_fee_bps: 200,
        seed_lamports: MIN_SEED_LAMPORTS,
        seed_tokens: MIN_SEED_LAMPORTS * 1_000,
    };
    env.launch(edge, &treasury).await;
}

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

#[tokio::test]
async fn random_trading_never_lowers_the_price_and_everyone_can_exit() {
    for (seed, p) in [
        (0x5EED_1234_ABCD_9876u64, launch_defaults()),
        (0xBADC_0FFE_E0DD_F00Du64, InitParams { buy_fee_bps: 10, sell_fee_bps: 10, team_fee_bps: 0, ..params() }),
    ] {
        let mut env = Env::new().await;
        let treasury = env.user(SOL).await.pubkey();
        let m = env.launch(p.clone(), &treasury).await;

        let mut users = Vec::new();
        for _ in 0..5 {
            let u = env.user(1_000 * SOL).await;
            let ata = env.ata(&u.pubkey(), &m.mint).await;
            users.push((u, ata));
        }
        let start = env.price(&m).await;
        let mut last = start;
        let mut rng = Rng(seed);
        for step in 0..150 {
            let i = (rng.next() % users.len() as u64) as usize;
            let held = env.balance(&users[i].1).await;
            if held == 0 || rng.next() % 2 == 0 {
                let sol_in = rng.range(SOL / 100, 20 * SOL);
                let (u, ata) = &users[i];
                env.buy_exact(&m, u, ata, sol_in).await;
            } else {
                let amount = rng.range(1, held);
                let (r, s) = env.price(&m).await;
                let (u, ata) = &users[i];
                if math::quote_sell(amount, r, s, p.sell_fee_bps, p.team_fee_bps).unwrap().user_out == 0 {
                    let err = env.send(&[ix_sell(&u.pubkey(), &m, ata, amount, 0)], &[u]).await.unwrap_err();
                    assert_eq!(custom_code(&err), Some(ours(OnlyUpError::ZeroOutput)));
                } else {
                    env.sell_exact(&m, u, ata, amount).await;
                }
            }
            let now = env.price(&m).await;
            assert!(higher_or_equal(last, now), "price fell at step {step}");
            env.check_books(&m, 0).await;
            last = now;
        }

        for (u, ata) in &users {
            let held = env.balance(ata).await;
            if held > 0 {
                env.sell_exact(&m, u, ata, held).await;
            }
        }
        if p.team_fee_bps > 0 {
            env.send(&[ix_claim(&m, &treasury)], &[]).await.unwrap();
        }
        env.check_books(&m, 0).await;
        let end = env.price(&m).await;
        assert_eq!(end.1, p.seed_tokens, "only the locked seed is left");
        assert!(strictly_higher(start, end), "after all that trading the price is higher than at launch");
    }
}
