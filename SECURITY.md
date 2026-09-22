# Security Policy

SO4.market is a decentralized perpetuals and spot exchange built on [Stellar](https://stellar.org) using [Soroban](https://soroban.stellar.org) smart contracts (Rust SDK). Because the protocol custodies real user deposits, margin collateral, and liquidity pool assets, security is of paramount importance.

We deeply value the contributions of independent security researchers and white-hat hackers who help keep the protocol and its users safe.

---

## 1. Reporting a Vulnerability

> [!IMPORTANT]
> **DO NOT report security vulnerabilities through public GitHub issues, pull requests, discussions, or social media channels.**
> Publicly disclosing an unpatched vulnerability exposes user funds to active exploitation.

If you believe you have discovered a security vulnerability in SO4.market contracts, please report it through one of our private, encrypted channels:

### Primary Channel: GitHub Private Vulnerability Reporting
Submit an advisory directly through GitHub Security Advisories:
👉 **[Report a Vulnerability via GHSA](https://github.com/SO4-Markets/contracts/security/advisories/new)**

### Alternative Channel: Encrypted Security Email
Send an encrypted or plain email to our security team:
📧 **`security@so4.market`**  
*(CC maintainers: `145639665+abayomicornelius@users.noreply.github.com`, `sunday.m1701072@st.futminna.edu.ng`)*

### Information to Include in Your Report
To accelerate validation and triage, please include:
1. **Summary & Impact**: Clear description of the vulnerability and its potential consequences (e.g. fund loss, unauthorized state manipulation, oracle manipulation, frozen funds).
2. **Affected Components**: Specific contract name(s), source file paths, and function names.
3. **Commit Hash / Branch**: The commit hash or release tag on which the vulnerability was verified.
4. **Step-by-Step Proof of Concept (PoC)**: Minimal Soroban Rust test case (`#[test]`) or reproduction script demonstrating the exploit vector.
5. **Mitigation / Suggested Patch**: Proposed code fix or architectural remedy, if available.
6. **Payout Details**: Your Stellar public key (`G...`) or contact handle for bounty attribution and reward distribution.

---

## 2. Scope & Target Assets

### In Scope
The following smart contracts and libraries located within this repository on the `main` branch (and deployed testnet/mainnet instances) are in scope:

| Component | Path | Description |
|---|---|---|
| **Exchange Router** | `contracts/exchange_router/` | Core entrypoint for trader interactions, orders, and circuit breaker |
| **Order Handler** | `contracts/order_handler/` | Limit, market, and trigger order processing and fill validation |
| **Order Vault** | `contracts/order_vault/` | Custody of escrowed trader deposits and collateral assets |
| **Deposit Handler** | `contracts/deposit_handler/` | Liquidity pool deposit routing and minting calculations |
| **Deposit Vault** | `contracts/deposit_vault/` | Custodial holding of incoming liquidity pool assets |
| **Withdrawal Handler** | `contracts/withdrawal_handler/` | Liquidity redemption, share burning, and asset distribution |
| **Withdrawal Vault** | `contracts/withdrawal_vault/` | Custodial settlement of withdrawal claims |
| **Liquidation Handler** | `contracts/liquidation_handler/` | Under-collateralized position detection and liquidation enforcement |
| **ADL Handler** | `contracts/adl_handler/` | Auto-deleveraging execution during severe market displacement |
| **Oracle** | `contracts/oracle/` | Ed25519 signed price verification, median aggregation, staleness gates |
| **Data Store** | `contracts/data_store/` | Central key-value protocol parameter and ledger state persistence |
| **Role Store** | `contracts/role_store/` | Role-based access control (RBAC), admin, and keeper authorization |
| **Insurance Fund Router**| `contracts/insurance_fund_router/`| Deficit coverage routing and bad-debt backstop mechanics |
| **Market Factory** | `contracts/market_factory/` | Market deployment, index/long/short token configuration |
| **Market Token** | `contracts/market_token/` | SEP-41 / ERC-20 compliant LP token representation |
| **Fee Handler & Sweeper**| `contracts/fee_handler/`, `contracts/fee_batch_sweeper/` | Protocol and execution fee collection and distribution |
| **Referral Storage** | `contracts/referral_storage/` | Trader referral codes, tiers, and rebate tracking |
| **Shared Libraries** | `libs/keys/`, `libs/math/` | Storage key hashing, fixed-point math, and PnL derivation |

### Out of Scope
The following areas are strictly out of scope:
- **Test Contracts & Fixtures**: `contracts/test_faucet/`, `contracts/test_token/`, and mock bindings.
- **Third-Party Upstream Dependencies**: Bugs in the Stellar Core protocol, Soroban Host environment, Rust standard library, or external dependencies unless directly caused by protocol misconfiguration.
- **Denial of Service (DoS)**: Resource exhaustion or network spam that does not permanently compromise protocol state or lock funds.
- **Social Engineering & Infrastructure**: Attacks against team members, phishing, DNS hijacking, or infrastructure hosting.
- **Previously Known Issues**: Vulnerabilities already reported, actively tracked in open issues, or disclosed in previous audit reports.

---

## 3. Response SLAs & Vulnerability Lifecycle

We adhere to a predictable timeline for acknowledging and remediating security disclosures:

| Stage | Target SLA | Description |
|---|---|---|
| **Acknowledgement** | **< 24 Hours** | We confirm receipt of your report and assign a primary security reviewer. |
| **Initial Triage** | **< 72 Hours** | We validate the PoC, reproduce the issue, and confirm the CVSS / severity rating. |
| **Status Updates** | **Every 3 Days** | Ongoing updates regarding root-cause analysis, patch progress, and testing. |
| **Fix Deployment** | **1 – 14 Days** | Patch deployed to testnet, verified, and upgraded on active networks. |
| **Public Disclosure** | **Coordinated** | Coordinated disclosure date mutually agreed upon after fix verification. |

---

## 4. Bug Bounty & Reward Program

SO4.market actively supports open-source security contributors through community incentives and the **GrantFox OSS Campaign**:

- **Reward Eligibility**: Submissions must present an original, previously unknown vulnerability within the defined scope, accompanied by an actionable PoC.
- **Severity Classification**:
  - 🔴 **Critical**: Direct loss of user or protocol funds, unauthorized arbitrary minting, complete insolvency.
  - 🟠 **High**: Temporary freezing of user funds, oracle manipulation under realistic conditions, unauthorized state alterations.
  - 🟡 **Medium**: Logic errors resulting in fee leakage, griefing without direct profit, inconsistent accounting across edge cases.
  - 🔵 **Low / Informational**: Non-exploitable validation edge cases, documentation gaps affecting security posture.
- **Payout Settlement**:
  - Rewards are distributed in **USDC** or **XLM** directly on Stellar/Soroban or via GrantFox escrow.
  - Please ensure your report includes your valid Stellar public address (`G...`).

---

## 5. Safe Harbor Policy

We consider security research conducted under this policy to be authorized. We will **not** initiate legal action or request law enforcement investigation against researchers who:
1. Make a good faith effort to avoid privacy violations, data destruction, and interruption or degradation of the protocol.
2. Only interact with their own accounts or test accounts during research; never access or compromise real user funds or personal information.
3. Keep details of the vulnerability confidential until a fix has been deployed and reasonable time has passed for users/nodes to update.
4. Promptly report any discovered vulnerability according to this policy.

If at any point you are uncertain whether your research complies with this policy, please reach out via `security@so4.market` before proceeding.
