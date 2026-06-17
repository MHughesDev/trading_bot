//! Model registry record types and lifecycle enums.
pub mod alias;
pub mod forecast;
pub mod status;

pub use alias::AliasName;
pub use forecast::{
    CalibratedForecast, Direction, Forecast, ForecastDistribution, ForecastRisk, RiskAtLevel,
};
pub use status::{ModelStatus, RunStatus};
