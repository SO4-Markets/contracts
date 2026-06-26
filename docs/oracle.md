# Oracle Price Verification

The oracle contract (`contracts/oracle`) is the only source of truth for asset prices in SO4.market. Keepers submit ed25519-signed price bundles before each keeper execution; handlers read those prices via `get_primary_price(token)`.

---

## 1. Why Ledger-Scoped Prices?

Each submitted price is valid for **exactly one ledger sequence window** (at most 60 ledgers, ~5 minutes). The ledger sequence is included in the signed message, so:

- A keeper cannot reuse a price signed for ledger N at ledger N+61 — the signature will no longer match.
- Prices are stored in **temporary storage** and expire automatically after one ledger.
- This prevents keepers from holding a favourable stale price and submitting it for a later, less favourable execution.

---

## 2. Price Pair (Min / Max)

Keepers submit both a `min_price` and a `max_price` for each token. The spread represents oracle uncertainty (bid/ask spread or aggregator confidence interval).

| Scenario | Price used | Rationale |
|---|---|---|
| Long increase (open long) | `max_price` | Worst case for the buyer |
| Long decrease (close long) | `min_price` | Worst case for the seller |
| Short increase (open short) | `min_price` | Worst case for the buyer |
| Short decrease (close short) | `max_price` | Worst case for the seller |

This ensures users always trade at the price least favourable to themselves, preventing oracle-based front-running.

---

## 3. ed25519 Verification

### Signed message format

The message bytes are assembled in `build_price_message` (`contracts/oracle/src/lib.rs`):

```
message = network_passphrase
        ‖ ledger_sequence  (4 bytes, big-endian u32)
        ‖ token_strkey     (variable-length Stellar address string, UTF-8)
        ‖ min_price        (16 bytes, big-endian i128)
        ‖ max_price        (16 bytes, big-endian i128)
        ‖ timestamp        (8 bytes, big-endian u64)
```

`env.crypto().ed25519_verify` hashes this message internally with SHA-512 before verifying; you do **not** pre-hash the message before signing.

### Keeper public keys

Keeper public keys are stored in `data_store` (persistent storage) at the key:

```
data_store_key = sha256( sha256("KEEPER_PUBLIC_KEY") ‖ keeper_index (4 bytes, big-endian u32) )
```

The value at that key is the raw 32-byte ed25519 public key (`BytesN<32>`).

Use `scripts/compute_key.py` to derive the `data_store_key`:

```bash
python3 scripts/compute_key.py keeper_pubkey_key <keeper_index>
```

### Verification call

```rust
env.crypto().ed25519_verify(&pubkey, &msg, &sp.signature);
// pubkey  : BytesN<32>  — 32-byte ed25519 public key
// msg     : Bytes       — raw message bytes as above (not pre-hashed)
// sig     : BytesN<64>  — ed25519 signature
```

If the signature is invalid the call panics; the transaction is rolled back.

---

## 4. Adding a New Price Feed

### Step 1 — Generate a keeper ed25519 keypair

```bash
stellar keys generate --global my-keeper --network testnet
```

Retrieve the Stellar address (this is the public key in Stellar's strkey encoding):

```bash
stellar keys address my-keeper
# → G...  (56-character strkey)
```

To export the raw 32-byte public key for storage in `data_store`, decode the strkey:

```bash
python3 - <<'EOF'
import base64, sys
# Stellar strkey: version byte (0x06 << 3 = 0x30) + 32-byte key + 2-byte checksum
raw = base64.b32decode(sys.argv[1].upper())
pubkey_hex = raw[1:33].hex()
print(pubkey_hex)
EOF "G..."
```

### Step 2 — Compute the data_store key

```bash
KEEPER_INDEX=0    # increment for each additional keeper
KEY_HEX=$(python3 scripts/compute_key.py keeper_pubkey_key "$KEEPER_INDEX")
echo "data_store key: $KEY_HEX"
```

### Step 3 — Register the public key in data_store

Requires CONTROLLER role on the caller:

```bash
stellar contract invoke \
  --id   "$DATA_STORE" \
  --source "$SOURCE" \
  --network testnet \
  -- set_bytes32 \
  --caller "$ADMIN" \
  --key    "$KEY_HEX" \
  --value  "<32-byte pubkey hex>"
```

### Step 4 — Test a sample price submission

```bash
bash scripts/submit_prices.sh testnet my-keeper
```

`submit_prices.sh` signs a test price bundle for the configured token and calls `oracle.set_prices`. If the signature or key lookup fails the invocation will revert.

---

## 5. Signer Key Rotation

If a keeper private key is compromised, rotate it immediately.

### OracleSignerRotated event

Every successful rotation emits an `OracleSignerRotated` event:

```
topic:  sig_rot
payload:
  keeper_index : u32         — the slot index whose key was replaced
  old_signer   : BytesN<32>  — the 32-byte pubkey that was overwritten
  new_signer   : BytesN<32>  — the 32-byte pubkey that now holds the slot
```

Off-chain monitoring services can subscribe to this event to audit key changes in real time and invalidate any in-flight price bundles signed by the old key before they are submitted.

### Option A — Use oracle.rotate_signer (recommended)

`oracle.rotate_signer` reads the old key, writes the new key, and emits `OracleSignerRotated` atomically. The caller must be the oracle admin.

```bash
NEW_PUBKEY_HEX="<new-32-byte-pubkey-hex>"

stellar contract invoke \
  --id   "$ORACLE" \
  --source "$ADMIN_SOURCE" \
  --network testnet \
  -- rotate_signer \
  --caller        "$ADMIN" \
  --keeper_index  "$KEEPER_INDEX" \
  --new_pubkey    "$NEW_PUBKEY_HEX"
```

The `OracleSignerRotated` event is emitted containing both the old and new public keys, giving a complete audit trail in the ledger history.

### Option B — Direct data_store overwrite

For manual or emergency rotation without going through the oracle contract:

1. Generate a new keypair (Step 1 above).
2. Overwrite the same `keeper_index` slot in `data_store` with the new 32-byte public key.

```bash
stellar contract invoke \
  --id   "$DATA_STORE" \
  --source "$SOURCE" \
  --network testnet \
  -- set_bytes32 \
  --caller "$ADMIN" \
  --key    "$KEY_HEX" \
  --value  "<new-32-byte-pubkey-hex>"
```

Note: Option B does **not** emit an `OracleSignerRotated` event. Prefer Option A for all routine rotations so audit logs remain complete.

The overwrite is atomic at the Soroban transaction level. Any price bundle signed by the old key that has not yet been submitted will be rejected once the key is replaced.

For a more complete treatment of key management and multi-signer setups, see `docs/SECURITY_REVIEW.md`.
