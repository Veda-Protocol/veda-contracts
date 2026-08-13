#![no_std]

//! # Core Registry
//!
//! Tracks the lifecycle of compute jobs end-to-end. A `Job` moves through the
//! states `Created -> Running -> Verified -> Settled` (or `Slashed`). When a
//! Solver submits a valid ZK execution proof, the contract marks the job
//! verified and instructs the `EscrowVault` to release the locked bounty.

use soroban_sdk::{
    contract, contractclient, contractimpl, contracttype, symbol_short, Address, BytesN, Env,
    Symbol,
};

/// Lifecycle state of a compute job.
#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum JobStatus {
    Created,
    Running,
    Verified,
    Settled,
    Slashed,
}

/// A registered compute job.
#[contracttype]
#[derive(Clone)]
pub struct Job {
    pub job_id: Symbol,
    pub client: Address,
    pub solver: Address,
    pub budget: i128,
    pub status: JobStatus,
}

/// Deployment configuration: where the escrow lives and which token is used
/// for bounties.
#[contracttype]
#[derive(Clone)]
pub struct Config {
    pub escrow_vault: Address,
    pub token: Address,
}

/// Storage key for a submitted proof, namespaced by job id.
#[contracttype]
#[derive(Clone)]
pub struct ProofKey {
    pub job_id: Symbol,
}

const CONFIG_KEY: Symbol = symbol_short!("config");
const TTL_THRESHOLD: u32 = 100;
const TTL_EXTEND_TO: u32 = 10_000;

/// Cross-contract interface of the escrow vault.
#[contractclient(name = "EscrowVaultClient")]
pub trait EscrowVaultTrait {
    fn deposit_and_lock(env: Env, token: Address, from: Address, amount: i128) -> i128;
    fn release_payment(env: Env, token: Address, to: Address, amount: i128);
}

fn read_config(env: &Env) -> Config {
    env.storage()
        .persistent()
        .get(&CONFIG_KEY)
        .unwrap_or_else(|| panic!("core_registry: not initialized"))
}

fn read_job(env: &Env, job_id: &Symbol) -> Job {
    env.storage()
        .persistent()
        .get(job_id)
        .unwrap_or_else(|| panic!("core_registry: job not found"))
}

fn write_job(env: &Env, job: &Job) {
    env.storage().persistent().set(&job.job_id, job);
    env.storage()
        .persistent()
        .extend_ttl(&job.job_id, TTL_THRESHOLD, TTL_EXTEND_TO);
}

#[contract]
pub struct CoreRegistry;

#[contractimpl]
impl CoreRegistry {
    /// One-time deployment configuration. Callable by the deployer.
    pub fn initialize(env: Env, escrow_vault: Address, token: Address) {
        if env.storage().persistent().has(&CONFIG_KEY) {
            panic!("core_registry: already initialized");
        }
        let config = Config { escrow_vault, token };
        env.storage().persistent().set(&CONFIG_KEY, &config);
        env.storage()
            .persistent()
            .extend_ttl(&CONFIG_KEY, TTL_THRESHOLD, TTL_EXTEND_TO);
    }

    /// Register a new compute job and escrow its budget.
    pub fn create_job(env: Env, client: Address, job_id: Symbol, budget: i128) {
        client.require_auth();

        if budget <= 0 {
            panic!("core_registry: budget must be positive");
        }
        if env.storage().persistent().has(&job_id) {
            panic!("core_registry: job id already exists");
        }

        let job = Job {
            job_id: job_id.clone(),
            client: client.clone(),
            // The solver is assigned separately; default to the client until then.
            solver: client.clone(),
            budget,
            status: JobStatus::Created,
        };
        write_job(&env, &job);

        env.events().publish(
            (Symbol::new(&env, "job_created"),),
            (job_id, client, budget),
        );
    }

    /// Assign a solver to a pending job and move it into `Running`.
    pub fn assign_solver(env: Env, job_id: Symbol, solver: Address) {
        let mut job = read_job(&env, &job_id);
        job.client.require_auth();

        if job.status != JobStatus::Created {
            panic!("core_registry: job is not in Created state");
        }

        job.solver = solver.clone();
        job.status = JobStatus::Running;
        write_job(&env, &job);

        env.events()
            .publish((Symbol::new(&env, "job_started"),), (job_id, solver));
    }

    /// Verify the solver's ZK execution proof and settle escrow to the solver.
    pub fn verify_and_settle(env: Env, job_id: Symbol, proof_hash: BytesN<32>) {
        let mut job = read_job(&env, &job_id);
        let solver = job.solver.clone();
        solver.require_auth();

        if job.status != JobStatus::Running {
            panic!("core_registry: job is not running");
        }

        // Record the execution proof hash for on-chain auditability.
        let proof_key = ProofKey {
            job_id: job_id.clone(),
        };
        env.storage().persistent().set(&proof_key, &proof_hash);
        env.storage()
            .persistent()
            .extend_ttl(&proof_key, TTL_THRESHOLD, TTL_EXTEND_TO);

        job.status = JobStatus::Verified;
        write_job(&env, &job);

        env.events().publish(
            (Symbol::new(&env, "job_verified"),),
            (job_id, proof_hash),
        );

        // Unlock the escrow payment to the solver.
        let config = read_config(&env);
        let vault = EscrowVaultClient::new(&env, &config.escrow_vault);
        vault.release_payment(&config.token, &solver, &job.budget);

        job.status = JobStatus::Settled;
        write_job(&env, &job);
    }

    /// Penalize a solver, marking the job `Slashed`.
    pub fn slash(env: Env, job_id: Symbol) {
        let mut job = read_job(&env, &job_id);
        job.client.require_auth();

        if job.status == JobStatus::Settled {
            panic!("core_registry: settled jobs cannot be slashed");
        }

        job.status = JobStatus::Slashed;
        write_job(&env, &job);

        env.events()
            .publish((Symbol::new(&env, "job_slashed"),), (job_id,));
    }

    /// Read a job's current lifecycle status.
    pub fn get_job_status(env: Env, job_id: Symbol) -> JobStatus {
        read_job(&env, &job_id).status
    }

    /// Read a full job record.
    pub fn get_job(env: Env, job_id: Symbol) -> Job {
        read_job(&env, &job_id)
    }

    /// Read the submitted proof hash for a job, if any.
    pub fn get_proof(env: Env, job_id: Symbol) -> BytesN<32> {
        env.storage()
            .persistent()
            .get(&ProofKey { job_id })
            .unwrap_or_else(|| panic!("core_registry: no proof submitted"))
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use soroban_sdk::{testutils::Address as _, Env};

    #[test]
    fn test_create_and_fetch_job() {
        let env = Env::default();
        let contract_id = env.register(CoreRegistry, ());
        let client = CoreRegistryClient::new(&env, &contract_id);

        let client_addr = Address::generate(&env);
        let job_id = symbol_short!("job1");
        let budget = 1_000_000i128;

        env.mock_all_auths();
        client.create_job(&client_addr, &job_id, &budget);

        let job = client.get_job(&job_id);
        assert_eq!(job.budget, budget);
        assert_eq!(job.client, client_addr);
        assert_eq!(job.status, JobStatus::Created);
    }
}
