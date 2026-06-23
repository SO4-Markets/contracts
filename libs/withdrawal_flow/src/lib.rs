// Issue #36: Add router-based withdrawal E2E flow
// Implement end-to-end withdrawal flow through the exchange router

use multiversx_sc::prelude::*;

#[derive(TopEncode, TopDecode, NestedEncode, NestedDecode, Clone, Debug)]
pub struct WithdrawalRequest {
    pub account: ManagedAddress,
    pub token: TokenIdentifier,
    pub amount: BigUint,
    pub market: ManagedAddress,
    pub collateral_token: TokenIdentifier,
}

#[derive(TopEncode, TopDecode, NestedEncode, NestedDecode, Clone, Debug)]
pub struct WithdrawalReceipt {
    pub request_id: u64,
    pub account: ManagedAddress,
    pub token: TokenIdentifier,
    pub amount: BigUint,
    pub status: WithdrawalStatus,
    pub timestamp: u64,
}

#[derive(TopEncode, TopDecode, NestedEncode, NestedDecode, Clone, Debug, PartialEq, Eq)]
pub enum WithdrawalStatus {
    Pending,
    Processing,
    Completed,
    Failed,
    Cancelled,
}

#[multiversx_sc::module]
pub trait WithdrawalFlowModule {
    #[storage_mapper("withdrawal_requests")]
    fn withdrawal_requests(&self) -> MapMapper<u64, WithdrawalRequest>;

    #[storage_mapper("withdrawal_receipts")]
    fn withdrawal_receipts(&self) -> MapMapper<u64, WithdrawalReceipt>;

    #[storage_mapper("withdrawal_counter")]
    fn withdrawal_counter(&self) -> SingleValueMapper<u64>;

    #[storage_mapper("account_withdrawals")]
    fn account_withdrawals(&self, account: &ManagedAddress) -> UnorderedSetMapper<u64>;

    /// Initiate a withdrawal request through the router
    /// Issue #36: Add router-based withdrawal E2E flow
    fn initiate_withdrawal(
        &self,
        account: &ManagedAddress,
        token: TokenIdentifier,
        amount: BigUint,
        market: ManagedAddress,
        collateral_token: TokenIdentifier,
    ) -> u64 {
        let request_id = self.withdrawal_counter().get() + 1;
        
        let request = WithdrawalRequest {
            account: account.clone(),
            token,
            amount,
            market,
            collateral_token,
        };

        self.withdrawal_requests().insert(request_id, request);
        
        let receipt = WithdrawalReceipt {
            request_id,
            account: account.clone(),
            token: self.withdrawal_requests().get(&request_id).unwrap().token,
            amount: self.withdrawal_requests().get(&request_id).unwrap().amount,
            status: WithdrawalStatus::Pending,
            timestamp: self.blockchain().get_block_timestamp(),
        };

        self.withdrawal_receipts().insert(request_id, receipt);
        self.account_withdrawals(account).insert(request_id);
        self.withdrawal_counter().set(request_id);

        request_id
    }

    /// Process a withdrawal request
    fn process_withdrawal(&self, request_id: u64) -> bool {
        if let Some(mut receipt) = self.withdrawal_receipts().get(&request_id) {
            receipt.status = WithdrawalStatus::Processing;
            self.withdrawal_receipts().insert(request_id, receipt);
            true
        } else {
            false
        }
    }

    /// Complete a withdrawal request
    fn complete_withdrawal(&self, request_id: u64) -> bool {
        if let Some(mut receipt) = self.withdrawal_receipts().get(&request_id) {
            receipt.status = WithdrawalStatus::Completed;
            self.withdrawal_receipts().insert(request_id, receipt);
            true
        } else {
            false
        }
    }

    /// Fail a withdrawal request
    fn fail_withdrawal(&self, request_id: u64) -> bool {
        if let Some(mut receipt) = self.withdrawal_receipts().get(&request_id) {
            receipt.status = WithdrawalStatus::Failed;
            self.withdrawal_receipts().insert(request_id, receipt);
            true
        } else {
            false
        }
    }

    /// Cancel a withdrawal request
    fn cancel_withdrawal(&self, request_id: u64) -> bool {
        if let Some(mut receipt) = self.withdrawal_receipts().get(&request_id) {
            if receipt.status == WithdrawalStatus::Pending {
                receipt.status = WithdrawalStatus::Cancelled;
                self.withdrawal_receipts().insert(request_id, receipt);
                return true;
            }
        }
        false
    }

    /// Get withdrawal request details
    fn get_withdrawal_request(&self, request_id: u64) -> Option<WithdrawalRequest> {
        self.withdrawal_requests().get(&request_id)
    }

    /// Get withdrawal receipt
    fn get_withdrawal_receipt(&self, request_id: u64) -> Option<WithdrawalReceipt> {
        self.withdrawal_receipts().get(&request_id)
    }

    /// Get all withdrawals for an account
    fn get_account_withdrawals(&self, account: &ManagedAddress) -> Vec<u64> {
        self.account_withdrawals(account).iter().collect()
    }

    /// Get withdrawal status
    fn get_withdrawal_status(&self, request_id: u64) -> Option<WithdrawalStatus> {
        self.withdrawal_receipts()
            .get(&request_id)
            .map(|receipt| receipt.status)
    }

    /// Get total withdrawals for an account
    fn get_account_withdrawal_count(&self, account: &ManagedAddress) -> u64 {
        self.account_withdrawals(account).len() as u64
    }
}
