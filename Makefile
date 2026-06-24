# SO4.market contract operator entrypoint.
#
# The implementation lives in mx/*.mk so deployment, testing, token setup, and
# upgrade workflows can grow without turning this file into a wall of shell.

include mx/common.mk
include mx/build.mk
include mx/test.mk
include mx/deploy.mk
include mx/upgrade.mk
include mx/tokens.mk

.PHONY: all help clean

all: build test

help:
	@printf '%s\n' 'SO4.market contract commands'
	@printf '%s\n' ''
	@printf '%s\n' 'Build and test:'
	@printf '%s\n' '  make check'
	@printf '%s\n' '  make lint'
	@printf '%s\n' '  make test'
	@printf '%s\n' '  make build'
	@printf '%s\n' '  make smoke-prices'
	@printf '%s\n' ''
	@printf '%s\n' 'Deploy and upgrade:'
	@printf '%s\n' '  make deploy-all NETWORK=testnet SOURCE=alice'
	@printf '%s\n' '  make deploy-contract CONTRACT=reader NETWORK=testnet SOURCE=alice'
	@printf '%s\n' '  make upgrade-contract CONTRACT=deposit_handler NETWORK=testnet SOURCE=alice'
	@printf '%s\n' '  make upgrade-all NETWORK=testnet SOURCE=alice'
	@printf '%s\n' '  make upload CONTRACT=deposit_handler NETWORK=testnet SOURCE=alice'
	@printf '%s\n' ''
	@printf '%s\n' 'Test assets:'
	@printf '%s\n' '  make token-deploy CODE=TWBTC NETWORK=testnet SOURCE=alice'
	@printf '%s\n' '  make token-mint CODE=TWBTC TO=alice AMOUNT=1000000000 NETWORK=testnet SOURCE=alice'
	@printf '%s\n' '  make token-bootstrap CODE=TWBTC TO=alice NETWORK=testnet SOURCE=alice'
	@printf '%s\n' '  make test-tokens-with-faucet NETWORK=testnet SOURCE=alice LONG_CODE=TWBTC SHORT_CODE=TUSDC'
	@printf '%s\n' '  make faucet-claim-market NETWORK=testnet SOURCE=alice TO=alice'
	@printf '%s\n' ''
	@printf '%s\n' 'Run make help-mx for the longer operator guide.'

clean:
	cargo clean
	rm -rf .deployed
