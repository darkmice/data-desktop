//! Client-side watch rules (source of truth). Mirrored to the server on every
//! change. Tracks the quota lifecycle: used += 1 on successful submit; when used
//! reaches qty the rule auto-stops (enabled=false); re-enabling resets used to 0
//! (each enable = a fresh quota), per spec §5.0.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Rule {
    pub id: String,
    #[serde(default)]
    pub label: String,
    /// Exact youpinSkuId to match. REQUIRED for the rule to be active.
    #[serde(default)]
    pub youpin_sku_id: Option<String>,
    /// Inclusive price bounds. 0 or None on a side = unbounded on that side.
    #[serde(default)]
    pub price_min: Option<f64>,
    #[serde(default)]
    pub price_max: Option<f64>,
    #[serde(default = "one")]
    pub qty: u32,
    #[serde(default)]
    pub used: u32,
    #[serde(default = "yes")]
    pub enabled: bool,
}

fn one() -> u32 {
    1
}
fn yes() -> bool {
    true
}

impl Rule {
    /// Record one successful submit. Returns true if the rule just hit its
    /// quota and auto-stopped.
    pub fn on_success(&mut self) -> bool {
        self.used = self.used.saturating_add(1);
        if self.used >= self.qty {
            self.enabled = false;
            true
        } else {
            false
        }
    }

    /// User re-enables: reset used to 0 and run a fresh quota.
    pub fn reenable(&mut self) {
        self.used = 0;
        self.enabled = true;
    }
}

/// Apply a successful submit for `youpin_sku_id` to the matching rule.
/// Returns true if a rule auto-stopped.
pub fn record_success(rules: &mut [Rule], rule_id: &str) -> bool {
    if let Some(r) = rules.iter_mut().find(|r| r.id == rule_id) {
        r.on_success()
    } else {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(qty: u32) -> Rule {
        Rule {
            id: "r1".into(),
            label: "".into(),
            youpin_sku_id: None,
            price_min: None,
            price_max: None,
            qty,
            used: 0,
            enabled: true,
        }
    }

    #[test]
    fn auto_stops_at_quota() {
        let mut r = rule(2);
        assert!(!r.on_success()); // used=1
        assert!(r.on_success()); // used=2 → stop
        assert!(!r.enabled);
        assert_eq!(r.used, 2);
    }

    #[test]
    fn reenable_resets_used() {
        let mut r = rule(5);
        r.used = 5;
        r.enabled = false;
        r.reenable();
        assert!(r.enabled);
        assert_eq!(r.used, 0);
    }
}
