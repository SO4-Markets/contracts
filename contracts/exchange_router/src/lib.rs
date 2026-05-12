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
    contract, contracterror, contractimpl, contracttype, Address, BytesN, Env, Vec, token,
};
use gmx_types::OrderType;

// ─── Storage keys ─────────────────────────────────────────────────────────────

const ADMIN_KEY:              &str = "ADMIN";
const ROLE_STORE_KEY:         &str = "ROLE_STORE";
const DATA_STORE_KEY:         &str = "DATA_STORE";
const DEPOSIT_HANDLER_KEY:    &str = "DEP_HANDLER";
const WITHDRAWAL_HANDLER_KEY: &str = "WD_HANDLER";
const ORDER_HANDLER_KEY:      &str = "ORD_HANDLER";
const FEE_HANDLER_KEY:        &str = "FEE_HANDLER";

// ─── Per-action param structs ─────────────────────────────────────────────────
// Soroban #[contracttype] enums do not support named fields — each variant
// wraps a dedicated struct instead.

#[contracttype]
pub struct SendTokensParams {
    pub token:    Address,
    pub receiver: Address,
    pub amount:   i128,
}

#[contracttype]
pub struct CreateDepositParams {
    pub market:             Address,
    pub receiver:           Address,
    pub long_token_amount:  i128,
    pub short_token_amount: i128,
    pub min_market_tokens:  i128,
    pub execution_fee:      i128,
}

#[contracttype]
pub struct CreateWithdrawalParams {
    pub market:                 Address,
    pub receiver:               Address,
    pub market_token_amount:    i128,
    pub min_long_token_amount:  i128,
    pub min_short_token_amount: i128,
    pub execution_fee:          i128,
}

