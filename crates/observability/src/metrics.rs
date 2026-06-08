/// Lightweight metric handles — Phase 1 stubs.
///
/// Phase 2 wires these to a real Prometheus registry.
#[derive(Default)]
pub struct Metrics;

impl Metrics {
    pub fn new() -> Self {
        Self
    }

    pub fn increment_published(&self, _lane: &str) {}

    pub fn increment_quarantined(&self, _source: &str) {}

    pub fn increment_gap_detected(&self, _lane: &str) {}
}
