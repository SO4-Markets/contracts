 ```diff
--- /dev/null
+++ b/tests/lp_fee_distribution.rs
@@ -0,0 +1,1 @@
+use soroban_sdk::{testutils::Address as _, Address, Env, Symbol};
+use testutils::{
+    create_test_env, create_token_contract, deploy_exchange_router, deploy_pool,
+    mint_tokens, register_lp, setup_market, submit_deposit, submit_withdrawal,
+    open_and_close_position, get_pool_amounts, get_lp_token_supply, get_lp_token_balance,
+};
+
+mod testutils;
+
+/// Test LP fee accrual从和 proportional distribution across multiple depositors.
+/// Scenario:
+/// 1. Alice deposits 10,000 USDC, receives 10,000 GM tokens
+/// 2. Bob deposits 10,000 USDC, receives 10,000 GM tokens
+/// 3. Trader opens and closes a position, generating 200 USDC in fees
+/// 4. Alice withdraws all 10,000 GM tokens
+/// 5. Bob withdraws all 10,000 GM tokens
+#[test]
+fn test_lp_fee_distribution_equal_shares() {
+    let env = create_test_env();
+    let admin = Address::generate(&env);
+    let alice = Address::generate(&env);
+    let bob = Address::generate(&env);
+    let trader = Address::generate(&env);
+    
+    // Setup market and tokens
+    let (market, usdc) = setup_market(&env, &admin);
+    let pool = deploy_pool(&env, &market);
+    let router = deploy_exchange_router(&env, &market);
+    
+    // Mint USDC to participants
+    mint_tokens(&env, &usdc, &alice, 10_000);
+    mint_tokens(&env, &usdc, &bob, 10_000);
+    mint_tokens(&env, &usdc, &trader, 10_000);
+    
+    // Alice deposits 10,000 USDC
+    let alice_deposit = submit_deposit(&env, &router, &alice, &usdc, 10_000);
+    let alice_lp_balance = get_lp_token_balance(&env, &pool, &alice);
+    assert_eq!(alice_lp_balance, 10_000, "Alice should receive 10,000 LP tokens");
+    
+    // Bob deposits 10,000 USDC
+    let bob_deposit = submit_deposit(&env, &router, &bob, &usdc, 10_000);
+    let bob_lp_balance = get_lp_token_balance(&env, &pool, &bob);
+    assert_eq!(bob_lp_balance, 10_000, "Bob should receive 10,000 LP tokens");
+    
+    // Verify pool has 20,000 USDC
+    let pool_amounts = get_pool_amounts(&env, &pool);
+    assert_eq!(pool_amounts.usdc, 20_000, "Pool should have 20,000 USDC");
+    
+    // Trader opens and closes position, generating 200 USDC in fees
+    open_and_close_position(&env, &router, &trader, &usdc, 200);
+    
+    // Verify fees accrued in pool
+    let pool_amounts_after_fees = get_pool_amounts(&env, &pool);
+    assert_eq!(pool_amounts_after_fees.usdc, 20_200, "Pool should have 20,200 USDC after fees");
+    
+    // Alice withdraws all LP tokens
+    let alice_withdrawal = submit_withdrawal(&env, &router, &alice, &pool, 10_000);
+    let alice_usdc_after = usdc.balance(&alice);
+    assert!(
+        alice_usdc_after >= 10_099 && alice_usdc_after <= 10_101,
+        "Alice should receive ~10,100 USDC, got {}", alice_usdc_after
+    );
+    
+    // Bob withdraws all LP tokens
+    let bob_withdrawal = submit_withdrawal(&env, &router, &bob, &pool, 10_000);
+    let bob_usdc_after = usdc.balance(&bob);
+    assert!(
+        bob_usdc_after >= 10_099 && bob_usdc_after <= 10_101,
+        "Bob should receive ~10,100 USDC, got {}", bob_usdc_after
+    );
+    
+    // Verify total withdrawn
+    let total_withdrawn = alice_usdc_after + bob_usdc_after;
+    assert_eq!(total_withdrawn, 20_200, "Total withdrawn should be 20,200 USDC");
+    
+    // Verify LP token supply is 0
+    let lp_supply = get_lp_token_supply(&env, &pool);
+    assert_eq!(lp_supply, 0, "LP token supply should be 0");
+    
+    // Verify pool amounts are 0
+    let final_pool_amounts = get_pool_amounts(&env, &pool);
+    assert_eq!(final_pool_amounts.usdc, 0, "Pool USDC should be 0");
+}
+
+/// Test LP fee distribution when depositors join at different times (unequal shares).
+/// Scenario:
+/// 1. Alice deposits 10,000 USDC, receives 10,000 GM tokens
+/// 2. Trader opens and closes a position, generating 200 USDC in fees
+/// 3. Bob deposits 10,000 USDC at higher price per share, receives fewer GM tokens
+/// 4. Alice withdraws all GM tokens
+/// 5. Bob withdraws all GM tokens
+#[test]
+fn test_lp_fee_distribution_unequal_shares() {
+    let env = create_test_env();
+    let admin = Address::generate(&env);
+    let alice = Address::generate(&env);
+    let bob = Address::generate(&env);
+    let trader = Address::generate(&env);
+    
+    // Setup market and tokens
+    let (market, usdc) = setup_market(&env, &admin);
+    let pool = deploy_pool(&env, &market);
+    let router = deploy_exchange_router(&env, &market);
+    
+    // Mint USDC to participants
+    mint_tokens(&env, &usdc, &alice, 10_000);
+    mint_tokens(&env, &usdc, &bob, 10_000);
+    mint_tokens(&env, &usdc, &trader, 10_000);
+    
+    // Alice deposits 10,000 USDC
+    let alice_deposit = submit_deposit(&env, &router, &alice, &usdc, 10_000);
+    let alice_lp_balance = get_lp_token_balance(&env,