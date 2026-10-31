use serde::{Deserialize, Serialize};
use serde_json::Value;

const JSON_STRUCTURAL_TOKEN_COST: u64 = 1;

/// Provider-neutral context budget for Mahayana Agent loops.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextBudget {
    pub max_input_tokens: u64,
    pub reserve_output_tokens: u64,
    pub compact_at_percent: u8,
    pub retain_recent_items: usize,
}

impl ContextBudget {
    pub fn new(
        max_input_tokens: u64,
        reserve_output_tokens: u64,
        compact_at_percent: u8,
        retain_recent_items: usize,
    ) -> Result<Self, ContextError> {
        if max_input_tokens == 0
            || reserve_output_tokens >= max_input_tokens
            || !(1..=100).contains(&compact_at_percent)
            || retain_recent_items == 0
        {
            return Err(ContextError::InvalidBudget);
        }
        Ok(Self {
            max_input_tokens,
            reserve_output_tokens,
            compact_at_percent,
            retain_recent_items,
        })
    }

    pub fn usable_input_tokens(&self) -> u64 {
        self.max_input_tokens
            .saturating_sub(self.reserve_output_tokens)
    }

    pub fn compaction_threshold(&self) -> u64 {
        self.usable_input_tokens()
            .saturating_mul(u64::from(self.compact_at_percent))
            / 100
    }
}

impl Default for ContextBudget {
    fn default() -> Self {
        Self {
            max_input_tokens: 128_000,
            reserve_output_tokens: 16_000,
            compact_at_percent: 80,
            retain_recent_items: 12,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompactionPlan {
    pub estimated_tokens: u64,
    pub threshold_tokens: u64,
    /// Items in `[0, compact_through)` should be summarized.
    pub compact_through: usize,
    pub retained_items: usize,
}

impl CompactionPlan {
    pub fn required(&self) -> bool {
        self.compact_through > 0
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompactionRequest {
    pub prefix: Vec<Value>,
    pub retained: Vec<Value>,
    pub estimated_tokens: u64,
    pub target_tokens: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompactionResult {
    pub summary: Value,
    pub retained: Vec<Value>,
}

impl CompactionResult {
    pub fn into_history(self) -> Vec<Value> {
        let mut history = Vec::with_capacity(self.retained.len() + 1);
        history.push(self.summary);
        history.extend(self.retained);
        history
    }
}

/// Deterministic fallback estimator used when a model/provider tokenizer is
/// unavailable. It intentionally overestimates structural JSON overhead and
/// can be replaced by a provider-specific exact tokenizer at the model edge.
pub fn estimate_json_tokens(value: &Value) -> u64 {
    match value {
        Value::Null | Value::Bool(_) => 1,
        Value::Number(number) => estimate_text_tokens(&number.to_string()).saturating_add(1),
        Value::String(text) => estimate_text_tokens(text).saturating_add(2),
        Value::Array(values) => values
            .iter()
            .fold(JSON_STRUCTURAL_TOKEN_COST, |sum, value| {
                sum.saturating_add(estimate_json_tokens(value))
                    .saturating_add(JSON_STRUCTURAL_TOKEN_COST)
            }),
        Value::Object(values) => {
            values
                .iter()
                .fold(JSON_STRUCTURAL_TOKEN_COST, |sum, (key, value)| {
                    sum.saturating_add(estimate_text_tokens(key))
                        .saturating_add(estimate_json_tokens(value))
                        .saturating_add(2 * JSON_STRUCTURAL_TOKEN_COST)
                })
        }
    }
}

pub fn estimate_history_tokens(history: &[Value]) -> u64 {
    history.iter().fold(0, |sum, value| {
        sum.saturating_add(estimate_json_tokens(value))
            .saturating_add(2)
    })
}

fn estimate_text_tokens(text: &str) -> u64 {
    let chars = text.chars().count() as u64;
    chars.saturating_add(2) / 3
}

pub fn plan_compaction(history: &[Value], budget: ContextBudget) -> CompactionPlan {
    let estimated_tokens = estimate_history_tokens(history);
    let threshold_tokens = budget.compaction_threshold();
    if estimated_tokens <= threshold_tokens || history.len() <= budget.retain_recent_items {
        return CompactionPlan {
            estimated_tokens,
            threshold_tokens,
            compact_through: 0,
            retained_items: history.len(),
        };
    }

    let compact_through = history.len().saturating_sub(budget.retain_recent_items);
    CompactionPlan {
        estimated_tokens,
        threshold_tokens,
        compact_through,
        retained_items: history.len().saturating_sub(compact_through),
    }
}

pub fn prepare_compaction(history: &[Value], budget: ContextBudget) -> Option<CompactionRequest> {
    let plan = plan_compaction(history, budget);
    if !plan.required() {
        return None;
    }
    Some(CompactionRequest {
        prefix: history[..plan.compact_through].to_vec(),
        retained: history[plan.compact_through..].to_vec(),
        estimated_tokens: plan.estimated_tokens,
        target_tokens: budget.compaction_threshold(),
    })
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ContextError {
    #[error("context budget is invalid")]
    InvalidBudget,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn estimator_is_monotonic_for_more_history() {
        let first = vec![json!({"role": "user", "content": "hello"})];
        let mut second = first.clone();
        second.push(json!({"role": "assistant", "content": "hello there"}));
        assert!(estimate_history_tokens(&second) > estimate_history_tokens(&first));
    }

    #[test]
    fn compaction_keeps_recent_turns_exact() {
        let history = (0..20)
            .map(|index| json!({"role": "user", "content": "x".repeat(120), "i": index}))
            .collect::<Vec<_>>();
        let budget = ContextBudget::new(600, 100, 50, 4).unwrap();
        let request = prepare_compaction(&history, budget).expect("compaction required");
        assert_eq!(request.retained, history[16..]);
        assert_eq!(request.prefix, history[..16]);
    }

    #[test]
    fn short_history_is_never_compacted_just_for_item_count() {
        let history = vec![json!({"role": "user", "content": "hello"})];
        assert!(prepare_compaction(&history, ContextBudget::default()).is_none());
    }

    #[test]
    fn compacted_history_places_summary_before_recent_items() {
        let result = CompactionResult {
            summary: json!({"role": "system", "content": "summary"}),
            retained: vec![json!({"role": "user", "content": "latest"})],
        };
        let history = result.into_history();
        assert_eq!(history.len(), 2);
        assert_eq!(history[0]["content"], "summary");
        assert_eq!(history[1]["content"], "latest");
    }

    #[test]
    fn invalid_budget_is_rejected() {
        assert_eq!(
            ContextBudget::new(100, 100, 80, 4),
            Err(ContextError::InvalidBudget)
        );
        assert_eq!(
            ContextBudget::new(100, 10, 0, 4),
            Err(ContextError::InvalidBudget)
        );
    }
}
