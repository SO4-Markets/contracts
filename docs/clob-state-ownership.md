# CLOB state ownership and topology

**Status:** v1 topology, chosen by #893. The options are being evaluated in #892, which is still
open. Maintainers should confirm this choice against #892 before it is treated as settled.

This document names the owner of every piece of CLOB state, the call paths that touch it, and
what the chosen topology means for deployment. Implementation issues that depend on the topology,
such as market configuration (#900) and custody (#905), should reference this document.

## 1. Topology

There are two kinds of contract.

1. **Market contract**, one instance per market. It owns all state for its market: orders, price
   levels, trader balances for its base and quote tokens, the protocol dust account, the order
   sequence counter, and the protocol fee accumulators. At creation it stores its immutable market
   parameters: base, quote, `lot_size`, `tick_size`, and `MarketFeeConfig`.
2. **Registry contract**, one instance. It owns the list of markets and creates market instances
   from the market contract code. It holds no trader balances, orders, or fees, and a market never
   calls it during trading.

All instances run the same market contract code. The registry only creates instances and records
their addresses.

## 2. State ownership

| State | Owner | Written by | Read by |
|-------|-------|-----------|---------|
| Market list and creation records | Registry | Registry admin, on market creation | Anyone, indexer |
| Market parameters (base, quote, `lot_size`, `tick_size`, fee rates) | Market | Market creation only | Market, reader |
| Trader balances per token: total, available, locked | Market | Deposit, withdrawal, placement, fill, cancel, expiry | Reader, indexer |
| Orders | Market | Placement, fill, cancel, expiry | Reader |
| Price levels and book index | Market | Placement, fill, cancel, expiry | Reader |
| Order sequence counter | Market | Placement | Placement |
| Protocol dust account | Market | Settlement (rounding residue) | Admin withdrawal |
| Protocol fee accumulators and liabilities, per token | Market | Settlement | Admin withdrawal |
| Tokens held by the market | Token contract, external | Token transfers made by the market | Solvency check (I2) |

No state has two authoritative owners. The registry owns the list of markets, not market state.
The market owns its balances and book. The token contract owns the tokens themselves, and the market's
accounting is authoritative for every trader and protocol balance. Invariant I2 checks the
token balance against that accounting.

## 3. Architecture and call paths

```text
                    +-------------------------+
  admin ----------> |        Registry         |  owns: market list, creation records
  create_market     +------------+------------+
                                 | deploy market instance (config passed once)
                                 v
  +---------------------------------------------------------------+
  |                      Market (one per market)                  |
  |  owns: orders, price levels, sequence counter,                |
  |        trader balances, dust account, fee accumulators,       |
  |        immutable market parameters                            |
  +----+------------------------+------------------------+--------+
       ^                        |                        |
       | deposit(amount)        | transfer out            | transfer out
       | place / cancel /       | on withdraw             | on fee withdrawal
       | expire / fill          v                        v
  trader, keeper        +---------------+        +---------------+
                        | Token (SAC)   |        | Token (SAC)   |
                        | external      |        | external      |
                        +---------------+        +---------------+
       ^
       | read-only queries and events
  reader / indexer
```

Call paths:

- **Deposit.** Trader calls `Market.deposit`. The market pulls tokens from the trader, then
  increases the trader's `total` and `available`.
- **Withdrawal.** Trader calls `Market.withdraw`. The market debits `available` and `total`, then
  transfers tokens to the trader.
- **Placement, fill, cancel, expiry.** These run entirely inside the market. They update balances,
  orders, and the book, and make no cross-contract call. Matching is bounded by `MAX_MATCH_STEPS`
  (I9).
- **Fee and dust withdrawal.** An admin calls `Market.withdraw_fees`. The market debits the
  protocol accumulator, then transfers the tokens to the treasury.
- **Market creation.** An admin calls `Registry.create_market` with the market configuration. The
  registry instantiates the market contract with that configuration and records the address.
- **Reads.** The reader and indexer call the market directly for read-only queries and consume its
  events. Neither writes market state.

External calls are limited to token transfers in deposit, withdrawal, and fee withdrawal. Trading
never calls out, so trading has no reentrancy surface.

## 4. Deployment implications

- **Code and instances.** The market code is uploaded once. Each market is an instance with its
  own storage, so the cost of a new market is one deployment.
- **Immutable parameters.** `lot_size`, `tick_size`, and fee rates are fixed at creation (spec
  section 3). Changing one means creating a new market in v1.
- **Upgrades.** Follow the upgradeability decision in #891. A market upgrade affects only that
  market's custody and book. A registry upgrade does not touch any balance.
- **Blast radius.** A defect in one market's code or state can affect only that market's funds
  and orders.
- **Shared assets.** A trader who holds one token in two markets deposits it into each market
  separately. Custody is never shared across markets, so no cross-market double-count check is
  needed. The cost is capital that cannot move between markets without withdrawing.
- **Resource limits.** Each invocation stays inside one market's storage and step budget. A busy
  market does not raise the cost of calls to another.
- **Events.** Each market emits its own events. The indexer subscribes to each market address,
  using the registry's list to discover them.

## 5. Alternatives considered

- **One shared custody contract for all markets.** This is the option the reader's #905 TODOs
  assume. It would add an external call to every deposit and withdrawal, and an invariant that no
  token is counted twice across markets. It is not selected for v1.
- **One contract holding every market.** This puts all markets under one storage budget and one
  failure domain, so it is not selected.
