use serde::{Deserialize, Serialize};
use specta::Type;

/// Default ignore rules used by inventory scans/checks.
pub const DEFAULT_INVENTORY_IGNORE_RULES: [&str; 2] = ["repo.json", "mod.srf"];

pub fn default_inventory_ignore_rules() -> String {
    DEFAULT_INVENTORY_IGNORE_RULES.join("\n")
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct InventoryIgnoreRules {
    pub patterns: Vec<String>,
}

impl Default for InventoryIgnoreRules {
    fn default() -> Self {
        Self {
            patterns: DEFAULT_INVENTORY_IGNORE_RULES
                .iter()
                .map(|s| (*s).to_string())
                .collect(),
        }
    }
}

impl InventoryIgnoreRules {
    pub fn parse(multiline: &str) -> Self {
        let patterns = multiline
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty() && !line.starts_with('#'))
            .map(|line| line.replace('\\', "/"))
            .collect();
        Self { patterns }
    }

    pub fn from_settings_value(value: &str) -> Self {
        let parsed = Self::parse(value);
        if parsed.patterns.is_empty() {
            Self::default()
        } else {
            parsed
        }
    }

    pub fn to_multiline_string(&self) -> String {
        self.patterns.join("\n")
    }
}
