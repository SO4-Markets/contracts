# Frontend Handoff: Testnet Faucet and Test Tokens

This note is for wiring the SO4.market testnet faucet into the frontend.

## Network

- Network: Stellar testnet
- RPC URL: `https://soroban-testnet.stellar.org`
- Network passphrase: `Test SDF Network ; September 2015`
- Deployer/admin: `GAUHMCMUP5FZO5675W3ISZ6E6CNYJGXBUW5WANE2JR4TGAARYCTSCBKI`

## Deployed Contract IDs

Current IDs are stored in:

- Protocol deployment: `.deployed/testnet.env`
- Test tokens/faucet: `.deployed/tokens-testnet.env`

Faucet and token IDs:

```txt
FAUCET=CDAARNES7HX5R4CPYUGQ7GE4YNDJLUMMIJ6W5VH6EGMQLDQBUAY6KDB4
TWBTC=CCJRNW5YLINR5QSY6I37GIBHW5SCKWDGTQ64YYTYF5TYFQDFQRJQY54O
TUSDC=CAURHHYKGSTPHFF6CIY6KMWASK26REMXZVEOR57UC7TMKUTTT4JYDV4J
```

Fresh protocol IDs after the force deploy:

```txt
ROLE_STORE=CDS2NPAZDNRBX6S2J5USXND2LQOMTSQNGN6AADOE2PYLB6BKC3JDDBV7
DATA_STORE=CASCNKIUZKVKCUVBU4OJLBNEPTAJ2LXF2RDWF23R3ZSY3T2CB2C6XES2
ORACLE=CD52LUS6LDU2KAWCO7O6XDTTDK2XXJORHV7C5ZXZEFHQBTQJNBW4SCGH
MARKET_FACTORY=CCS56AQSRNQQJKSUA4RDZZENF75V25OPPSPQOIEVV2H4VFKAJBGRBV3R
DEPOSIT_HANDLER=CCEDLVT6WND6WLYASYTA3SFNTZK33KGKG6CPL7PO4MVQX477TYTCYZGR
WITHDRAWAL_HANDLER=CA3H6FYEMHCWD3DZVLPEF4RRHCFMHZW4EHZOLPJAK7LBRIQR32PBQ2LS
ORDER_HANDLER=CCOJAUMH5LGNBMWWLBGL7WW2JGRJNFVQYOPRKWN6QO4GG7MAETJXQRVG
EXCHANGE_ROUTER=CBPLB57CHQONUJD4TZJ5LZST44FZVF42SMDXQQFPQ3RFQX32NOXRRWF3
READER=CB6GMEZMPOCHHMCH5JMR4UGF2OSCYAL5LFKLVSDCBKOJH4MJLJ7H4H5M
```

## Source Contract Paths

- Faucet contract source: `contracts/test_faucet/src/lib.rs`
- Test token contract source: `contracts/test_token/src/lib.rs`
- Faucet manifest: `contracts/test_faucet/Cargo.toml`
- Test token manifest: `contracts/test_token/Cargo.toml`

## Frontend Binding Paths

TypeScript bindings were generated from the current WASM artifacts:

- Faucet bindings package: `bindings/test-faucet`
- Faucet binding entrypoint: `bindings/test-faucet/src/index.ts`
- Token bindings package: `bindings/test-token`
- Token binding entrypoint: `bindings/test-token/src/index.ts`

Each generated package has:

- `package.json`
- `src/index.ts`
- `README.md`
- `tsconfig.json`

To build them:

```sh
cd bindings/test-faucet && npm install && npm run build
cd ../test-token && npm install && npm run build
```

In the frontend app, either copy these generated packages into the app workspace,
or add them as local file dependencies.

Example package dependencies:

```json
{
  "dependencies": {
    "test-faucet": "file:../contracts/bindings/test-faucet",
    "test-token": "file:../contracts/bindings/test-token"
  }
}
```

Adjust the relative path to match the frontend repo location.

## Faucet User Flow

The user should connect a Stellar wallet on testnet, then call the faucet with
their public key as `account`.

Primary faucet methods:

