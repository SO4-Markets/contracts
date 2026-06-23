#!/usr/bin/env bash
set -e

# submit_prices.sh
# Smoke test script to push oracle prices to a local or testnet deployment.
# Requires the oracle contract to be compiled with `--features testutils`
# so that `set_prices_simple` is available.

if [ -z "$DEPLOY_ENV" ]; then
    DEPLOY_ENV=".deployed/testnet.env"
fi

if [ ! -f "$DEPLOY_ENV" ]; then
    echo "Error: DEPLOY_ENV file ($DEPLOY_ENV) not found. Run make deploy-all first."
    exit 1
fi

source "$DEPLOY_ENV"

NETWORK="${NETWORK:-testnet}"
SOURCE="${SOURCE:-alice}"

if [ -z "$ORACLE" ]; then
    echo "Error: ORACLE address not found in $DEPLOY_ENV"
    exit 1
fi

# We need a token to price. Let's try to get one from the env, or use a dummy.
# If MARKET_LONG_TOKEN is defined, we'll price that. Otherwise we just print a warning.
TOKEN_TO_PRICE="${MARKET_LONG_TOKEN:-CCZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZ}"

# Price = 2000 * FLOAT_PRECISION (10^30)
PRICE="2000000000000000000000000000000000"

echo "Submitting simple price ($PRICE) for token $TOKEN_TO_PRICE to Oracle $ORACLE on $NETWORK..."

# Extract the public key for the source account
CALLER=$(stellar keys address "$SOURCE")

stellar contract invoke \
  --id "$ORACLE" \
  --source "$SOURCE" \
  --network "$NETWORK" \
  -- set_prices_simple \
  --caller "$CALLER" \
  --prices '[{"token": "'"$TOKEN_TO_PRICE"'", "min": '"$PRICE"', "max": '"$PRICE"'}]'

echo "Prices submitted successfully!"
