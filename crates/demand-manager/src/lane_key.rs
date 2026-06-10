//! `LaneKey` — the four-component demand key for collector pipelines.

use domain::{AssetClass, DataType, SupportedVenue};

/// Unique identifier for a collector demand lane.
///
/// One lane key maps to exactly one running collector pipeline (shared across
/// all consumers that reference it).  Equal keys share the same pipeline.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct LaneKey {
    pub venue: SupportedVenue,
    pub asset_class: AssetClass,
    pub data_type: DataType,
    pub instrument_id: String,
}

impl LaneKey {
    pub fn new(
        venue: SupportedVenue,
        asset_class: AssetClass,
        data_type: DataType,
        instrument_id: impl Into<String>,
    ) -> Self {
        Self {
            venue,
            asset_class,
            data_type,
            instrument_id: instrument_id.into(),
        }
    }

    /// Stable string representation — used in log fields.
    pub fn as_display(&self) -> String {
        format!(
            "{}.{}.{}.{}",
            self.venue.as_str(),
            format!("{:?}", self.asset_class).to_lowercase(),
            self.data_type.as_key(),
            self.instrument_id
        )
    }
}

impl std::fmt::Display for LaneKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_display())
    }
}
