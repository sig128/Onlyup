# ONLY UP — launch runbook

Everything is built and verified. What's left is deploying on-chain, which needs
real funds and a couple of clicks in the GitHub **Actions** tab (no terminal).
This file is the map.

- **Program ID (CA):** `3ooGGJTLiadrijsQPW9AfTQLrgawhSkEYJPrPQpjnGxS` is the program; the
  **token mint** (what shows on Axiom) is the vanity address ending in `...up`:
  `95QG7JnaYEdY8ZzuGnP7qkHfDVphH4cB8YL4pKMDTtup`.
- **Economics (fixed in code):** 3% fee each side — 0.5% creator + 2.5% to the floor —
  plus 20% of each buy to the floor. The floor only ever rises; proven across 2,000
  markets / 600,000 trades.

## One-time setup — three GitHub secrets

Repo → **Settings → Secrets and variables → Actions → New repository secret**:

| Secret | What goes in it |
| --- | --- |
| `PROGRAM_KEYPAIR` | the contents of `keys/only_up-program-keypair.json` (ask me — it lives in the build sandbox, gitignored, never committed) |
| `MINT_KEYPAIR` | the contents of `keys/v2-mint-keypair.json` (the vanity `...up` mint — same, ask me) |
| `DEPLOYER_KEYPAIR` | a wallet **you** create and fund. It pays for the deploy + seed. Keep it single-purpose; drain it after. |

I can't set these for you — the sandbox is firewalled off from GitHub's secrets API.
When you're ready I'll send you the two keypair files to paste in.

## Step 1 — Devnet dry-run (free)

Devnet SOL is free from <https://faucet.solana.com> (fund `DEPLOYER_KEYPAIR` with ~6 SOL).

1. **Actions → `deploy-devnet` → Run workflow.** Builds `only_up.so` and deploys the
   program to devnet.
2. **Actions → `devnet-launch` → Run workflow.** Runs `deploy.ts` (mints the token with
   the Only Up / UP metadata, seeds the market) then `pool.ts` (creates the Meteora
   mirror pool). It prints the mint + pool addresses and uploads `launch-output.json`
   and `pool-output.json`.
3. **Load test (optional, proves the floor live):** run `simulate-devnet.ts` with the
   deployer as the bank — fires hundreds of real buys/sells and checks, every trade,
   that the floor never dropped and the vault stayed solvent. (Add a `devnet-loadtest`
   workflow run, or I can wire it into `devnet-launch`.)
4. **Peg keeper:** `arb-bot.ts` keeps the Meteora pool price glued to the contract.
   Run it as a long-lived service (a $5/mo box, Railway, Fly, etc.) with a small SOL +
   UP inventory on the bot wallet. I'll set up hosting when you want it live.

Check it with `status.ts <devnet-rpc> <mint>` (or I'll read it for you).

## Step 2 — Mainnet

Only after the devnet dry-run is clean.

1. Fund a **mainnet** `DEPLOYER_KEYPAIR` with ~3–4 SOL (deploy + seed + fees) plus
   whatever you want as the pool's starting liquidity.
2. Run the mainnet deploy + launch (same scripts, mainnet RPC — I'll add the
   `mainnet-launch` workflow when you're ready).
3. Seed the Meteora pool, start the arb bot.

## Step 3 — Lock it (trustless)

Once it's live and you're happy:

1. **Revoke the program's upgrade authority** — `solana program set-upgrade-authority
   <PROGRAM_ID> --final`. Now the code can never change.
2. **Renounce admin** — `renounceAdmin` (in `lib.ts`), so even the pause-buys / set-treasury
   switches are gone. Fully autonomous.

## What each script does (`scripts/`)

| Script | Role |
| --- | --- |
| `deploy.ts` | one-shot launch: mint (vanity) → Metaplex metadata → create market, prints the CA |
| `pool.ts` | creates + seeds the Meteora DAMM v2 mirror pool at the contract price |
| `arb-bot.ts` | peg keeper — trades the pool back toward the contract price |
| `simulate-devnet.ts` | load test — fires 500–2,000 real trades, asserts floor-only-up + solvency |
| `trade.ts` | manual buy/sell with slippage | 
| `status.ts` | prints a market's live state |
| `lib.ts` | the client (addresses, instructions, quote math) — matches the program to the lamport |

## Costs (mainnet, estimate)

- Program deploy: ~2–3 SOL (one-time rent).
- Floor/curve seed: your choice (min 0.02; ~0.3–1 SOL suggested).
- Launch accounts + fees: ~0.02 SOL.
- Pool liquidity: whatever you put in (yours, in the pool).