- `claim({ account, token })`: claim one token.
- `claim_many({ account, tokens })`: claim TWBTC and TUSDC in one transaction.
- `claim_amount({ token })`: read configured claim amount.
- `last_claim_ledger({ account, token })`: read user's last claim ledger.
- `cooldown_ledgers()`: read faucet cooldown.

Useful token read methods:

- `balance({ id })`
- `decimals()`
- `name()`
- `symbol()`

Token amounts use 7 decimals. Example:

- `10000000` = 1 token
- `1000000000` = 100 tokens

## Example Client Setup

```ts
import { Client as FaucetClient } from "test-faucet";
import { Client as TokenClient } from "test-token";

export const TESTNET = {
  rpcUrl: "https://soroban-testnet.stellar.org",
  networkPassphrase: "Test SDF Network ; September 2015",
};

export const CONTRACTS = {
  faucet: "CDAARNES7HX5R4CPYUGQ7GE4YNDJLUMMIJ6W5VH6EGMQLDQBUAY6KDB4",
  twbtc: "CCJRNW5YLINR5QSY6I37GIBHW5SCKWDGTQ64YYTYF5TYFQDFQRJQY54O",
  tusdc: "CAURHHYKGSTPHFF6CIY6KMWASK26REMXZVEOR57UC7TMKUTTT4JYDV4J",
};

export const faucet = new FaucetClient({
  ...TESTNET,
  contractId: CONTRACTS.faucet,
});

export const twbtc = new TokenClient({
  ...TESTNET,
  contractId: CONTRACTS.twbtc,
});

export const tusdc = new TokenClient({
  ...TESTNET,
  contractId: CONTRACTS.tusdc,
});
```

## Example Read Calls

```ts
const connectedAddress = "G...";

const [twbtcBalanceTx, tusdcBalanceTx, twbtcClaimTx, tusdcClaimTx] =
  await Promise.all([
    twbtc.balance({ id: connectedAddress }),
    tusdc.balance({ id: connectedAddress }),
    faucet.claim_amount({ token: CONTRACTS.twbtc }),
    faucet.claim_amount({ token: CONTRACTS.tusdc }),
  ]);

const balances = {
  twbtc: twbtcBalanceTx.result,
  tusdc: tusdcBalanceTx.result,
};

const claimAmounts = {
  twbtc: twbtcClaimTx.result,
  tusdc: tusdcClaimTx.result,
};
```

## Example Claim Transaction

Use `claim_many` for the main faucet button:

```ts
const tx = await faucet.claim_many({
  account: connectedAddress,
  tokens: [CONTRACTS.twbtc, CONTRACTS.tusdc],
});

// The returned object is an AssembledTransaction. The frontend wallet adapter
// should sign and submit this transaction for the connected user.
await tx.signAndSend();
```

If the wallet adapter does not patch `signAndSend`, use the assembled
transaction XDR flow expected by the wallet provider:

```ts
const tx = await faucet.claim_many({
  account: connectedAddress,
  tokens: [CONTRACTS.twbtc, CONTRACTS.tusdc],
});

const xdr = tx.toXDR();
// Send xdr to the connected wallet, then submit the signed XDR via Stellar SDK.
```

## UX Rules

- Show balances for TWBTC and TUSDC.
- Show configured claim amount for each token.
- Disable the claim button while a transaction is pending.
- If the contract returns `ClaimTooSoon`, tell the user to wait for the cooldown.
- If the user is on the wrong network, ask them to switch to Stellar testnet.

## Contract Behavior

- The faucet contract owns the test token contracts.
- Users authorize their own claim transaction with `account.require_auth()`.
- The faucet mints configured token amounts directly to the user.
- Each token can have an independent cooldown per user.
- The token contracts are only for testnet/demo use, not mainnet collateral.

## Regenerating Bindings

After contract code changes and a new build:

```sh
make build

stellar contract bindings typescript \
  --wasm target/wasm32v1-none/release/test_faucet.wasm \
  --output-dir bindings/test-faucet \
  --overwrite

stellar contract bindings typescript \
  --wasm target/wasm32v1-none/release/test_token.wasm \
  --output-dir bindings/test-token \
  --overwrite
```
