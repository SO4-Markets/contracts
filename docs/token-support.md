# Supported Token Behavior and Custody Invariants (Issue #914, CLOB-038)

## Overview

The SO4.market Central Limit Order Book (CLOB) operates on a **fully-collateralized-offer principle**: any accepted resting order must be backed 100% by already-deposited, reserved collateral in the protocol vaults.

To maintain deterministic accounting and prevent insolvency, the protocol defines explicit requirements for supported tokens and rejects unsafe token assumptions. This document details supported token behavior, policies on clawback/freeze capabilities, and the mathematical custody invariants enforced across all operations.

---

## 1. Supported Token Standards

The exchange supports tokens operating under Soroban and Stellar standard asset interfaces:

| Property | Requirement | Rationale |
|---|---|---|
| **Interface** | Stellar Asset Contract (SAC) or SEP-41 compliant smart contract | Ensures standard `balance`, `transfer`, `transfer_from`, and `authorized` methods. |
| **Precision** | Standard 7 decimal places (Stellar stroops) or explicitly configured market decimals | Matches Stellar native precision and deterministic integer tick/lot conversions. |
| **Transfer Semantics** | Exact linear transfer: `amount_debited == amount_credited` | Any difference causes immediate reconciliation failure. |
| **Balance Predictability** | Static balances unless transferred | Balances must not change without an explicit transfer event. |

---

## 2. Unsafe Token Behaviors (Explicitly Rejected)

The following token models violate exchange custody assumptions and are rejected at the market listing boundary or detected and reverted at runtime:

1. **Fee-on-Transfer / Deflationary Tokens**:
   - Tokens where transfer burns or deducts a tax fee deliver less than the requested amount to the recipient (`amount_received < amount_sent`).
   - *Exchange Protection*: The deposit and transfer handlers verify balance deltas before and after every token movement. If `balance_after - balance_before < expected_amount`, the transaction reverts immediately.

2. **Rebasing / Elastic Supply Tokens**:
   - Tokens where user balances adjust algorithmically based on an external index or price feed.
   - *Exchange Protection*: Rebasing breaks the immutable custody ledger where `available + locked + protocol_fees == token_custody`. Rebasing assets cannot be listed in CLOB markets.

3. **Re-entrant Callback Tokens**:
   - Tokens that execute arbitrary recipient callbacks (e.g. ERC-777 style `tokensReceived`) during `transfer` or `transfer_from`.
   - *Exchange Protection*: Standard SEP-41 and SAC tokens do not invoke recipient hooks. External tokens with callback capabilities are rejected.

4. **Clawback-Capable and Freezable Assets**:
   - Stellar assets issued with `AUTH_CLAWBACK_ENABLED_FLAG` or `AUTH_REVOCABLE_FLAG` permit the asset issuer to claw back or freeze balances held in smart contract vaults.
   - *Exchange Policy*: See Section 3 below.

---

## 3. Clawback and Freeze Risk Policy

Stellar native classic assets wrapped via SAC retain their Stellar issuer flags:
- `AUTH_REVOCABLE_FLAG`: Issuer can freeze trustlines.
- `AUTH_CLAWBACK_ENABLED_FLAG`: Issuer can unilaterally claw back funds from any holder, including contract vaults.

### Protocol Policy

1. **Tier 1 (Core Permissionless Markets)**:
   - Must use assets with clawback **permanently disabled** (`AUTH_CLAWBACK_ENABLED_FLAG = 0`).
   - Native XLM and established non-clawback stablecoins.
   - Any market listing where base or quote token is clawback-capable is rejected by default.

2. **Tier 2 (Regulated / Issuer-Administered Markets)**:
   - For institutionally issued real-world assets (RWA) or regulated fiat tokens requiring clawback by law:
     - The market must be explicitly flagged with `allows_clawback = true` during market creation.
     - Keepers and UI must surface the "Issuer Clawback Enabled" warning badge to traders.
     - Custody is segregated: losses from an issuer clawback do not affect other markets or pairs.
     - Pre- and post-operation invariant assertions detect unexpected balance drops and freeze trading to protect unaffected traders.

---

## 4. Market Listing Checklist

Before a token pair is accepted into the CLOB market registry, the market factory and governance verify the following 8 criteria:

- [ ] **1. SEP-41 / SAC Conformance**: Token contract implements canonical Soroban token interface.
- [ ] **2. Non-Rebasing Verification**: Token contract has no supply rebase functions or dynamic balance multiplier.
- [ ] **3. Exact Transfer Check**: Test transfer verifies `balance_delta == transferred_amount` (0 fee on transfer).
- [ ] **4. Decimals Validation**: Decimals match the configured market lot and tick granularity.
- [ ] **5. Clawback Status**: Issuer flags inspected; clawback-disabled status confirmed for Tier 1 markets.
- [ ] **6. Contract Code Audit**: WASM hash is verified against known safe templates or audited implementations.
- [ ] **7. Liquidity and Custody Segregation**: Token vault address is verified and isolated from unrelated markets.
- [ ] **8. Dust & Min Notional Threshold**: Lot size and tick size prevent dust order spam and calculation underflow.

---

## 5. Custody Accounting Invariants

Every operation involving funds (deposits, order creation, cancellations, fills, fees, and withdrawals) maintains the strict accounting conservation law:

$$\sum_{i} \text{available}_i + \sum_{i} \text{locked}_i + \text{protocol\_fees} = \text{Actual Vault Balance}$$

### Invariant Rules:
1. **Never Credit Failed Transfers**:
   If a token `transfer_from` fails or traps, the transaction aborts atomically. No internal ledger credit or order state is written.
2. **Never Lock Unbacked Amounts**:
   An order can never lock more than the trader's spendable `available` balance:
   $$\text{order\_collateral} \le \text{available}$$
3. **Conservative Rounding**:
   Fee calculations round in protocol favor (maker fees round down, taker fees round up, proceeds round down). Value is never created from nothing.
