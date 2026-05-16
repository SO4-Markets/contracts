//! Exchange router — single entry point for all user-facing protocol actions.
//! Mirrors GMX's ExchangeRouter.sol.
//!
//! Combines token transfers, vault interactions, and handler calls into
//! atomic multicall transactions. Users approve the router, then call
//! `multicall(Vec<RouterAction>)` with encoded instructions.
//!
//! Supported actions:
//!   SendTokens, CreateDeposit, CancelDeposit,
//!   CreateWithdrawal, CancelWithdrawal,
//!   CreateOrder, UpdateOrder, CancelOrder,
//!   ClaimFundingFees
#![no_std]
#![allow(dependency_on_unit_never_type_fallback)]

use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, panic_with_error,
    Address, BytesN, Env, Vec, token,
};
use gmx_types::{
    CreateDepositParams, CreateWithdrawalParams, CreateOrderParams,
};

// ─── Storage keys ─────────────────────────────────────────────────────────────

#[contracttype]
enum InstanceKey {
    Initialized,
    Admin,
    RoleStore,
    DataStore,
    DepositHandler,
    WithdrawalHandler,
    OrderHandler,
    FeeHandler,
}

// ─── Router-only param structs ────────────────────────────────────────────────
// These are user-facing types for actions that have no handler equivalent.

#[contracttype]
pub struct SendTokensParams {
    pub token:    Address,
    pub receiver: Address,
    pub amount:   i128,
}

#[contracttype]
pub struct UpdateOrderParams {
    pub key:               BytesN<32>,
    pub size_delta_usd:    i128,
    pub acceptable_price:  i128,
    pub trigger_price:     i128,
    pub min_output_amount: i128,
}

#[contracttype]
pub struct ClaimFundingFeesParams {
    pub markets: Vec<Address>,
    pub tokens:  Vec<Address>,
}

// ─── Multicall action discriminant ────────────────────────────────────────────

/// Each element in a multicall Vec is one action variant.
#[contracttype]
pub enum RouterAction {
    SendTokens(SendTokensParams),
    CreateDeposit(CreateDepositParams),
    CancelDeposit(BytesN<32>),
    CreateWithdrawal(CreateWithdrawalParams),
    CancelWithdrawal(BytesN<32>),
    CreateOrder(CreateOrderParams),
    UpdateOrder(UpdateOrderParams),
    CancelOrder(BytesN<32>),
    ClaimFundingFees(ClaimFundingFeesParams),
}

// ─── Errors ───────────────────────────────────────────────────────────────────

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Error {
    AlreadyInitialized = 1,
    NotInitialized     = 2,
    Unauthorized       = 3,
}

// ─── External handler clients ─────────────────────────────────────────────────
// Signatures must match the handler contract's public functions exactly.

#[allow(dead_code)]
#[soroban_sdk::contractclient(name = "DepositHandlerClient")]
trait IDepositHandler {
    fn create_deposit(env: Env, caller: Address, params: CreateDepositParams) -> BytesN<32>;
    fn cancel_deposit(env: Env, caller: Address, key: BytesN<32>);
}

#[allow(dead_code)]
#[soroban_sdk::contractclient(name = "WithdrawalHandlerClient")]
trait IWithdrawalHandler {
    fn create_withdrawal(env: Env, caller: Address, params: CreateWithdrawalParams) -> BytesN<32>;
    fn cancel_withdrawal(env: Env, caller: Address, key: BytesN<32>);
}

#[allow(dead_code)]
#[soroban_sdk::contractclient(name = "OrderHandlerClient")]
trait IOrderHandler {
    fn create_order(env: Env, caller: Address, params: CreateOrderParams) -> BytesN<32>;
    fn update_order(
        env: Env,
        caller: Address,
        key: BytesN<32>,
        size_delta_usd: i128,
        acceptable_price: i128,
        trigger_price: i128,
        min_output_amount: i128,
    );
    fn cancel_order(env: Env, caller: Address, key: BytesN<32>);
}

