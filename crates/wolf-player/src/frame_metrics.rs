//! Bounded timing samples, collected only when an explicit report is requested.
use std::{collections::BTreeMap, time::Duration};

#[derive(Default)]
pub struct FrameMetrics {
    samples: BTreeMap<&'static str, Vec<f64>>,
}
impl FrameMetrics {
    pub fn record(&mut self, name: &'static str, duration: Duration) {
        let samples = self.samples.entry(name).or_default();
        if samples.len() < 216_000 {
            samples.push(duration.as_secs_f64() * 1000.0);
        }
    }
    pub fn report(&self) -> serde_json::Value {
        self.samples.iter().map(|(name, values)| {
            let mut sorted = values.clone();
            sorted.sort_by(f64::total_cmp);
            let percentile = |p: usize| sorted[((sorted.len() - 1) * p).div_ceil(100)];
            ((*name).to_owned(), serde_json::json!({
                "count": sorted.len(), "mean_ms": sorted.iter().sum::<f64>() / sorted.len() as f64,
                "p50_ms": percentile(50), "p95_ms": percentile(95), "p99_ms": percentile(99),
                "max_ms": sorted.last(), "above_16_667_ms": sorted.iter().filter(|&&v|v>16.667).count()
            }))
        }).collect::<serde_json::Map<_,_>>().into()
    }
}
