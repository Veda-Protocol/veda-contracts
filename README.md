# veda-contracts

Soroban WASM smart contracts for **Veda Protocol** — a decentralized machine
learning compute marketplace. Instead of re-running expensive training
on-chain, Veda offloads execution to distributed GPU nodes (*Solvers*) and uses
zero-knowledge proofs, probabilistic spot-checks, and state-channel
checkpoints to settle verification on Stellar.

## Repository layout

```
veda-contracts/
├── Cargo.toml                       # Cargo workspace manifest
├── Makefile                         # build / test / bindings / clean
└── contracts/
    ├── core_registry/               # Job lifecycle + verification + settlement
    │   ├── Cargo.toml
    │   └── src/lib.rs
    └── escrow_vault/                # SAC-backed bounty custody
        ├── Cargo.toml
        └── src/lib.rs
```

## Contracts

### `core_registry`

Tracks the full lifecycle of a compute job.

| State | Meaning |
|-------|---------|
| `Created` | Job registered with a locked budget; awaiting a solver. |
| `Running` | A solver has been assigned and is executing the task. |
| `Verified` | A valid ZK execution proof was submitted and accepted. |
| `Settled` | Escrow was released to the solver; job is complete. |
| `Slashed` | The job was penalized (e.g. invalid/failed execution). |

Key entrypoints:

- `initialize(env, escrow_vault, token)` — one-time configuration.
- `create_job(env, client, job_id, budget)` — `client.require_auth()`, persists
  the `Job` into `Persistent` storage, extends TTL rent, and emits a
  `job_created` event.
- `assign_solver(env, job_id, solver)` — assigns a solver and moves the job to
  `Running`.
- `verify_and_settle(env, job_id, proof_hash: BytesN<32>)` — verifies the
  solver's execution proof hash, emits `job_verified`, and instructs the
  `escrow_vault` to release the bounty to the solver before marking the job
  `Settled`.
- `slash(env, job_id)` — marks a job `Slashed`.
- `get_job_status`, `get_job`, `get_proof` — read helpers.

### `escrow_vault`

Non-custodial bounty custody built on the Stellar Asset Contract (SAC).

- `initialize(env, registry)` — only the configured `registry` may release funds.
- `deposit_and_lock(env, token, from, amount)` — wraps the client's classic
  asset into the vault via `StellarAssetClient::deposit` and records the locked
  balance per owner.
- `release_payment(env, token, to, amount)` — callable only by the registry;
  transfers wrapped funds to the solver via `token::Client`.
- `locked_balance(env, token, owner)` — read helper.

## Verification lifecycle

1. **Register** — the ML engineer calls `create_job`, locking a budget in the
   `escrow_vault`.
2. **Execute** — a solver is assigned (`assign_solver`) and runs the training
   job off-chain (`veda-agent`).
3. **Prove** — the solver produces a SHA-256/ZK execution-trace proof.
4. **Verify & settle** — `verify_and_settle` records the `proof_hash` on-chain,
   emits `job_verified`, and releases escrow to the solver.
5. **Dispute** — the client may call `slash` to penalize a solver before
   settlement.

## Prerequisites

- [Rust](https://rustup.rs) (1.81+)
- [Stellar CLI](https://soroban.stellar.org/docs/cli) (`stellar`)

Install the Soroban WASM target:

```bash
rustup target add wasm32v1-none
```

## Build, test, bindings

```bash
make build      # stellar contract build (wasm32v1-none)
make test       # cargo test --workspace
make bindings   # regenerate TypeScript bindings into veda-sdk
make clean      # cargo clean && rm -rf target
```

## Notes

- All entrypoints are `#![no_std]` and compile to the `wasm32v1-none` target.
- Storage uses `Persistent` entries with explicit TTL extension on every write.
- ZK verification is represented by the submission of a `BytesN<32>` proof
  hash; full on-chain proof verification would plug into a dedicated verifier
  contract without changing this registry's interface.
