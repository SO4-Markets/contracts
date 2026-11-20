use clob_client::{
    build_cancel, build_deposit, build_order, simulate, CancelRequest, DepositRequest, Market,
    OrderRequest, Receipt, Side, Simulator, TimeInForce, TransactionRequest,
};

struct DryRun;
impl Simulator for DryRun {
    type Error = &'static str;
    fn simulate(&self, _request: &TransactionRequest) -> Result<Receipt, Self::Error> {
        Ok(Receipt {
            successful: true,
            result: None,
            error: None,
            events: Vec::new(),
        })
    }
}

fn main() {
    let market = Market {
        market: "testnet-market".into(),
        base: "base".into(),
        quote: "quote".into(),
        lot_size: 1,
        tick_size: 100_000_000,
        max_tick: 1_000_000,
    };
    let simulator = DryRun;
    let deposit = build_deposit(DepositRequest {
        trader: "trader".into(),
        asset: "quote".into(),
        amount: 1_000_000,
    })
    .unwrap();
    simulate(&simulator, &deposit).unwrap();
    let order = build_order(
        OrderRequest {
            owner: "trader".into(),
            side: Side::Bid,
            lots: 10,
            tick: 100,
            tif: TimeInForce::Gtc,
        },
        1,
        &market,
    )
    .unwrap();
    simulate(&simulator, &order).unwrap();
    let cancel = build_cancel(CancelRequest {
        owner: "trader".into(),
        order_id: 1,
    })
    .unwrap();
    simulate(&simulator, &cancel).unwrap();
}
