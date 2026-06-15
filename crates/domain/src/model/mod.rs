//! Model registry record types and lifecycle enums.
pub mod alias;
pub mod forecast;
pub mod status;

pub use alias::AliasName;
pub use forecast::{Direction, Forecast};
pub use status::{ModelStatus, RunStatus};
