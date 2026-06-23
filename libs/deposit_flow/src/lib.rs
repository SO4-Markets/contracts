// Issue #35: Add router-based deposit E2E flow
// Implement end-to-end deposit flow through the exchange router

use multiversx_sc::prelude::*;

#[derive(TopEncode, TopDecode, NestedEncode, NestedDecode, Clone, Debug)]
pub struct DepositRequest {
    pub account: ManagedAddress,
    pub token: TokenIdentifier,
    pub amount: BigUint,
    pub market: ManagedAddress,
    pub collateral_token: TokenIdentifier,
}

#[derive(TopEncode, TopDecode, NestedEncode, NestedDecode, Clone, Debug)]
pub struct DepositReceipt {
    pub request_id: u64,
    pub account: ManagedAddress,
    pub token: TokenIdentifier,
    pub amount: BigUint,
    pub status: DepositStatus,
    pub timestamp: u64,
}

#[derive(TopEncode, TopDecode, NestedEncode, NestedDecode, Clone, Debug, PartialEq, Eq)]
pub enum DepositStatus {
    Pending,
    Processing,
    Completed,
    Failed,
    Cancelled,
}

#[multiversx_sc::module]
pub trait DepositFlowModule {
    #[storage_mapper("deposit_requests")]
    fn deposit_requests(&self) -> MapMapper<u64, DepositRequest>;

    #[storage_mapper("deposit_receipts")]
    fn deposit_receipts(&self) -> MapMapper<u64, DepositReceipt>;

    #[storage_mapper("deposit_counter")]
    fn deposit_counter(&self) -> SingleValueMapper<u64>;

    #[storage_mapper("account_deposits")]
    fn account_deposits(&self, account: &ManagedAddress) -> UnorderedSetMapper<u64>;

    /// Initiate a deposit request through the router
    /// Issue #35: Add router-based deposit E2E flow
    fn initiate_deposit(
        &self,
        account: &ManagedAddress,
        token: TokenIdentifier,
        amount: BigUint,
        market: ManagedAddress,
        collateral_token: TokenIdentifier,
    ) -> u64 {
        let request_id = self.deposit_counter().get() + 1;
        
        let request = DepositRequest {
            account: account.clone(),
            token,
            amount,
            market,
            collateral_token,
        };

        self.deposit_requests().insert(request_id, request);
        
        let receipt = DepositReceipt {
            request_id,
            account: account.clone(),
            token: self.deposit_requests().get(&request_id).unwrap().token,
            amount: self.deposit_requests().get(&request_id).unwrap().amount,
            status: DepositStatus::Pending,
            timestamp: self.blockchain().get_block_timestamp(),
        };

        self.deposit_receipts().insert(request_id, receipt);
        self.account_deposits(account).insert(request_id);
        self.deposit_counter().set(request_id);

        request_id
    }

    /// Process a deposit request
    fn process_deposit(&self, request_id: u64) -> bool {
        if let Some(mut receipt) = self.deposit_receipts().get(&request_id) {
            receipt.status = DepositStatus::Processing;
            self.deposit_receipts().insert(request_id, receipt);
            true
        } else {
            false
        }
    }

    /// Complete a deposit request
    fn complete_deposit(&self, request_id: u64) -> bool {
        if let Some(mut receipt) = self.deposit_receipts().get(&request_id) {
            receipt.status = DepositStatus::Completed;
            self.deposit_receipts().insert(request_id, receipt);
            true
        } else {
            false
        }
    }

    /// Fail a deposit request
    fn fail_deposit(&self, request_id: u64) -> bool {
        if let Some(mut receipt) = self.deposit_receipts().get(&request_id) {
            receipt.status = DepositStatus::Failed;
            self.deposit_receipts().insert(request_id, receipt);
            true
        } else {
            false
        }
    }

    /// Cancel a deposit request
    fn cancel_deposit(&self, request_id: u64) -> bool {
        if let Some(mut receipt) = self.deposit_receipts().get(&request_id) {
            if receipt.status == DepositStatus::Pending {
                receipt.status = DepositStatus::Cancelled;
                self.deposit_receipts().insert(request_id, receipt);
                return true;
            }
        }
        false
    }

    /// Get deposit request details
    fn get_deposit_request(&self, request_id: u64) -> Option<DepositRequest> {
        self.deposit_requests().get(&request_id)
    }

    /// Get deposit receipt
    fn get_deposit_receipt(&self, request_id: u64) -> Option<DepositReceipt> {
        self.deposit_receipts().get(&request_id)
    }

    /// Get all deposits for an account
    fn get_account_deposits(&self, account: &ManagedAddress) -> Vec<u64> {
        self.account_deposits(account).iter().collect()
    }

    /// Get deposit status
    fn get_deposit_status(&self, request_id: u64) -> Option<DepositStatus> {
        self.deposit_receipts()
            .get(&request_id)
            .map(|receipt| receipt.status)
    }

    /// Get total deposits for an account
    fn get_account_deposit_count(&self, account: &ManagedAddress) -> u64 {
        self.account_deposits(account).len() as u64
    }
}