#[contracttype]
pub struct CreateOrderParams {
    pub market:                   Address,
    pub receiver:                 Address,
    pub initial_collateral_token: Address,
    pub swap_path:                Vec<Address>,
    pub size_delta_usd:           i128,
    pub collateral_delta_amount:  i128,
    pub trigger_price:            i128,
    pub acceptable_price:         i128,
    pub execution_fee:            i128,
    pub min_output_amount:        i128,
    pub order_type:               OrderType,
    pub is_long:                  bool,
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
pub enum Error {
    AlreadyInitialized = 1,
    NotInitialized     = 2,
    Unauthorized       = 3,
}

// ─── External handler clients ─────────────────────────────────────────────────

#[soroban_sdk::contractclient(name = "DepositHandlerClient")]
trait IDepositHandler {
    fn create_deposit(env: Env, caller: Address, params: gmx_types::DepositProps) -> BytesN<32>;
    fn cancel_deposit(env: Env, caller: Address, key: BytesN<32>);
}

#[soroban_sdk::contractclient(name = "WithdrawalHandlerClient")]
trait IWithdrawalHandler {
    fn create_withdrawal(env: Env, caller: Address, params: gmx_types::WithdrawalProps) -> BytesN<32>;
    fn cancel_withdrawal(env: Env, caller: Address, key: BytesN<32>);
}

#[soroban_sdk::contractclient(name = "OrderHandlerClient")]
trait IOrderHandler {
    fn create_order(env: Env, caller: Address, params: gmx_types::OrderProps) -> BytesN<32>;
    fn update_order(env: Env, caller: Address, key: BytesN<32>, size_delta_usd: i128,
                    acceptable_price: i128, trigger_price: i128, min_output_amount: i128);
    fn cancel_order(env: Env, caller: Address, key: BytesN<32>);
}

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
        // TODO: panic if already initialized (ADMIN_KEY exists in instance storage)
        //       Store all seven addresses in instance storage under their respective keys
        todo!()
    }

    // ── Multicall ─────────────────────────────────────────────────────────────

    /// Execute a batch of actions atomically.
    ///
    /// Single caller.require_auth() covers all sub-actions (they run inside this invocation).
    /// Returns one BytesN<32> result per action (create_* returns a key; others return zero hash).
    /// If any action panics, the entire transaction reverts (Soroban atomicity).
    pub fn multicall(env: Env, caller: Address, actions: Vec<RouterAction>) -> Vec<BytesN<32>> {
        // TODO: (mirrors GMX ExchangeRouter.multicall)
        //
        // 1. caller.require_auth()
        //
        // 2. let mut results: Vec<BytesN<32>> = Vec::new(&env);
        //    let zero_key = BytesN::from_array(&env, &[0u8; 32]);
        //
        // 3. for action in actions.iter() {
        //        match action {
        //            RouterAction::SendTokens(p) =>
        //                token::Client::new(&env, &p.token)
        //                    .transfer(&caller, &p.receiver, &p.amount);
        //                results.push_back(zero_key.clone());
        //
        //            RouterAction::CreateDeposit(p) =>
        //                let key = Self::create_deposit(env, caller.clone(), p);
        //                results.push_back(key);
        //
        //            RouterAction::CancelDeposit(key) =>
        //                Self::cancel_deposit(env, caller.clone(), key);
        //                results.push_back(zero_key.clone());
        //
        //            RouterAction::CreateWithdrawal(p) => ...
        //            RouterAction::CancelWithdrawal(key) => ...
        //            RouterAction::CreateOrder(p) => ...
        //            RouterAction::UpdateOrder(p) => ...
        //            RouterAction::CancelOrder(key) => ...
        //            RouterAction::ClaimFundingFees(p) =>
        //                Self::claim_funding_fees(env, caller.clone(), p.markets, p.tokens);
        //                results.push_back(zero_key.clone());
        //        }
        //    }
        //
        // 4. Return results
        todo!()
    }

    // ── Individual action helpers ─────────────────────────────────────────────

    /// Transfer `amount` of `token` from caller to `receiver` (funds a vault).
    pub fn send_tokens(env: Env, caller: Address, token: Address, receiver: Address, amount: i128) {
        // TODO:
        // 1. caller.require_auth()
        // 2. token::Client::new(&env, &token).transfer(&caller, &receiver, &amount)
        todo!()
    }

    /// Forward create_deposit to the deposit_handler.
    pub fn create_deposit(env: Env, caller: Address, params: CreateDepositParams) -> BytesN<32> {
        // TODO:
        // 1. caller.require_auth()
        // 2. Load deposit_handler address from instance storage
        // 3. Build DepositProps and call:
        //    DepositHandlerClient::new(&env, &deposit_handler).create_deposit(&caller, &props)
        // Returns the deposit key
        todo!()
    }

    /// Forward cancel_deposit to the deposit_handler.
    pub fn cancel_deposit(env: Env, caller: Address, key: BytesN<32>) {
        // TODO:
        // 1. caller.require_auth()
        // 2. DepositHandlerClient::new(&env, &deposit_handler).cancel_deposit(&caller, &key)
        todo!()
    }

    /// Forward create_withdrawal to the withdrawal_handler.
    pub fn create_withdrawal(env: Env, caller: Address, params: CreateWithdrawalParams) -> BytesN<32> {
        // TODO:
        // 1. caller.require_auth()
        // 2. Build WithdrawalProps and forward to withdrawal_handler
        // Returns the withdrawal key
        todo!()
    }

    /// Forward cancel_withdrawal to the withdrawal_handler.
    pub fn cancel_withdrawal(env: Env, caller: Address, key: BytesN<32>) {
        // TODO:
        // 1. caller.require_auth()
        // 2. WithdrawalHandlerClient::new(&env, &withdrawal_handler).cancel_withdrawal(&caller, &key)
        todo!()
    }

    /// Forward create_order to the order_handler.
    pub fn create_order(env: Env, caller: Address, params: CreateOrderParams) -> BytesN<32> {
        // TODO:
        // 1. caller.require_auth()
        // 2. Build OrderProps from params and forward to order_handler
        // Returns the order key
        todo!()
    }

    /// Forward update_order to the order_handler.
    pub fn update_order(env: Env, caller: Address, params: UpdateOrderParams) {
        // TODO:
        // 1. caller.require_auth()
        // 2. OrderHandlerClient::new(&env, &order_handler)
        //        .update_order(&caller, &params.key, params.size_delta_usd,
        //                      params.acceptable_price, params.trigger_price,
        //                      params.min_output_amount)
        todo!()
    }

    /// Forward cancel_order to the order_handler.
    pub fn cancel_order(env: Env, caller: Address, key: BytesN<32>) {
        // TODO:
        // 1. caller.require_auth()
        // 2. OrderHandlerClient::new(&env, &order_handler).cancel_order(&caller, &key)
        todo!()
    }

    /// Claim earned funding fees across multiple markets in one call.
    pub fn claim_funding_fees(env: Env, caller: Address, markets: Vec<Address>, tokens: Vec<Address>) {
        // TODO:
        // 1. caller.require_auth()
        // 2. Load fee_handler from instance storage
        // 3. Iterate: for i in 0..markets.len() {
        //        FeeHandlerClient::new(&env, &fee_handler)
        //            .claim_funding_fees(&caller, &markets.get(i).unwrap(),
        //                                &tokens.get(i).unwrap());
        //    }
        todo!()
    }
}
