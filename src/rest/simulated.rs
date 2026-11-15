use serde::Serialize;
use serde_json::{Value, json};

use crate::client::{Call, Client};
use crate::endpoints::{self as ep, Endpoint};
use crate::error::{Error, Result};
use crate::models::*;

/// Simulated trader/account management. Enterprise users on the demo
/// environment only; everything else gets `403 Forbidden`. None of these
/// calls is retried automatically.
impl Client {
    async fn sim_call<B: Serialize, T: serde::de::DeserializeOwned>(
        &self,
        endpoint: &'static Endpoint,
        body: &B,
    ) -> Result<T> {
        let body = serde_json::to_value(body)
            .map_err(|e| Error::InvalidRequest(format!("unserializable request: {e}")))?;
        self.send(Call::new(endpoint).body(body)).await
    }

    /// Creates a simulated trader. Follow up with
    /// [`simulated_account_add`](Self::simulated_account_add).
    pub async fn simulated_trader_create(
        &self,
        request: &SimulatedTraderCreate,
    ) -> Result<SimulatedTraderCreateResponse> {
        self.sim_call(&ep::SIM_TRADER_CREATE, request).await
    }

    /// Adds a simulated account to a simulated trader.
    pub async fn simulated_account_add(
        &self,
        request: &SimulatedAccountAdd,
    ) -> Result<SimulatedAccountAddResponse> {
        self.sim_call(&ep::SIM_ACCOUNT_ADD, request).await
    }

    /// Resets a simulated account to the template's starting state. Destroys
    /// the account's trades and positions.
    pub async fn simulated_account_reset(
        &self,
        account_id: &str,
        template_id: &str,
    ) -> Result<ApiResponse> {
        let body = json!({ "AccountId": account_id, "TemplateId": template_id });
        self.sim_call(&ep::SIM_ACCOUNT_RESET, &body).await
    }

    /// Expires a simulated account.
    pub async fn simulated_account_expire(&self, account_id: &str) -> Result<ApiResponse> {
        self.sim_call(&ep::SIM_ACCOUNT_EXPIRE, &json!({ "AccountId": account_id }))
            .await
    }

    /// Adds cash to a simulated account.
    pub async fn simulated_account_add_cash(
        &self,
        account_id: &str,
        amount: f64,
    ) -> Result<ApiResponse> {
        if !amount.is_finite() {
            return Err(Error::InvalidRequest(format!(
                "amount must be finite, got {amount}"
            )));
        }
        let body = json!({ "AccountId": account_id, "Amount": amount });
        self.sim_call(&ep::SIM_ADD_CASH, &body).await
    }

    /// Cash report of a simulated account. Dates are `YYYYMMDD` integers.
    pub async fn simulated_account_cash_report(
        &self,
        account_id: &str,
        start_date: u32,
        end_date: u32,
    ) -> Result<SimulatedAccountCashReport> {
        let call = Call::new(&ep::SIM_CASH_REPORT)
            .params(&[("accountId", account_id)])
            .query("startDate", start_date.to_string())
            .query("endDate", end_date.to_string());
        self.send(call).await
    }

    /// Sets or clears liquidate-only mode. `template_id` is required when
    /// `liquidate_only` is `false`.
    #[deprecated(note = "deprecated by Ironbeam; use `simulated_account_set_risk`")]
    pub async fn simulated_account_set_liquidate_only(
        &self,
        account_id: &str,
        liquidate_only: bool,
        template_id: Option<&str>,
    ) -> Result<ApiResponse> {
        if !liquidate_only && template_id.is_none() {
            return Err(Error::InvalidRequest(
                "template_id is required when liquidate_only is false".into(),
            ));
        }
        let mut body = json!({ "AccountId": account_id, "LiquidateOnly": liquidate_only });
        if let Some(t) = template_id {
            body["TemplateId"] = Value::from(t);
        }
        self.sim_call(&ep::SIM_SET_LIQUIDATE_ONLY, &body).await
    }

    /// Liquidates simulated accounts: cancels working orders, closes
    /// positions at market and blocks new positions.
    pub async fn simulated_account_liquidate(
        &self,
        request: &SimulatedAccountLiquidate,
    ) -> Result<ApiResponse> {
        if request.accounts.is_empty() && request.groups.is_empty() {
            return Err(Error::InvalidRequest(
                "no accounts or groups to liquidate".into(),
            ));
        }
        self.sim_call(&ep::SIM_LIQUIDATE, request).await
    }

    /// Sets risk limits and automated-liquidation parameters.
    pub async fn simulated_account_set_risk(
        &self,
        request: &SimulatedAccountSetRisk,
    ) -> Result<ApiResponse> {
        self.sim_call(&ep::SIM_SET_RISK, request).await
    }
}
