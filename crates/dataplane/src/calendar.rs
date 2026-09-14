//! Pinned trading calendars (SPEC §2 rule 3). Calendars get revised retroactively,
//! so the version is part of every dataset hash.

use chrono::{DateTime, Datelike, NaiveDate, NaiveTime, Utc, Weekday};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CalendarVersion {
    pub calendar_id: String,
    pub version: String,
}

impl CalendarVersion {
    #[must_use]
    pub fn new(calendar_id: impl Into<String>, version: impl Into<String>) -> Self {
        Self { calendar_id: calendar_id.into(), version: version.into() }
    }

    #[must_use]
    pub fn crypto_24_7() -> Self {
        Self::new("continuous_24_7", "1")
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Calendar {
    Continuous { version: CalendarVersion },
    Sessions {
        version: CalendarVersion,
        /// UTC session window per trading day.
        open_utc: NaiveTime,
        close_utc: NaiveTime,
        holidays: Vec<NaiveDate>,
    },
}

impl Calendar {
    #[must_use]
    pub fn version(&self) -> &CalendarVersion {
        match self {
            Self::Continuous { version } | Self::Sessions { version, .. } => version,
        }
    }

    /// Sessions are attributes of an observation, never the index.
    #[must_use]
    pub fn is_open(&self, t: DateTime<Utc>) -> bool {
        match self {
            Self::Continuous { .. } => true,
            Self::Sessions { open_utc, close_utc, holidays, .. } => {
                let d = t.date_naive();
                !matches!(d.weekday(), Weekday::Sat | Weekday::Sun)
                    && !holidays.contains(&d)
                    && t.time() >= *open_utc
                    && t.time() < *close_utc
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn holiday_is_closed_crypto_is_not() {
        let nyse = Calendar::Sessions {
            version: CalendarVersion::new("XNYS", "2026.1"),
            open_utc: NaiveTime::from_hms_opt(14, 30, 0).unwrap(),
            close_utc: NaiveTime::from_hms_opt(21, 0, 0).unwrap(),
            holidays: vec![NaiveDate::from_ymd_opt(2026, 1, 19).unwrap()],
        };
        let mlk = Utc.with_ymd_and_hms(2026, 1, 19, 15, 0, 0).unwrap();
        let sat = Utc.with_ymd_and_hms(2026, 1, 17, 15, 0, 0).unwrap();
        assert!(!nyse.is_open(mlk));
        assert!(!nyse.is_open(sat));
        let crypto = Calendar::Continuous { version: CalendarVersion::crypto_24_7() };
        assert!(crypto.is_open(sat));
    }
}
