 ```diff
--- /dev/null
+++ b/tests/lp_fee_distribution.rs
@@ -0,0 +1,370 @@
+use soroban_sdk::{
+    testutils::{Address as _, Ledger},
+    Address, Env, Symbol,
+};
+
+// Import contract crates
+use exchange_router::ExchangeRouterClient;
+use data_store::DataStoreClient;
+use market_token::MarketTokenClient;
+use pool::PoolClient;
+
+mod utils;
+use utils::{
+    create_env, deploy_contracts, setup_market, create_user_with_usdc,
+    deposit_lp, withdraw_lp, open_and_close_position_with_fees,
+    get_pool_amounts, get_gm_token_supply, get_user_gm_balance,
+};
+
+/// Test: LP fee accrual and proportional distribution across multiple depositors
+/// 
+/// Scenario:
+/// 1. LP Alice deposits 10,000 USDC, receives 10,000 GM tokens
+/// 2. LP Bob deposits 10,000 USDC, receives 10,000 GM tokens (same price, equal shares)
+/// 3. Trader opens and closes a position, generating 200 USDC in fees
+/// 4. Alice withdraws all 10,000 GM tokens
+/// 5. Bob withdraws all 10,000 GM tokens
+/// 
+/// Expected:
+/// - Alice receives ≈10,100 USDC (10,000 principal + 100 fee share)
+/// - Bob receives ≈10,100 USDC
+/// - Total withdrawn = 20,200 USDC (20,000 principal + 200 fees)
+/// - GM token supply = 0 after both withdrawals
+/// - Pool amounts = 0
+#[test]
+fn test_lp_fee_distribution_equal_shares() {
+    let env = create_env();
+    let (router, data_store, pool, market_token, usdc) = deploy_contracts(&env);
+    
+    // Setup market with USDC as collateral
+    let market = setup_market(&env, &usdc);
+    
+    // Create users
+    let alice = create_user_with_usdc(&env, &usdc, 10_000_000_000); // 10,000 USDC
+    let bob = create_user_with_usdc(&env, &usdc, 10_000_000_000);   // 10,000 USDC
+    let trader = create_user_with_usdc(&env, &usdc, 5_000_000_000); // For trading
+    
+    // Step 1: Alice deposits 10,000 USDC
+    let alice_deposit = 10_000_000_000i128; // 10,000 USDC with 6 decimals
+    let alice_gm_received = deposit_lp(&env, &router, &alice, &market, &usdc, alice_deposit);
+    
+    // Verify Alice received GM tokens (should be ~10,000 for first depositor)
+    assert_eq!(alice_gm_received, 10_000_000_000, "Alice should receive 10,000 GM tokens");
+    assert_eq!(get_gm_token_supply(&env, &market_token), 10_000_000_000, "GM supply should be 10,000");
+    
+    // Step 2: Bob deposits 10,000 USDC
+    let bob_deposit = 10_000_000_000i128;
+    let bob_gm_received = deposit_lp(&env, &router, &bob, &market, &usdc, bob_deposit);
+    
+    // Verify Bob received equal GM tokens (same price)
+    assert_eq!(bob_gm_received, 10_000_000_000, "Bob should receive 10,000 GM tokens");
+    assert_eq!(get_gm_token_supply(&env, &market_token), 20_000_000_000, "GM supply should be 20,000");
+    
+    // Verify pool has 20,000 USDC
+    let pool_amounts = get_pool_amounts(&env, &pool, &market);
+    assert_eq!(pool_amounts, 20_000_000_000, "Pool should have 20,000 USDC");
+    
+    // Step 3: Trader opens and closes position, generating 200 USDC in fees
+    let fees_generated = open_and_close_position_with_fees(&env, &router, &trader, &market, &usdc, 200_000_000);
+    
+    // Verify fees accumulated in pool
+    let pool_after_fees = get_pool_amounts(&env, &pool, &market);
+    assert_eq!(pool_after_fees, 20_200_000_000, "Pool should have 20,200 USDC after fees");
+    
+    // Step 4: Alice withdraws all GM tokens
+    let alice_withdrawal = withdraw_lp(&env, &router, &alice, &market, &usdc, alice_gm_received);
+    
+    // Verify Alice received ~10,100 USDC (with small rounding tolerance)
+    let expected_alice = 10_100_000_000i128;
+    let tolerance = 1i128;
+    assert!(
+        (alice_withdrawal - expected_alice).abs() <= tolerance,
+        "Alice should receive ~10,100 USDC, got {}",
+        alice_withdrawal
+    );
+    
+    // Step 5: Bob withdraws all GM tokens
+    let bob_withdrawal = withdraw_lp(&env, &router, &bob, &market, &usdc, bob_gm_received);
+    
+    // Verify Bob received ~10,100 USDC
+    let expected_bob = 10_100_000_000i128;
+    assert!(
+        (bob_withdrawal - expected_bob).abs() <= tolerance,
+        "Bob should receive ~10,100 USDC, got {}",
+        bob_withdrawal
+    );
+    
+    // Verify total withdrawn equals 20,200 USDC
+    let total_withdrawn = alice_withdrawal + bob_withdrawal;
+    assert_eq!(total_withdrawn, 20_200_000_000, "Total withdrawn should be 20,200 USDC");
+    
+    // Verify GM token supply is 0
+    assert_eq!(get_gm_token_supply(&env, &market_token), 0, "GM supply should be 0");
+    
+    // Verify pool is empty
+    let final_pool = get_pool_amounts(&env, &pool, &market);
+    assert_eq!(final_pool, 0, "Pool should be empty");
+}
+
+/// Test: LP fee distribution with unequal shares
+/// 
+/// Scenario