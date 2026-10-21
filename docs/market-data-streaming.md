# Real-Time Market-Data Streaming Architecture

This document describes how live market data flows from the SO4 on-chain contracts to client
applications, covering the transport layer, message schemas, sequence-gap handling, and the
client-side helpers for building order-book views and execution previews.

---

## Overview

```
┌──────────────────────┐        WebSocket / SSE         ┌─────────────────────┐
│  SO4 Contracts       │  ──── on-chain events ────►   │  Indexer / Gateway  │
│  (Soroban ledger)    │                                │  (off-chain relay)  │
└──────────────────────┘                                └────────┬────────────┘
                                                                 │  JSON stream
                                                                 ▼
                                                        ┌─────────────────────┐
                                                        │  Client SDK         │
                                                        │  (browser / node)   │
                                                        └─────────────────────┘
```

The indexer subscribes to Stellar's event stream for the SO4 contract address and re-publishes
normalised JSON messages over a WebSocket endpoint.  Clients connect once and receive a snapshot
followed by a continuous stream of incremental updates.

---

## Message Types

### Snapshot (`type: "snapshot"`)

Sent once immediately after a client subscribes.  Contains the full current order-book state.

```jsonc
{
  "type": "snapshot",
  "seq": 4201,          // last sequence number included
  "market": "XLM/USDC",
  "bids": [             // sorted best-to-worst
    { "price": "0.1050", "size": "12000" },
    { "price": "0.1049", "size":  "8500" }
  ],
  "asks": [
    { "price": "0.1051", "size":  "9000" },
    { "price": "0.1052", "size": "15000" }
  ],
  "ts": 1760000000000   // server-side Unix ms
}
```

### Delta (`type: "delta"`)

Incremental update.  Each entry has a `side` (`"bid"` or `"ask"`), a `price`, and a `size`.
A `size` of `"0"` means the price level has been removed.

```jsonc
{
  "type": "delta",
  "seq": 4202,
  "market": "XLM/USDC",
  "changes": [
    { "side": "bid", "price": "0.1050", "size": "13500" },
    { "side": "ask", "price": "0.1051", "size":     "0" }
  ],
  "ts": 1760000000100
}
```

### Trade (`type: "trade"`)

Published when a match occurs on-chain.

```jsonc
{
  "type": "trade",
  "seq": 4203,
  "market": "XLM/USDC",
  "side": "buy",
  "price": "0.1051",
  "size": "500",
  "ts": 1760000000200
}
```

### Heartbeat (`type: "heartbeat"`)

Sent every 5 seconds when no other message has been produced, so clients can distinguish a quiet
market from a dropped connection.

```jsonc
{ "type": "heartbeat", "seq": 4210, "ts": 1760000005000 }
```

---

## Sequence Numbers and Gap Detection

Every message carries a monotonically increasing `seq` field scoped to a single market.  Clients
**must** check that each incoming `seq` equals `(previous seq + 1)`.

| Condition                  | Action                                      |
|----------------------------|---------------------------------------------|
| `seq === previous + 1`     | Apply the delta normally                    |
| `seq > previous + 1`       | Gap detected — discard local book, re-subscribe to receive a fresh snapshot |
| `seq <= previous`          | Duplicate or rewind — discard silently      |

Re-subscription is a plain WebSocket reconnect; the server always sends a snapshot first.

---

## Client-Side Book Management

### Initialising from a Snapshot

```ts
type Level = { price: bigint; size: bigint }; // prices as integer ticks (×1e7)

interface LocalBook {
  seq: number;
  bids: Map<bigint, bigint>; // price → size
  asks: Map<bigint, bigint>;
}

function applySnapshot(msg: SnapshotMessage): LocalBook {
  const book: LocalBook = { seq: msg.seq, bids: new Map(), asks: new Map() };
  for (const l of msg.bids) book.bids.set(parseTick(l.price), parseTick(l.size));
  for (const l of msg.asks) book.asks.set(parseTick(l.price), parseTick(l.size));
  return book;
}
```

### Applying a Delta

```ts
function applyDelta(book: LocalBook, msg: DeltaMessage): void {
  if (msg.seq !== book.seq + 1) throw new Error("sequence gap");
  book.seq = msg.seq;
  for (const c of msg.changes) {
    const side = c.side === "bid" ? book.bids : book.asks;
    const size = parseTick(c.size);
    if (size === 0n) side.delete(parseTick(c.price));
    else side.set(parseTick(c.price), size);
  }
}
```

---

## Quote and Execution Preview Helpers

### Top-of-Book (L1)

```ts
function topOfBook(book: LocalBook) {
  const bestBid = maxKey(book.bids); // highest bid price
  const bestAsk = minKey(book.asks); // lowest ask price
  return {
    bid: bestBid ? { price: bestBid, size: book.bids.get(bestBid)! } : null,
    ask: bestAsk ? { price: bestAsk, size: book.asks.get(bestAsk)! } : null,
    spread: bestBid && bestAsk ? bestAsk - bestBid : null,
  };
}
```

### Level-2 View

```ts
function l2View(book: LocalBook, depth = 20) {
  const bids = [...book.bids.entries()]
    .sort(([a], [b]) => Number(b - a))
    .slice(0, depth)
    .map(([price, size]) => ({ price, size }));
  const asks = [...book.asks.entries()]
    .sort(([a], [b]) => Number(a - b))
    .slice(0, depth)
    .map(([price, size]) => ({ price, size }));
  return { bids, asks };
}
```

### Execution Preview (market order fill simulation)

```ts
/** Returns { filled, avgPrice, remaining, levels } for a market buy of `qty` ticks. */
function previewMarketBuy(book: LocalBook, qty: bigint) {
  const asks = [...book.asks.entries()].sort(([a], [b]) => Number(a - b));
  let remaining = qty;
  let cost = 0n;
  const levels: { price: bigint; size: bigint }[] = [];

  for (const [price, size] of asks) {
    if (remaining === 0n) break;
    const take = remaining < size ? remaining : size;
    cost += take * price;
    remaining -= take;
    levels.push({ price, size: take });
  }

  const filled = qty - remaining;
  const avgPrice = filled > 0n ? cost / filled : 0n;
  return { filled, avgPrice, remaining, levels };
}
```

---

## Connection Lifecycle

1. **Connect** — open `wss://<gateway>/stream?market=<symbol>`.
2. **Receive snapshot** — initialise local book.
3. **Receive deltas / trades** — apply in order; detect gaps as above.
4. **Heartbeat timeout** — if no message (including heartbeat) arrives within 10 s, reconnect.
5. **Reconnect back-off** — 1 s, 2 s, 4 s, 8 s (capped at 30 s), jittered ±20 %.

---

## Related Issues

- CLOB-157: client-side quote and execution preview helpers
- CLOB-158: initialise local book from snapshot and apply lifecycle events
- CLOB-159: sequence-gap detection, top-of-book and L2 views
