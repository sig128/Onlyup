# ONLY UP

A Solana token whose price in SOL can never go down.

Program address: `3ooGGJTLiadrijsQPW9AfTQLrgawhSkEYJPrPQpjnGxS`

## How it works

The contract is the market. It holds a safe of SOL, and the price of one token is the SOL in the safe divided by the number of tokens.

- **Buying:** you pay SOL into the safe and the contract creates new tokens for you at today's price, plus the buy fee.
- **Selling:** you hand tokens back, the contract destroys them and pays you today's price from the safe, minus the sell fee.
- **Fees stay in the safe for holders, instantly.** The holder share of every trade joins the price the moment the trade lands, so after every buy and every sell there is a little more SOL per token. Nothing to claim, nothing to wait for — every token you hold is just worth a bit more.

Example with a 1% fee: the safe holds 100 SOL and there are 100,000 tokens (price 0.001 SOL). Alice buys for 10 SOL and gets about 9,900 tokens; the price is now 0.0010009. Bob sells 5,000 tokens and gets about 4.95 SOL; the price is now 0.0010014. Both trades pushed the price up. (This exact example is checked in the tests.)

## Rules built into the contract

- Every buy and sell re-checks that the price did not fall. If it would have, the whole transaction is cancelled.
- Selling can never be paused and never costs more than the fixed sell fee.
- The admin can only pause buying, change the treasury wallet, and give up admin rights for good (which also switches buying back on). The admin cannot touch the safe, change fees, or mint tokens.
- There is no instruction that takes the safe's SOL out, except paying sellers.
- The creator fee is booked separately from the safe and can only be sent to the treasury wallet, which must be an ordinary wallet.
- The sell fee is always at least the buy fee. A buy can lift the price by at most the buy fee, so an exit that costs at least as much means buying right before someone and selling right after can never turn a profit — trades can't be sandwiched, even though fees land instantly. (Proven in the tests across 50,000 random attempts.)
- Only the mint's own authority can create its market, and the market takes over the mint authority in that same step. Nobody else can ever mint, and the token has no freeze authority, so no wallet can be frozen.
- The seed SOL and seed tokens are locked forever, so the safe can never be emptied.
- Limits are fixed in code: buy fee 0.1–5%, sell fee 0.1–5% (and at least the buy fee), creator fee 0–2%.

**What it does not do:** the price is in SOL, so the dollar value moves with SOL. Outside pools (Meteora, Raydium) can trade slightly below the contract's price, within the fee gap, so their charts can show small red candles. And the guarantees only become permanent once the program is locked (see the launch checklist): until then, whoever holds the upgrade key could replace the code.

## Settings

All are chosen at launch in `scripts/launch.config.json` and cannot be changed afterwards. The defaults charge 3% on every buy and every sell: 2.5% to holders and 0.5% to the creator.

| Setting | Default | Allowed | Meaning |
|---|---|---|---|
| `buyFeeBps` | 250 (2.5%) | 10–500 | Holders' share of every buy; joins the price at once |
| `sellFeeBps` | 250 (2.5%) | 10–500, and ≥ buy fee | Holders' share of every sell; joins the price at once |
| `teamFeeBps` | 50 (0.5%) | 0–200 | Creator's cut of every buy and sell, claimable to the treasury |
| `seedSol` | 0.1 | at least 0.01 | SOL locked in the safe forever |
| `startPriceSol` | 0.000001 | at least 0.000001 | Starting price per token |
| `launchBuySol` | 0 | any | Optional first buy by the deployer (these tokens are not locked) |
| `metadataMutable` | false | true / false | false = name, symbol and image can never be changed |

The market cap is the safe: the seed SOL plus everything bought since, minus what sellers took out.

## Folder

- `programs/only-up/src` — the on-chain program (Anchor 0.31)
- `programs/only-up/src/math.rs` — all pricing math, with its own tests (including 50,000 random sandwich attempts, all of which lose money)
- `programs/only-up/tests/integration.rs` — full-transaction tests against the real SPL Token and System programs
- `programs/only-up/tests/layout.rs` — writes the fixtures that prove the client matches the program
- `scripts/` — `deploy.ts` (launch the token), `status.ts` (live numbers), `trade.ts` (buy or sell from a wallet file), `lib.ts` (client library for a website), `check-layout.ts` (checks the client against the fixtures)
- `.github/workflows/` — builds the program file and runs every test on GitHub

## Commands

```bash
# all tests (math, full transactions, client layout fixtures)
cargo test -p only-up
cd scripts && npm install && npx tsx check-layout.ts

# build the on-chain program file (needs the Solana CLI)
cargo build-sbf --manifest-path programs/only-up/Cargo.toml --sbf-out-dir target/deploy

# deploy the program to the address above
solana program deploy target/deploy/only_up.so --program-id keys/only_up-program-keypair.json

# launch the token on it
cd scripts && cp launch.config.example.json launch.config.json  # edit it
npx tsx deploy.ts launch.config.json

# live numbers, and trading from a wallet file
npx tsx status.ts <rpcUrl> <mint>
npx tsx trade.ts <rpcUrl> <keypair.json> <mint> buy 0.5
npx tsx trade.ts <rpcUrl> <keypair.json> <mint> sell all
```

## Launch checklist

1. Run the tests and build on GitHub (the workflow does both on every push).
2. Deploy to devnet and launch with a devnet config. Buy and sell with `trade.ts`, watch `status.ts`.
3. Get an independent security audit before real money goes in.
4. Deploy to mainnet and run `deploy.ts` with the mainnet config (`metadataMutable: false`).
5. Lock the program so nobody can ever change it: `solana program set-upgrade-authority 3ooGGJTLiadrijsQPW9AfTQLrgawhSkEYJPrPQpjnGxS --final`. This cannot be undone.
6. Give up admin rights when you no longer need the buy pause (`renounce_admin`).
7. Optional: open a small Meteora pool (SOL plus tokens bought from the contract) so Axiom and DexScreener chart it, and run a bot that keeps its price in line with the contract.

`keys/only_up-program-keypair.json` is the key for the program address. Keep it private and out of git; it is only needed for the first deployment.

Not legal advice: describe this as a token with fixed, public rules, never as a promised return.
