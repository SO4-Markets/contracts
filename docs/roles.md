# Role Reference

SO4.market uses a role-based access control system managed by the `role_store` contract. Every privileged operation checks role membership before proceeding; unauthorised callers panic with `Unauthorized`.

---

## Role Table

| Role constant | Key seed | Grantor | Typical holder | Permitted actions |
|---|---|---|---|---|
| `ROLE_ADMIN` | `"ROLE_ADMIN"` | Self (bootstrap) | Protocol multisig / deployer | Grant and revoke any role in `role_store` |
| `CONTROLLER` | `"CONTROLLER"` | `ROLE_ADMIN` | Core contracts (oracle, order_handler, etc.) | Write to `data_store` — prices, OI, pool amounts, flags |
| `MARKET_KEEPER` | `"MARKET_KEEPER"` | `ROLE_ADMIN` | Market creation bot | Create and configure new markets in `data_store` |
| `ORDER_KEEPER` | `"ORDER_KEEPER"` | `ROLE_ADMIN` | Execution keeper bots | Submit prices via `oracle.set_prices`; execute pending orders |
| `LIQUIDATION_KEEPER` | `"LIQUIDATION_KEEPER"` | `ROLE_ADMIN` | Liquidation bots | Call `liquidation_handler.liquidate_position` |
| `ADL_KEEPER` | `"ADL_KEEPER"` | `ROLE_ADMIN` | ADL bots | Call `adl_handler.execute_adl` |
| `FEE_KEEPER` | `"FEE_KEEPER"` | `ROLE_ADMIN` | Fee distribution account | Claim and distribute protocol fees |

> **Note on contract upgrades:** `ROLE_ADMIN` only gates `grant_role` and `revoke_role` inside `role_store`. Contract upgrades (`fn upgrade`) are **not** routed through `role_store` at all — each upgradeable contract stores its own admin address in its own instance storage and calls `admin.require_auth()` directly. Revoking or rotating a `role_store` role has no effect on any contract's upgrade authority.

> The `BytesN<32>` key stored in `role_store` is `sha256(role_name_utf8_bytes)`. Use `scripts/compute_key.py role <ROLE_NAME>` to derive the hex key for any role name.

---

## Key Derivation

Role keys are computed in `libs/keys/src/lib.rs` under the `roles` module:

```rust
pub fn order_keeper(env: &Env) -> BytesN<32> {
    env.crypto().sha256(&Bytes::from_slice(env, b"ORDER_KEEPER")).into()
}
```

Every role follows the same pattern: `sha256(UPPERCASE_ROLE_NAME_BYTES)`.

---

## Checking a Role

Any contract can verify role membership synchronously:

```rust
RoleStoreClient::new(env, &role_store).has_role(&account, &role_key)
// returns bool — true if account holds the role
```

---

## Granting a Role

Requires the caller to hold `ROLE_ADMIN`. Granting a role emits a `RoleGranted` event.

```bash
stellar contract invoke \
  --id   "$ROLE_STORE" \
  --source "$ADMIN_SOURCE" \
  --network testnet \
  -- grant_role \
  --caller  "$ADMIN_ADDRESS" \
  --account "$ACCOUNT_TO_GRANT" \
  --role    "$(python3 scripts/compute_key.py role ORDER_KEEPER)"
```

Replace `ORDER_KEEPER` with any role name from the table above.

---

## Revoking a Role

Requires the caller to hold `ROLE_ADMIN`. Revoking a role emits a `RoleRevoked` event.

```bash
stellar contract invoke \
  --id   "$ROLE_STORE" \
  --source "$ADMIN_SOURCE" \
  --network testnet \
  -- revoke_role \
  --caller  "$ADMIN_ADDRESS" \
  --account "$ACCOUNT_TO_REVOKE" \
  --role    "$(python3 scripts/compute_key.py role ORDER_KEEPER)"
```

---

## Role Events

`role_store` emits an event on every successful grant or revoke:

| Event topic | Payload | When |
|---|---|---|
| `RoleGranted` | `{ account: Address, role: BytesN<32> }` | After a successful `grant_role` call |
| `RoleRevoked` | `{ account: Address, role: BytesN<32> }` | After a successful `revoke_role` call |

---

## Bootstrap Sequence

On a fresh deployment, initialise roles in this order:

1. **Deploy `role_store`** — call `initialize(admin)`. The admin address can now grant all other roles.
2. **Deploy `data_store`** — call `initialize(admin, role_store)`.
3. **Deploy core contracts** — oracle, order_handler, liquidation_handler, adl_handler, etc.
4. **Grant `CONTROLLER`** to every state-changing handler that writes to `data_store`: `market_factory`, `deposit_handler`, `withdrawal_handler`, `order_handler`, `liquidation_handler`, `adl_handler`, `fee_handler`, `exchange_router`, and `oracle` (its circuit-breaker writes a market-pause flag). See `docs/deployment.md`'s "Grant `CONTROLLER`" section for the authoritative, up-to-date grant list — reference it rather than duplicating it here so the two docs can't drift out of sync again.
5. **Grant `ORDER_KEEPER`** to keeper bot accounts.
6. **Grant `LIQUIDATION_KEEPER`** and **`ADL_KEEPER`** to their respective bots.
7. **Grant `FEE_KEEPER`** to the fee distribution account.
8. **Optionally transfer `ROLE_ADMIN`** to a multisig after all roles are configured.

Keeping the `ROLE_ADMIN` key in a hardware wallet or multisig is strongly recommended — all other roles are granted through it.
