import { buildDeposit, buildOrder, buildCancel, Transaction } from "../src/index.js";

export async function runTestnetFlow(simulator: import("../src/index.js").Simulator) {
  const deposit = new Transaction(buildDeposit({ trader: "G...", asset: "C...", amount: 1_000_000n }), simulator);
  await deposit.simulate();
  const order = new Transaction(buildOrder({ owner: "G...", side: "bid", lots: 10n, tick: 100n, tif: { kind: "gtc" } }, 1n, {
    market: "M...", base: "C...", quote: "C...", lotSize: 1n, tickSize: 100_000_000n, maxTick: 1_000_000n
  }), simulator);
  await order.simulate();
  return new Transaction(buildCancel({ owner: "G...", orderId: 1n }), simulator).simulate();
}
