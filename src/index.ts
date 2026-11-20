export const PRICE_SCALE = 100_000_000n;

export type Side = "bid" | "ask";
export type TimeInForce = { kind: "gtc" } | { kind: "gtd"; expiryLedger: bigint };

export interface Market {
  market: string;
  base: string;
  quote: string;
  lotSize: bigint;
  tickSize: bigint;
  maxTick: bigint;
}

export interface OrderRequest {
  owner: string;
  side: Side;
  lots: bigint;
  tick: bigint;
  tif: TimeInForce;
}

export interface DepositRequest { trader: string; asset: string; amount: bigint }
export interface WithdrawalRequest { trader: string; asset: string; amount: bigint }
export interface CancelRequest { owner: string; orderId: bigint }
export interface ReadRequest { market: string; cursor?: bigint; limit?: number }

export interface TransactionRequest {
  method: "deposit" | "withdraw" | "place" | "cancel";
  args: Record<string, string | bigint | boolean>;
}

export interface SimulationResult<T = unknown> {
  ok: boolean;
  result?: T;
  error?: DecodedError;
  events?: DecodedEvent[];
}

export interface Simulator {
  simulate(request: TransactionRequest): Promise<SimulationResult>;
}

export interface DecodedEvent {
  type: string;
  data: Record<string, unknown>;
}

export interface DecodedError {
  code?: number;
  message: string;
  raw?: unknown;
}

export interface DecodedReceipt<T = unknown> {
  successful: boolean;
  result?: T;
  error?: DecodedError;
  events: DecodedEvent[];
}

export class SdkError extends Error {
  constructor(message: string, public readonly code?: string) {
    super(message);
    this.name = "SdkError";
  }
}

function assertPositive(value: bigint, name: string): void {
  if (value <= 0n) throw new SdkError(`${name} must be positive`, "INVALID_AMOUNT");
}

function assertAddress(value: string, name: string): void {
  if (value.length === 0) throw new SdkError(`${name} must not be empty`, "INVALID_ADDRESS");
}

export function decimalToUnits(value: string, decimals: number): bigint {
  if (!Number.isInteger(decimals) || decimals < 0) throw new SdkError("decimals must be a non-negative integer");
  if (!/^(?:0|[1-9]\d*)(?:\.\d+)?$/.test(value)) throw new SdkError(`invalid decimal: ${value}`, "INVALID_DECIMAL");
  const [whole, fraction = ""] = value.split(".");
  if (fraction.length > decimals) throw new SdkError("decimal has more precision than the asset", "PRECISION_LOSS");
  return BigInt(whole ?? "0") * 10n ** BigInt(decimals) + BigInt((fraction + "0".repeat(decimals)).slice(0, decimals) || "0");
}

export function decimalToLots(value: string, market: Market, baseDecimals: number): bigint {
  const atoms = decimalToUnits(value, baseDecimals);
  if (atoms % market.lotSize !== 0n) throw new SdkError("base amount is not lot-aligned", "LOT_ALIGNMENT");
  return atoms / market.lotSize;
}

export function decimalToTicks(value: string, market: Market, quoteDecimals: number, baseDecimals: number): bigint {
  const numerator = decimalToUnits(value, quoteDecimals) * 10n ** BigInt(baseDecimals) * PRICE_SCALE;
  const denominator = 10n ** BigInt(quoteDecimals) * market.tickSize;
  if (numerator % denominator !== 0n) throw new SdkError("price is not tick-aligned", "TICK_ALIGNMENT");
  const tick = numerator / denominator;
  if (tick < 1n || tick > market.maxTick) throw new SdkError("tick is outside market bounds", "TICK_RANGE");
  return tick;
}

export function scaledPrice(tick: bigint, market: Market): bigint {
  if (tick < 1n || tick > market.maxTick) throw new SdkError("tick is outside market bounds", "TICK_RANGE");
  return tick * market.tickSize;
}

export function quoteAmount(lots: bigint, tick: bigint, market: Market, rounding: "up" | "down"): bigint {
  assertPositive(lots, "lots");
  const numerator = lots * market.lotSize * scaledPrice(tick, market);
  return rounding === "down" ? numerator / PRICE_SCALE : (numerator + PRICE_SCALE - 1n) / PRICE_SCALE;
}

