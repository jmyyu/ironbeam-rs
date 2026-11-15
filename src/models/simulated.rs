//! Simulated trader/account management (Enterprise users, demo environment
//! only). All wire names in this group are PascalCase.

use serde::{Deserialize, Serialize, Serializer};

use super::common::{ResponseStatus, null_default, opt_lenient_i64};

/// `POST /simulatedTraderCreate` request.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct SimulatedTraderCreate {
    pub first_name: String,
    pub last_name: String,
    pub address1: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub address2: Option<String>,
    pub city: String,
    pub state: String,
    pub country: String,
    pub zip_code: String,
    pub phone: String,
    pub email: String,
    pub password: String,
    /// Account template, e.g. `EVAL50`.
    pub template_id: String,
}

/// `POST /simulatedTraderCreate` response.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SimulatedTraderCreateResponse {
    pub status: Option<ResponseStatus>,
    pub message: Option<String>,
    #[serde(rename = "TraderId", alias = "traderId")]
    pub trader_id: Option<String>,
}

/// `POST /simulatedAccountAdd` request.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct SimulatedAccountAdd {
    pub trader_id: String,
    pub password: String,
    pub template_id: String,
}

/// `POST /simulatedAccountAdd` response.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SimulatedAccountAddResponse {
    pub status: Option<ResponseStatus>,
    pub message: Option<String>,
    #[serde(rename = "AccountId", alias = "accountId")]
    pub account_id: Option<String>,
}

/// One cash movement of a simulated account.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct CashReportEntry {
    pub amount: Option<f64>,
    /// `YYYYMMDD` as an integer.
    #[serde(deserialize_with = "opt_lenient_i64")]
    pub entry_date: Option<i64>,
    /// `YYYYMMDD` as an integer.
    #[serde(deserialize_with = "opt_lenient_i64")]
    pub available_date: Option<i64>,
}

/// `GET /simulatedAccount/getCashReport/{accountId}` response.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SimulatedAccountCashReport {
    pub status: Option<ResponseStatus>,
    pub message: Option<String>,
    #[serde(rename = "AccountId", alias = "accountId")]
    pub account_id: Option<String>,
    #[serde(
        rename = "CashReport",
        alias = "cashReport",
        deserialize_with = "null_default"
    )]
    pub cash_report: Vec<CashReportEntry>,
}

/// `POST /simulatedAccount/liquidate` request. The liquidated set is
/// `accounts ∪ accounts-in(groups) − except_accounts`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct SimulatedAccountLiquidate {
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub accounts: Vec<String>,
    /// Groups (`"303"`) or sub-groups (`"303-234"`).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub groups: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub except_accounts: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub force_manual_liquidation: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub use_manual_liquidation_for_illiquid_markets: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub send_account_email: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub send_office_email: Option<bool>,
}

/// Three-state value for the risk parameters of
/// [`SimulatedAccountSetRisk`]: the API distinguishes an omitted property
/// (leave unchanged) from an explicit `null` (disable).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum Setting<T> {
    /// Property omitted: the server keeps the current value.
    #[default]
    Unchanged,
    /// Property sent as `null`: the parameter is disabled.
    Clear,
    /// Property sent with a value.
    Set(T),
}

impl<T> Setting<T> {
    fn is_unchanged(&self) -> bool {
        matches!(self, Self::Unchanged)
    }
}

impl<T: Serialize> Serialize for Setting<T> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Set(v) => v.serialize(s),
            // `Unchanged` is skipped by the containing struct.
            Self::Clear | Self::Unchanged => s.serialize_none(),
        }
    }
}

/// `POST /simulatedAccount/setRisk` request. See the endpoint documentation
/// for the exact liquidation-level formula behind each parameter.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct SimulatedAccountSetRisk {
    pub account_id: String,
    #[serde(skip_serializing_if = "Setting::is_unchanged")]
    pub liquidation_account_value: Setting<f64>,
    #[serde(skip_serializing_if = "Setting::is_unchanged")]
    pub liquidation_loss_from_start_of_day: Setting<f64>,
    #[serde(skip_serializing_if = "Setting::is_unchanged")]
    pub liquidation_loss_from_high_of_day: Setting<f64>,
    #[serde(skip_serializing_if = "Setting::is_unchanged")]
    pub liquidation_loss_from_high_of_multiday: Setting<f64>,
    /// Percentage, 0–100.
    #[serde(skip_serializing_if = "Setting::is_unchanged")]
    pub liquidation_pct_loss_from_start_of_day: Setting<f64>,
    /// Percentage, 0–100.
    #[serde(skip_serializing_if = "Setting::is_unchanged")]
    pub liquidation_pct_loss_from_high_of_day: Setting<f64>,
    /// Percentage, 0–100.
    #[serde(skip_serializing_if = "Setting::is_unchanged")]
    pub liquidation_pct_loss_from_high_of_multiday: Setting<f64>,
    /// Percentage, 0–100.
    #[serde(skip_serializing_if = "Setting::is_unchanged")]
    pub liquidation_pct_margin_deficiency: Setting<f64>,
    #[serde(skip_serializing_if = "Setting::is_unchanged")]
    pub liquidation_max_value_override: Setting<f64>,
    /// `true` sets buying power to 0; `false` does nothing.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reduce_positions_only: Option<bool>,
    /// `true` takes the account out of liquidation (needs `template_id`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub restore_trading: Option<bool>,
    /// `Set("")` removes the margin schedule from the account.
    #[serde(skip_serializing_if = "Setting::is_unchanged")]
    pub margin_schedule_name: Setting<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub template_id: Option<String>,
}

impl SimulatedAccountSetRisk {
    /// A request that changes nothing until fields are set.
    pub fn new(account_id: impl Into<String>) -> Self {
        Self {
            account_id: account_id.into(),
            ..Self::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn set_risk_distinguishes_omitted_null_and_value() {
        let mut req = SimulatedAccountSetRisk::new("12345");
        req.liquidation_account_value = Setting::Set(90000.0);
        req.liquidation_loss_from_high_of_day = Setting::Clear;
        req.margin_schedule_name = Setting::Set(String::new());
        assert_eq!(
            serde_json::to_value(&req).unwrap(),
            json!({
                "AccountId": "12345",
                "LiquidationAccountValue": 90000.0,
                "LiquidationLossFromHighOfDay": null,
                "MarginScheduleName": ""
            })
        );
    }

    #[test]
    fn liquidate_omits_empty_lists() {
        let req = SimulatedAccountLiquidate {
            accounts: vec!["A-1".into()],
            ..Default::default()
        };
        assert_eq!(
            serde_json::to_value(&req).unwrap(),
            json!({"Accounts": ["A-1"]})
        );
    }
}
