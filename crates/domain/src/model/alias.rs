use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AliasName {
    Production,
    Candidate,
    Staging,
    Fallback,
}

impl AliasName {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Production => "production",
            Self::Candidate => "candidate",
            Self::Staging => "staging",
            Self::Fallback => "fallback",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "production" => Some(Self::Production),
            "candidate" => Some(Self::Candidate),
            "staging" => Some(Self::Staging),
            "fallback" => Some(Self::Fallback),
            _ => None,
        }
    }
}
