use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelStatus {
    Draft,
    Training,
    Evaluating,
    Candidate,
    Active,
    Archived,
    Failed,
}

impl ModelStatus {
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Active | Self::Archived | Self::Failed)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Draft => "draft",
            Self::Training => "training",
            Self::Evaluating => "evaluating",
            Self::Candidate => "candidate",
            Self::Active => "active",
            Self::Archived => "archived",
            Self::Failed => "failed",
        }
    }

    pub fn from_str_loose(s: &str) -> Self {
        match s {
            "training" => Self::Training,
            "evaluating" => Self::Evaluating,
            "candidate" => Self::Candidate,
            "active" => Self::Active,
            "archived" => Self::Archived,
            "failed" => Self::Failed,
            _ => Self::Draft,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Queued,
    Running,
    Succeeded,
    Failed,
    Cancelled,
}

impl RunStatus {
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Succeeded | Self::Failed | Self::Cancelled)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Running => "running",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }

    pub fn from_str_loose(s: &str) -> Self {
        match s {
            "running" => Self::Running,
            "succeeded" => Self::Succeeded,
            "failed" => Self::Failed,
            "cancelled" => Self::Cancelled,
            _ => Self::Queued,
        }
    }
}