#[allow(dead_code)]
#[soroban_sdk::contractclient(name = "FeeHandlerClient")]
trait IFeeHandler {
    fn claim_funding_fees(env: Env, account: Address, market: Address, token: Address) -> u128;
}

// ─── Contract ─────────────────────────────────────────────────────────────────

#[contract]
pub struct ExchangeRouter;

#[contractimpl]
impl ExchangeRouter {
    /// One-time setup — store all handler addresses.
    #[allow(clippy::too_many_arguments)]
    pub fn initialize(
        env: Env,
        admin: Address,
        role_store: Address,
        data_store: Address,
        deposit_handler: Address,
        withdrawal_handler: Address,
        order_handler: Address,
        fee_handler: Address,
    ) {
        admin.require_auth();
        if env.storage().instance().has(&InstanceKey::Initialized) {
            panic_with_error!(&env, Error::AlreadyInitialized);
        }
        env.storage().instance().set(&InstanceKey::Initialized, &true);
        env.storage().instance().set(&InstanceKey::Admin, &admin);
        env.storage().instance().set(&InstanceKey::RoleStore, &role_store);
        env.storage().instance().set(&InstanceKey::DataStore, &data_store);
        env.storage().instance().set(&InstanceKey::DepositHandler, &deposit_handler);
        env.storage().instance().set(&InstanceKey::WithdrawalHandler, &withdrawal_handler);
        env.storage().instance().set(&InstanceKey::OrderHandler, &order_handler);
        env.storage().instance().set(&InstanceKey::FeeHandler, &fee_handler);
    }

    // ── Multicall ─────────────────────────────────────────────────────────────

    /// Execute a batch of actions atomically.
    ///
    /// Single caller.require_auth() covers all sub-actions (they run inside this invocation).
    /// Returns one BytesN<32> result per action (create_* returns a key; others return zero hash).
    /// If any action panics, the entire transaction reverts (Soroban atomicity).
    pub fn multicall(env: Env, caller: Address, actions: Vec<RouterAction>) -> Vec<BytesN<32>> {
        caller.require_auth();

        let mut results: Vec<BytesN<32>> = Vec::new(&env);
        let zero_key = BytesN::from_array(&env, &[0u8; 32]);

        let len = actions.len();
        let mut i = 0u32;
        while i < len {
            let action = actions.get(i).unwrap();
            match action {
                RouterAction::SendTokens(p) => {
                    token::Client::new(&env, &p.token)
                        .transfer(&caller, &p.receiver, &p.amount);
                    results.push_back(zero_key.clone());
                }
                RouterAction::CreateDeposit(p) => {
                    let key = Self::create_deposit(env.clone(), caller.clone(), p);
                    results.push_back(key);
                }
                RouterAction::CancelDeposit(key) => {
                    Self::cancel_deposit(env.clone(), caller.clone(), key);
                    results.push_back(zero_key.clone());
                }
                RouterAction::CreateWithdrawal(p) => {
                    let key = Self::create_withdrawal(env.clone(), caller.clone(), p);
                    results.push_back(key);
                }
                RouterAction::CancelWithdrawal(key) => {
                    Self::cancel_withdrawal(env.clone(), caller.clone(), key);
                    results.push_back(zero_key.clone());
                }
                RouterAction::CreateOrder(p) => {
                    let key = Self::create_order(env.clone(), caller.clone(), p);
                    results.push_back(key);
                }
                RouterAction::UpdateOrder(p) => {
                    Self::update_order(env.clone(), caller.clone(), p);
                    results.push_back(zero_key.clone());
                }
                RouterAction::CancelOrder(key) => {
                    Self::cancel_order(env.clone(), caller.clone(), key);
                    results.push_back(zero_key.clone());
                }
                RouterAction::ClaimFundingFees(p) => {
                    Self::claim_funding_fees(env.clone(), caller.clone(), p.markets, p.tokens);
                    results.push_back(zero_key.clone());
                }
            }
            i += 1;
        }

        results
    }

    // ── Individual action helpers ─────────────────────────────────────────────

