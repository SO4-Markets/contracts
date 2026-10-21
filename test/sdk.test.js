import assert from "node:assert/strict";
import test from "node:test";
import { PRICE_SCALE, SdkError, decimalToLots, decimalToTicks, quoteAmount, decodeReceipt } from "../dist/index.js";

const market = { market: "m", base: "b", quote: "q", lotSize: 100n, tickSize: 150000000n, maxTick: 1000n };
test("exact conversions reject unaligned values", () => {
  assert.equal(decimalToLots("0.000001", market, 8), 1n);
  assert.throws(() => decimalToLots("0.00000001", market, 8), SdkError);
  assert.equal(decimalToTicks("4.5", market, 1, 0), 3n);
  assert.throws(() => decimalToTicks("4.6", market, 1, 0), SdkError);
});
test("quote arithmetic never uses floating point", () => {
  assert.equal(quoteAmount(3n, 3n, market, "up"), 1350n);
  assert.equal(quoteAmount(3n, 3n, market, "down"), 1350n);
  assert.equal(PRICE_SCALE, 100000000n);
});
test("receipts decode events and errors", () => {
  assert.deepEqual(decodeReceipt({ successful: true, result: 1n, events: [{ name: "Fill", data: { lots: 2n } }] }).events[0].type, "Fill");
  assert.equal(decodeReceipt({ status: "failed", error: { code: 7, message: "bad tick" } }).error.message, "bad tick");
});
