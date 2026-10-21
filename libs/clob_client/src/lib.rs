//! Canonical off-chain protocol types.  These types deliberately contain no
//! contract storage representation and use checked integer arithmetic only.

use serde::{Deserialize, Serialize};

pub const PRICE_SCALE: u128 = 100_000_000;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Side {
    Bid,
    Ask,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum TimeInForce {
    Gtc,
    Gtd { expiry_ledger: u64 },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Market {
    pub market: String,
    pub base: String,
    pub quote: String,
    pub lot_size: u128,
    pub tick_size: u128,
    pub max_tick: u128,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct OrderRequest {
    pub owner: String,
    pub side: Side,
    pub lots: u128,
    pub tick: u128,
    pub tif: TimeInForce,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DepositRequest {
    pub trader: String,
    pub asset: String,
    pub amount: i128,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct WithdrawalRequest {
    pub trader: String,
    pub asset: String,
    pub amount: i128,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CancelRequest {
    pub owner: String,
    pub order_id: u128,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "method", content = "args")]
pub enum TransactionRequest {
    Deposit(DepositRequest),
    Withdraw(WithdrawalRequest),
    Place(OrderRequest),
    Cancel(CancelRequest),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ClientError {
    Invalid(String),
    Arithmetic(&'static str),
    Decode(String),
}

impl std::fmt::Display for ClientError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Invalid(message) => write!(f, "{message}"),
            Self::Arithmetic(message) => write!(f, "{message}"),
            Self::Decode(message) => write!(f, "{message}"),
        }
    }
}
impl std::error::Error for ClientError {}

fn positive(value: u128, name: &str) -> Result<(), ClientError> {
    if value == 0 {
        return Err(ClientError::Invalid(format!("{name} must be positive")));
    }
    Ok(())
}

pub fn scaled_price(tick: u128, market: &Market) -> Result<u128, ClientError> {
    if tick == 0 || tick > market.max_tick {
        return Err(ClientError::Invalid("tick is outside market bounds".into()));
    }
    tick.checked_mul(market.tick_size)
        .ok_or(ClientError::Arithmetic("scaled price overflow"))
}

pub fn quote_amount(
    lots: u128,
    tick: u128,
    market: &Market,
    round_up: bool,
) -> Result<u128, ClientError> {
    positive(lots, "lots")?;
    let numerator = lots
        .checked_mul(market.lot_size)
        .and_then(|value| {
            scaled_price(tick, market)
                .ok()
                .and_then(|price| value.checked_mul(price))
        })
        .ok_or(ClientError::Arithmetic("quote overflow"))?;
    if round_up {
        numerator
            .checked_add(PRICE_SCALE - 1)
            .map(|n| n / PRICE_SCALE)
            .ok_or(ClientError::Arithmetic("quote rounding overflow"))
    } else {
        Ok(numerator / PRICE_SCALE)
    }
}

pub fn validate_order(
    order: &OrderRequest,
    current_ledger: u64,
    market: &Market,
) -> Result<(), ClientError> {
    if order.owner.is_empty() {
        return Err(ClientError::Invalid("owner must not be empty".into()));
    }
    positive(order.lots, "lots")?;
    scaled_price(order.tick, market)?;
    if let TimeInForce::Gtd { expiry_ledger } = order.tif {
        if expiry_ledger <= current_ledger {
            return Err(ClientError::Invalid(
                "GTD expiry must be in the future".into(),
            ));
        }
    }
    Ok(())
}

pub fn build_order(
    order: OrderRequest,
    current_ledger: u64,
    market: &Market,
) -> Result<TransactionRequest, ClientError> {
    validate_order(&order, current_ledger, market)?;
    Ok(TransactionRequest::Place(order))
}

pub fn build_deposit(request: DepositRequest) -> Result<TransactionRequest, ClientError> {
    if request.trader.is_empty() || request.asset.is_empty() {
        return Err(ClientError::Invalid("address must not be empty".into()));
    }
    if request.amount <= 0 {
        return Err(ClientError::Invalid("amount must be positive".into()));
    }
    Ok(TransactionRequest::Deposit(request))
}

pub fn build_withdrawal(request: WithdrawalRequest) -> Result<TransactionRequest, ClientError> {
    if request.trader.is_empty() || request.asset.is_empty() {
        return Err(ClientError::Invalid("address must not be empty".into()));
    }
    if request.amount <= 0 {
        return Err(ClientError::Invalid("amount must be positive".into()));
    }
    Ok(TransactionRequest::Withdraw(request))
}

pub fn build_cancel(request: CancelRequest) -> Result<TransactionRequest, ClientError> {
    if request.owner.is_empty() {
        return Err(ClientError::Invalid("owner must not be empty".into()));
    }
    positive(request.order_id, "order_id")?;
    Ok(TransactionRequest::Cancel(request))
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProtocolError {
    pub code: Option<u32>,
    pub message: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProtocolEvent {
    pub event_type: String,
    #[serde(default)]
    pub data: serde_json::Value,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Receipt<T = serde_json::Value> {
    pub successful: bool,
    pub result: Option<T>,
    pub error: Option<ProtocolError>,
    #[serde(default)]
    pub events: Vec<ProtocolEvent>,
}

pub fn decode_receipt<T: for<'de> Deserialize<'de>>(
    bytes: &[u8],
) -> Result<Receipt<T>, ClientError> {
    serde_json::from_slice(bytes).map_err(|error| ClientError::Decode(error.to_string()))
}

pub trait Simulator {
    type Error;
    fn simulate(&self, request: &TransactionRequest) -> Result<Receipt, Self::Error>;
}

pub fn simulate<S: Simulator>(
    simulator: &S,
    request: &TransactionRequest,
) -> Result<Receipt, S::Error> {
    simulator.simulate(request)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn market() -> Market {
        Market {
            market: "m".into(),
            base: "b".into(),
            quote: "q".into(),
            lot_size: 100,
            tick_size: 150_000_000,
            max_tick: 1000,
        }
    }

    #[test]
    fn arithmetic_matches_protocol_rounding() {
        assert_eq!(quote_amount(3, 3, &market(), true).unwrap(), 1350);
        assert_eq!(quote_amount(3, 3, &market(), false).unwrap(), 1350);
    }

    #[test]
    fn invalid_requests_are_rejected() {
        let order = OrderRequest {
            owner: "x".into(),
            side: Side::Bid,
            lots: 1,
            tick: 0,
            tif: TimeInForce::Gtc,
        };
        assert!(validate_order(&order, 1, &market()).is_err());
        assert!(build_cancel(CancelRequest {
            owner: "x".into(),
            order_id: 0
        })
        .is_err());
    }

    #[test]
    fn request_serialization_is_stable() {
        let request = build_order(
            OrderRequest {
                owner: "x".into(),
                side: Side::Bid,
                lots: 2,
                tick: 3,
                tif: TimeInForce::Gtc,
            },
            1,
            &market(),
        )
        .unwrap();
        let encoded = serde_json::to_string(&request).unwrap();
        assert_eq!(
            encoded,
            r#"{"method":"Place","args":{"owner":"x","side":"bid","lots":2,"tick":3,"tif":{"kind":"gtc"}}}"#
        );
        let decoded: TransactionRequest = serde_json::from_str(&encoded).unwrap();
        assert_eq!(decoded, request);
    }

    #[test]
    fn receipt_errors_and_events_decode() {
        let receipt: Receipt = decode_receipt(br#"{"successful":false,"result":null,"error":{"code":7,"message":"bad tick"},"events":[{"event_type":"OrderRejected","data":{"tick":0}}]}"#).unwrap();
        assert_eq!(receipt.error.unwrap().code, Some(7));
        assert_eq!(receipt.events[0].event_type, "OrderRejected");
    }
}
