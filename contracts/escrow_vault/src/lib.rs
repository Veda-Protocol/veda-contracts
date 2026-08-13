#![no_std]

//! # Escrow Vault
//!
//! Custody for job bounties. Clients wrap their classic asset (XLM/USDC via a
//! Stellar Asset Contract) into the vault with `deposit_and_lock`, and the
//! `CoreRegistry` releases those funds to Solvers with `release_payment`.

use soroban_sdk::token::{self, StellarAssetClient};
use soroban_sdk::{contract, contractimpl, contracttype, symbol_short, Address, Env, Symbol};

/// Per-owner locked balance key.
#[contracttype]
#[derive(Clone)]
pub struct LockKey {
    pub token: Address,
    pub owner: Address,
}

const REGISTRY_KEY: Symbol = symbol_short!("registry");
const TTL_THRESHOLD: u32 = 100;
const TTL_EXTEND_TO: u32 = 10_000;

fn read_registry(env: &Env) -> Address {
    env.storage()
        .persistent()
        .get(&REGISTRY_KEY)
        .unwrap_or_else(|| panic!("escrow_vault: not initialized"))
}

fn read_locked(env: &Env, key: &LockKey) -> i128 {
    env.storage().persistent().get(key).unwrap_or(0)
}

#[contract]
pub struct EscrowVault;

#[contractimpl]
impl EscrowVault {
    /// One-time deployment configuration. `registry` is the only contract
    /// authorized to release funds.
    pub fn initialize(env: Env, registry: Address) {
        if env.storage().persistent().has(&REGISTRY_KEY) {
            panic!("escrow_vault: already initialized");
        }
        env.storage().persistent().set(&REGISTRY_KEY, &registry);
        env.storage()
            .persistent()
            .extend_ttl(&REGISTRY_KEY, TTL_THRESHOLD, TTL_EXTEND_TO);
    }

    /// Wrap `from`'s classic asset into the vault via the Stellar Asset
    /// Contract and record the locked amount against `from`.
    pub fn deposit_and_lock(env: Env, token: Address, from: Address, amount: i128) -> i128 {
        from.require_auth();

        if amount <= 0 {
            panic!("escrow_vault: amount must be positive");
        }

        // Wrap the owner's classic asset into the vault's SAC balance.
        let sac = StellarAssetClient::new(&env, &token);
        sac.deposit(&from, &amount);

        let key = LockKey {
            token: token.clone(),
            owner: from.clone(),
        };
        let mut locked = read_locked(&env, &key);
        locked += amount;
        env.storage().persistent().set(&key, &locked);
        env.storage()
            .persistent()
            .extend_ttl(&key, TTL_THRESHOLD, TTL_EXTEND_TO);

        env.events()
            .publish((symbol_short!("escrow_locked"),), (token, from, amount));

        locked
    }

    /// Release wrapped funds to `to`. Callable only by the core registry.
    pub fn release_payment(env: Env, token: Address, to: Address, amount: i128) {
        let registry = read_registry(&env);
        registry.require_auth();

        if amount <= 0 {
            panic!("escrow_vault: amount must be positive");
        }

        // Move wrapped asset out of the vault to the recipient.
        let token_client = token::Client::new(&env, &token);
        token_client.transfer(&env.current_contract_address(), &to, &amount);

        env.events()
            .publish((symbol_short!("escrow_released"),), (token, to, amount));
    }

    /// Query a given owner's locked balance for a token.
    pub fn locked_balance(env: Env, token: Address, owner: Address) -> i128 {
        read_locked(&env, &LockKey { token, owner })
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use soroban_sdk::{testutils::Address as _, Env};

    #[test]
    fn test_initialize_and_query_balance() {
        let env = Env::default();
        let contract_id = env.register(EscrowVault, ());
        let client = EscrowVaultClient::new(&env, &contract_id);

        let registry = Address::generate(&env);
        client.initialize(&registry);

        let token = Address::generate(&env);
        let owner = Address::generate(&env);
        assert_eq!(client.locked_balance(&token, &owner), 0i128);
    }
}
