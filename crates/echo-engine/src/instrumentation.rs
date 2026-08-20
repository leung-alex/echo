use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OperationMetric {
    pub operation: String,
    pub samples: u64,
    pub total_duration_us: u64,
    pub total_count: u64,
}

#[derive(Clone, Default)]
pub struct OperationMetrics {
    values: Arc<Mutex<BTreeMap<String, OperationMetric>>>,
}

impl OperationMetrics {
    pub fn record(&self, operation: &'static str, duration: Duration, count: u64) {
        let duration_us = duration.as_micros().min(u128::from(u64::MAX)) as u64;
        let mut values = self
            .values
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let metric = values
            .entry(operation.to_owned())
            .or_insert_with(|| OperationMetric {
                operation: operation.to_owned(),
                samples: 0,
                total_duration_us: 0,
                total_count: 0,
            });
        metric.samples = metric.samples.saturating_add(1);
        metric.total_duration_us = metric.total_duration_us.saturating_add(duration_us);
        metric.total_count = metric.total_count.saturating_add(count);
    }

    pub fn snapshot(&self) -> Vec<OperationMetric> {
        self.values
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .values()
            .cloned()
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metrics_are_structured_and_do_not_store_operation_inputs() {
        let metrics = OperationMetrics::default();
        metrics.record("history_query", Duration::from_micros(12), 3);
        let snapshot = metrics.snapshot();
        assert_eq!(snapshot[0].operation, "history_query");
        assert_eq!(snapshot[0].samples, 1);
        assert_eq!(snapshot[0].total_count, 3);
        let encoded = serde_json::to_string(&snapshot).unwrap();
        assert!(!encoded.contains("clipboard payload"));
    }
}