export function buildDeposit(request: DepositRequest): TransactionRequest {
  assertAddress(request.trader, "trader"); assertAddress(request.asset, "asset"); assertPositive(request.amount, "amount");
  return { method: "deposit", args: { trader: request.trader, asset: request.asset, amount: request.amount } };
}

export function buildWithdrawal(request: WithdrawalRequest): TransactionRequest {
  assertAddress(request.trader, "trader"); assertAddress(request.asset, "asset"); assertPositive(request.amount, "amount");
  return { method: "withdraw", args: { trader: request.trader, asset: request.asset, amount: request.amount } };
}

export function buildOrder(request: OrderRequest, currentLedger: bigint, market: Market): TransactionRequest {
  assertAddress(request.owner, "owner"); assertPositive(request.lots, "lots");
  if (request.tick < 1n || request.tick > market.maxTick) throw new SdkError("tick is outside market bounds", "TICK_RANGE");
  if (request.tif.kind === "gtd" && request.tif.expiryLedger <= currentLedger) throw new SdkError("GTD expiry must be in the future", "EXPIRY");
  if (request.tif.kind === "gtc" && "expiryLedger" in request.tif) throw new SdkError("GTC cannot have an expiry", "EXPIRY");
  return { method: "place", args: { owner: request.owner, side: request.side, lots: request.lots, tick: request.tick, tif: request.tif.kind, ...(request.tif.kind === "gtd" ? { expiryLedger: request.tif.expiryLedger } : {}) } };
}

export function buildCancel(request: CancelRequest): TransactionRequest {
  assertAddress(request.owner, "owner"); assertPositive(request.orderId, "orderId");
  return { method: "cancel", args: { owner: request.owner, orderId: request.orderId } };
}

export class Transaction<T = unknown> {
  constructor(public readonly request: TransactionRequest, private readonly simulator?: Simulator) {}
  async simulate(): Promise<SimulationResult<T>> {
    if (!this.simulator) throw new SdkError("no simulator configured", "NO_SIMULATOR");
    return this.simulator.simulate(this.request) as Promise<SimulationResult<T>>;
  }
}

export function decodeError(raw: unknown): DecodedError {
  if (raw instanceof Error) return { message: raw.message, raw };
  if (typeof raw === "string") return { message: raw, raw };
  if (raw && typeof raw === "object") {
    const value = raw as { message?: unknown; error?: unknown; code?: unknown };
    const error: DecodedError = { message: typeof value.message === "string" ? value.message : typeof value.error === "string" ? value.error : "transaction failed", raw };
    if (typeof value.code === "number") error.code = value.code;
    return error;
  }
  return { message: "transaction failed", raw };
}

export function decodeEvent(raw: unknown): DecodedEvent {
  if (!raw || typeof raw !== "object") throw new SdkError("invalid event", "INVALID_EVENT");
  const value = raw as { type?: unknown; name?: unknown; data?: unknown; value?: unknown };
  const type = typeof value.type === "string" ? value.type : typeof value.name === "string" ? value.name : undefined;
  if (!type) throw new SdkError("event type is missing", "INVALID_EVENT");
  const data = value.data ?? value.value ?? {};
  if (!data || typeof data !== "object" || Array.isArray(data)) throw new SdkError("event data must be an object", "INVALID_EVENT");
  return { type, data: data as Record<string, unknown> };
}

export function decodeReceipt<T = unknown>(raw: unknown): DecodedReceipt<T> {
  if (!raw || typeof raw !== "object") throw new SdkError("invalid receipt", "INVALID_RECEIPT");
  const value = raw as { successful?: unknown; status?: unknown; result?: unknown; error?: unknown; events?: unknown };
  const successful = value.successful === true || value.status === "success";
  const events = Array.isArray(value.events) ? value.events.map(decodeEvent) : [];
  if (!successful) return { successful: false, error: decodeError(value.error ?? raw), events };
  return { successful: true, result: value.result as T, events };
}
