use crate::client::{Call, Client};
use crate::endpoints as ep;
use crate::error::Result;
use crate::models::*;

impl Client {
    /// Balance of one account.
    pub async fn account_balance(
        &self,
        account_id: &str,
        balance_type: BalanceType,
    ) -> Result<AccountBalanceResponse> {
        let call = Call::new(&ep::ACCOUNT_BALANCE)
            .params(&[("accountId", account_id)])
            .query("balanceType", balance_type.as_wire());
        self.send(call).await
    }

    /// Open positions of one account.
    pub async fn account_positions(&self, account_id: &str) -> Result<AccountPositions> {
        self.send(Call::new(&ep::ACCOUNT_POSITIONS).params(&[("accountId", account_id)]))
            .await
    }

    /// Risk information (liquidation levels) of one account.
    pub async fn account_risk(&self, account_id: &str) -> Result<AccountRiskResponse> {
        self.send(Call::new(&ep::ACCOUNT_RISK).params(&[("accountId", account_id)]))
            .await
    }

    /// Fills of one account, in no particular order.
    pub async fn account_fills(&self, account_id: &str) -> Result<FillsResponse> {
        self.send(Call::new(&ep::ACCOUNT_FILLS).params(&[("accountId", account_id)]))
            .await
    }

    /// All accounts of the authenticated user.
    pub async fn all_accounts(&self) -> Result<AllAccountsResponse> {
        self.send(Call::new(&ep::ALL_ACCOUNTS)).await
    }

    /// Balances of all accounts.
    pub async fn all_balances(&self, balance_type: BalanceType) -> Result<AccountBalanceResponse> {
        let call = Call::new(&ep::ALL_BALANCES).query("balanceType", balance_type.as_wire());
        self.send(call).await
    }

    /// Fills of all accounts, in no particular order.
    pub async fn all_fills(&self) -> Result<FillsResponse> {
        self.send(Call::new(&ep::ALL_FILLS)).await
    }

    /// Positions of all accounts.
    pub async fn all_positions(&self) -> Result<AllPositionsResponse> {
        self.send(Call::new(&ep::ALL_POSITIONS)).await
    }

    /// Risk information of all accounts.
    pub async fn all_risk(&self) -> Result<AccountRiskResponse> {
        self.send(Call::new(&ep::ALL_RISK)).await
    }
}
