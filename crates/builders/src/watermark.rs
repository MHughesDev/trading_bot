use chrono::{DateTime, Duration, Utc};
use domain::{compute_available_time, AvailableTimeParams};

/// Watermark policy controlling when a bar's `available_time` is stamped.
pub struct WatermarkPolicy {
    pub watermark: Duration,
    pub processing_delay: Duration,
}

impl WatermarkPolicy {
    pub fn new(watermark: Duration, processing_delay: Duration) -> Self {
        Self {
            watermark,
            processing_delay,
        }
    }

    /// Typical policy for a liquid CEX stream: 2 s watermark, 50 ms pipeline delay.
    pub fn default_cex() -> Self {
        Self::new(Duration::seconds(2), Duration::milliseconds(50))
    }

    /// Compute `available_time` for a bar whose window closed at `window_close`
    /// and whose last trade was observed at `observed_time`.
    pub fn available_time_for_bar(
        &self,
        window_close: DateTime<Utc>,
        observed_time: DateTime<Utc>,
    ) -> DateTime<Utc> {
        compute_available_time(&AvailableTimeParams {
            window_close,
            observed_time,
            watermark: self.watermark,
            processing_delay: self.processing_delay,
        })
    }

    /// Latest wall-clock time at which a late trade is still accepted for a
    /// window that closed at `window_close`.
    pub fn deadline(&self, window_close: DateTime<Utc>) -> DateTime<Utc> {
        window_close + self.watermark
    }
}