    /// Transfer `amount` of `token` from caller to `receiver` (funds a vault).
    pub fn send_tokens(env: Env, caller: Address, token: Address, receiver: Address, amount: i128) {
        caller.require_auth();
        token::Client::new(&env, &token).transfer(&caller, &receiver, &amount);
    }

    /// Forward create_deposit to the deposit_handler.
    pub fn create_deposit(env: Env, caller: Address, params: CreateDepositParams) -> BytesN<32> {
        caller.require_auth();
        let deposit_handler: Address = env.storage().instance()
            .get(&InstanceKey::DepositHandler).unwrap();
        DepositHandlerClient::new(&env, &deposit_handler).create_deposit(&caller, &params)
    }

    /// Forward cancel_deposit to the deposit_handler.
    pub fn cancel_deposit(env: Env, caller: Address, key: BytesN<32>) {
        caller.require_auth();
        let deposit_handler: Address = env.storage().instance()
            .get(&InstanceKey::DepositHandler).unwrap();
        DepositHandlerClient::new(&env, &deposit_handler).cancel_deposit(&caller, &key);
    }

    /// Forward create_withdrawal to the withdrawal_handler.
    pub fn create_withdrawal(
        env: Env,
        caller: Address,
        params: CreateWithdrawalParams,
    ) -> BytesN<32> {
        caller.require_auth();
        let withdrawal_handler: Address = env.storage().instance()
            .get(&InstanceKey::WithdrawalHandler).unwrap();
        WithdrawalHandlerClient::new(&env, &withdrawal_handler)
            .create_withdrawal(&caller, &params)
    }

    /// Forward cancel_withdrawal to the withdrawal_handler.
    pub fn cancel_withdrawal(env: Env, caller: Address, key: BytesN<32>) {
        caller.require_auth();
        let withdrawal_handler: Address = env.storage().instance()
            .get(&InstanceKey::WithdrawalHandler).unwrap();
        WithdrawalHandlerClient::new(&env, &withdrawal_handler).cancel_withdrawal(&caller, &key);
    }

    /// Forward create_order to the order_handler.
    pub fn create_order(env: Env, caller: Address, params: CreateOrderParams) -> BytesN<32> {
        caller.require_auth();
        let order_handler: Address = env.storage().instance()
            .get(&InstanceKey::OrderHandler).unwrap();
        OrderHandlerClient::new(&env, &order_handler).create_order(&caller, &params)
    }

    /// Forward update_order to the order_handler.
    pub fn update_order(env: Env, caller: Address, params: UpdateOrderParams) {
        caller.require_auth();
        let order_handler: Address = env.storage().instance()
            .get(&InstanceKey::OrderHandler).unwrap();
        OrderHandlerClient::new(&env, &order_handler).update_order(
            &caller,
            &params.key,
            &params.size_delta_usd,
            &params.acceptable_price,
            &params.trigger_price,
            &params.min_output_amount,
        );
    }

    /// Forward cancel_order to the order_handler.
    pub fn cancel_order(env: Env, caller: Address, key: BytesN<32>) {
        caller.require_auth();
        let order_handler: Address = env.storage().instance()
            .get(&InstanceKey::OrderHandler).unwrap();
        OrderHandlerClient::new(&env, &order_handler).cancel_order(&caller, &key);
    }

    /// Claim earned funding fees across multiple markets in one call.
    pub fn claim_funding_fees(
        env: Env,
        caller: Address,
        markets: Vec<Address>,
        tokens: Vec<Address>,
    ) {
        caller.require_auth();
        let fee_handler: Address = env.storage().instance()
            .get(&InstanceKey::FeeHandler).unwrap();
        let fee_client = FeeHandlerClient::new(&env, &fee_handler);
        let len = markets.len();
        let mut i = 0u32;
        while i < len {
            fee_client.claim_funding_fees(
                &caller,
                &markets.get(i).unwrap(),
                &tokens.get(i).unwrap(),
            );
            i += 1;
        }
    }
}
