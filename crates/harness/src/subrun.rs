//! Sub-runs: a run with a parent, a role, and a projection (AGENT-010 §9;
//! Set S task S-10.11).
//!
//! # What a role actually constrains
//!
//! §9 gives three roles and describes each as a charter tail and a catalogue
//! slice. Both of those are prose handed to a model, and a model asked to ignore
//! what it can see is being asked rather than prevented. The part that can be
//! made structural is made structural here:
//!
//! * a **critic** reads the evidence bundle. Its projection is
//!   [`Projection::Critic`](crate::context::projection::Projection::Critic), so
//!   the parent's reasoning is not filtered out of its prompt — it is not in the
//!   store the critic selects from;
//! * a **worker** reads a pinned set, identical for every worker in a fan-out,
//!   so the provider cache sees one prefix rather than one per worker;
//! * a **reader** has no tools and summarises one handle. Nothing about the
//!   parent's store reaches it at all.
//!
//! [`Delegation::check`] is what stops a role and a projection disagreeing. A
//! critic dispatched under `full` would be a critic that saw the reasoning it
//! exists to be independent of — and the failure would be invisible, because the
//! sub-run would run, return a verdict, and record the word "critic" beside it.
//!
//! # Why this is here rather than in the driver
//!
//! The driver performs the dispatch; it is not the place the rule lives. A rule
//! in the driver is a rule the second driver does not have, and this platform has
//! had two agent drivers before.

use serde::{Deserialize, Serialize};

/// What a sub-run is for (AGENT-010 §9).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    /// Judges the evidence without the reasoning that produced it.
    Critic,
    /// Proposes, from a brief identical to every other worker's.
    Worker,
    /// Summarises one handle into a bounded number of tokens. No tools.
    Reader,
}

impl Role {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Role::Critic => "critic",
            Role::Worker => "worker",
            Role::Reader => "reader",
        }
    }

    /// Every role, so a per-role table cannot quietly omit one.
    pub const ALL: &'static [Role] = &[Role::Critic, Role::Worker, Role::Reader];

    /// The projection this role is only meaningful under.
    ///
    /// `None` for a reader: it is handed a document, not a view of the parent's
    /// store, so there is no projection to constrain — and returning a name
    /// there would invent a relationship that does not exist.
    #[must_use]
    pub fn requires_projection(self) -> Option<&'static str> {
        match self {
            Role::Critic => Some(crate::context::projection::Projection::Critic.as_str()),
            Role::Worker => Some("worker"),
            Role::Reader => None,
        }
    }

    /// Whether this role may call tools at all.
    ///
    /// A reader may not. §9 is specific about it, and the reason is the budget:
    /// a reader exists so a large handle can be summarised without the parent
    /// paying for the whole handle, and a reader that could fetch would spend
    /// more than it saved.
    #[must_use]
    pub fn allows_tools(self) -> bool {
        !matches!(self, Role::Reader)
    }
}

/// Why a dispatch was refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DelegationError {
    #[error(
        "a `{role}` sub-run was dispatched under the `{used}` projection, and it must be \
         `{required}`: the role's independence is the projection, and a role that saw more \
         than its projection allows is one the recorded role now describes wrongly"
    )]
    WrongProjection {
        role: &'static str,
        used: String,
        required: &'static str,
    },
    #[error(
        "a `reader` sub-run was given a projection of the parent's store; a reader summarises \
         one handle and has no view of the parent at all"
    )]
    ReaderWithProjection,
    #[error("a sub-run was dispatched with an empty brief: nothing was delegated")]
    EmptyBrief,
    #[error(
        "the depth of this sub-run is {depth}, above the profile's max_subagent_depth of \
         {max}: a sub-run that may delegate is a tree, and a tree has no budget"
    )]
    TooDeep { depth: u32, max: u32 },
}

/// One dispatch, and the facts a later reader needs to check it.
///
/// Recorded on both sides — §9's `Delegation` trace event — so "what did the
/// critic see" has an answer that does not depend on asking the critic.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Delegation {
    pub role: Role,
    /// The projection name, from
    /// [`Projection::as_str`](crate::context::projection::Projection::as_str).
    pub projection: Option<String>,
    /// A digest of the brief's bytes. Two workers whose digests differ did not
    /// share a prefix, whatever the fan-out intended.
    pub brief_digest: String,
    /// Records the parent had that this sub-run does not.
    pub withheld: usize,
    /// One for a sub-run of a top-level run.
    pub depth: u32,
}

