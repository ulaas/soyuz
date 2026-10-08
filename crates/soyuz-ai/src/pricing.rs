//! Token / cost accounting: a configurable price table (`[[ai.prices]]`, model globs) and a
//! monthly usage ledger with per-profile budget caps (warn at N %, hard stop at 100 %).

use std::collections::BTreeMap;
use std::path::Path;

use globset::{Glob, GlobMatcher};
use serde::{Deserialize, Serialize};
use soyuz_core::config::AiPrice;

use crate::error::{AiError, AiResult};
use crate::types::Usage;

/// Compiled price table; the first matching row wins.
#[derive(Clone, Debug, Default)]
pub struct PriceTable {
    rows: Vec<(GlobMatcher, AiPrice)>,
}

impl PriceTable {
    pub fn new(prices: &[AiPrice]) -> Self {
        let rows = prices
            .iter()
            .filter_map(|p| match Glob::new(&p.model) {
                Ok(g) => Some((g.compile_matcher(), p.clone())),
                Err(e) => {
                    log::warn!("ai.prices: bad model glob “{}”: {e}", p.model);
                    None
                }
            })
            .collect();
        PriceTable { rows }
    }

    pub fn price(&self, model: &str) -> Option<&AiPrice> {
        self.rows.iter().find(|(g, _)| g.is_match(model)).map(|(_, p)| p)
    }

    /// Cost in USD, `None` when the model is not in the table.
    pub fn cost(&self, model: &str, u: &Usage) -> Option<f64> {
        let p = self.price(model)?;
        let m = 1_000_000.0;
        Some(
            u.input_tokens as f64 * p.input_per_mtok / m
                + u.output_tokens as f64 * p.output_per_mtok / m
                + u.cache_read_tokens as f64 * p.cache_read_per_mtok / m
                + u.cache_write_tokens as f64 * p.cache_write_per_mtok / m,
        )
    }
}

/// Spend of one profile in one month.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Spend {
    pub usage: Usage,
    pub cost_usd: f64,
    pub requests: u64,
    /// Requests whose model had no price (cost unknown).
    pub unpriced: u64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum BudgetStatus {
    /// No cap configured.
    Unlimited,
    Ok {
        fraction: f64,
    },
    /// At or above the warning threshold.
    Warn {
        fraction: f64,
    },
    /// Cap reached: new requests must be refused.
    Exceeded,
}

/// Usage per month (`YYYY-MM`) per profile, persisted as JSON (`ai-usage.json`).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct UsageLedger {
    pub months: BTreeMap<String, BTreeMap<String, Spend>>,
}

/// `YYYY-MM` of the current local month.
pub fn current_month() -> String {
    chrono::Local::now().format("%Y-%m").to_string()
}

impl UsageLedger {
    /// Load from `path`; a missing file is an empty ledger.
    pub fn load(path: &Path) -> AiResult<Self> {
        match std::fs::read_to_string(path) {
            Ok(t) => serde_json::from_str(&t).map_err(|e| AiError::Decode(format!("{}: {e}", path.display()))),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(AiError::Config(format!("{}: {e}", path.display()))),
        }
    }

    /// Atomic save.
    pub fn save(&self, path: &Path) -> AiResult<()> {
        let text = serde_json::to_vec_pretty(self).map_err(|e| AiError::Decode(e.to_string()))?;
        soyuz_fs::write_atomic(path, &text).map_err(|e| AiError::Config(e.to_string()))
    }

    /// Record one request's usage; returns its cost (when priced).
    pub fn record(
        &mut self,
        month: &str,
        profile: &str,
        model: &str,
        usage: &Usage,
        prices: &PriceTable,
    ) -> Option<f64> {
        let cost = prices.cost(model, usage);
        let s = self.months.entry(month.to_string()).or_default().entry(profile.to_string()).or_default();
        s.usage.add(usage);
        s.requests += 1;
        match cost {
            Some(c) => s.cost_usd += c,
            None => s.unpriced += 1,
        }
        cost
    }

    pub fn spend(&self, month: &str, profile: &str) -> Spend {
        self.months.get(month).and_then(|m| m.get(profile)).copied().unwrap_or_default()
    }

    /// Budget state of `profile` for `month` with a cap in USD (0 = none).
    pub fn budget(&self, month: &str, profile: &str, cap_usd: f64, warn_percent: u32) -> BudgetStatus {
        if cap_usd <= 0.0 {
            return BudgetStatus::Unlimited;
        }
        let fraction = self.spend(month, profile).cost_usd / cap_usd;
        if fraction >= 1.0 {
            BudgetStatus::Exceeded
        } else if fraction * 100.0 >= f64::from(warn_percent) {
            BudgetStatus::Warn { fraction }
        } else {
            BudgetStatus::Ok { fraction }
        }
    }

    /// `Err(BudgetExceeded)` when a new request for `profile` must be refused.
    pub fn check(&self, month: &str, profile: &str, cap_usd: f64) -> AiResult<()> {
        match self.budget(month, profile, cap_usd, 100) {
            BudgetStatus::Exceeded => Err(AiError::BudgetExceeded { profile: profile.to_string() }),
            _ => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use soyuz_core::config::default_prices;

    #[test]
    fn costs_and_budget() {
        let t = PriceTable::new(&default_prices());
        let u = Usage { input_tokens: 1_000_000, output_tokens: 100_000, cache_read_tokens: 0, cache_write_tokens: 0 };
        let c = t.cost("claude-opus-5-5", &u).unwrap();
        assert!((c - 6.0).abs() < 1e-9, "{c}");
        assert!(t.cost("gpt-unknown", &u).is_none());

        let mut l = UsageLedger::default();
        l.record("2026-10", "claude", "claude-opus-5-5", &u, &t);
        l.record("2026-10", "claude", "mystery", &u, &t);
        let s = l.spend("2026-10", "claude");
        assert_eq!(s.requests, 2);
        assert_eq!(s.unpriced, 1);
        assert_eq!(l.budget("2026-10", "claude", 0.0, 80), BudgetStatus::Unlimited);
        assert!(matches!(l.budget("2026-10", "claude", 7.0, 80), BudgetStatus::Warn { .. }));
        assert!(matches!(l.budget("2026-10", "claude", 100.0, 80), BudgetStatus::Ok { .. }));
        assert!(l.check("2026-10", "claude", 6.0).is_err());
        assert!(l.check("2026-11", "claude", 6.0).is_ok());

        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("ai-usage.json");
        assert_eq!(UsageLedger::load(&p).unwrap(), UsageLedger::default());
        l.save(&p).unwrap();
        assert_eq!(UsageLedger::load(&p).unwrap(), l);
    }
}