impl Delegation {
    /// The checks that must hold before a sub-run is dispatched.
    ///
    /// # Errors
    /// [`DelegationError`] when the role and the projection disagree, when a
    /// reader is handed a projection, when the brief is empty, or when the
    /// dispatch would exceed the profile's depth.
    pub fn check(&self, max_depth: u32) -> Result<(), DelegationError> {
        if self.brief_digest.trim().is_empty() {
            return Err(DelegationError::EmptyBrief);
        }
        if self.depth > max_depth {
            return Err(DelegationError::TooDeep {
                depth: self.depth,
                max: max_depth,
            });
        }
        match (self.role.requires_projection(), self.projection.as_deref()) {
            (None, Some(_)) => Err(DelegationError::ReaderWithProjection),
            (None, None) => Ok(()),
            (Some(required), used) => {
                if used == Some(required) {
                    Ok(())
                } else {
                    Err(DelegationError::WrongProjection {
                        role: self.role.as_str(),
                        used: used.unwrap_or("none").to_string(),
                        required,
                    })
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn worker(projection: Option<&str>) -> Delegation {
        Delegation {
            role: Role::Worker,
            projection: projection.map(str::to_string),
            brief_digest: "abc123".into(),
            withheld: 3,
            depth: 1,
        }
    }

    #[test]
    fn a_worker_under_its_own_projection_is_admitted() {
        assert_eq!(worker(Some("worker")).check(1), Ok(()));
    }

    /// The failure this type exists for. A critic under `full` would still run,
    /// still return a verdict, and still be recorded as a critic — and the only
    /// evidence that it had been led is a column nobody reads.
    #[test]
    fn a_critic_under_the_full_projection_is_refused() {
        let d = Delegation {
            role: Role::Critic,
            projection: Some("full".into()),
            brief_digest: "abc123".into(),
            withheld: 0,
            depth: 1,
        };
        assert_eq!(
            d.check(1),
            Err(DelegationError::WrongProjection {
                role: "critic",
                used: "full".into(),
                required: "critic",
            })
        );
    }

    #[test]
    fn a_worker_with_no_projection_at_all_is_refused() {
        assert!(matches!(
            worker(None).check(1),
            Err(DelegationError::WrongProjection { .. })
        ));
    }

    /// A reader summarises a handle. Handing it a view of the parent's store
    /// would make it a second, cheaper critic — with none of a critic's
    /// guarantees and a name that says it has them.
    #[test]
    fn a_reader_handed_a_projection_is_refused() {
        let d = Delegation {
            role: Role::Reader,
            projection: Some("full".into()),
            brief_digest: "abc123".into(),
            withheld: 0,
            depth: 1,
        };
        assert_eq!(d.check(1), Err(DelegationError::ReaderWithProjection));
        let ok = Delegation {
            projection: None,
            ..d
        };
        assert_eq!(ok.check(1), Ok(()));
    }

    #[test]
    fn an_empty_brief_is_refused_before_a_model_is_called() {
        let mut d = worker(Some("worker"));
        d.brief_digest = "   ".into();
        assert_eq!(d.check(1), Err(DelegationError::EmptyBrief));
    }

    /// `max_subagent_depth` is 1 on every shipped profile. A sub-run that could
    /// delegate would make the budget a tree's, and the lease is split from the
    /// parent's remaining budget exactly once.
    #[test]
    fn a_sub_run_may_not_delegate_past_the_profiles_depth() {
        let mut d = worker(Some("worker"));
        d.depth = 2;
        assert_eq!(
            d.check(1),
            Err(DelegationError::TooDeep { depth: 2, max: 1 })
        );
    }

    /// Every role names a projection or explains why it has none, and the names
    /// it gives are names the projection type actually produces.
    #[test]
    fn every_roles_projection_is_one_the_projection_type_emits() {
        use crate::context::projection::Projection;
        let emitted = [
            Projection::Critic.as_str(),
            Projection::worker([]).as_str(),
            Projection::Full.as_str(),
        ];
        for role in Role::ALL {
            if let Some(name) = role.requires_projection() {
                assert!(
                    emitted.contains(&name),
                    "{} requires `{name}`, which no Projection produces",
                    role.as_str()
                );
            }
        }
    }
}
