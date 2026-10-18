//! Wake a session on the monitor's own notification instead of on its poll
//! (AGENT-014 §7 rows 8–10, §12.6; Set S task S-11.7).
//!
//! # What was missing
//!
//! §7 row 8 says a kill-switch trip pauses every continuous session *within one
//! event*. The driver noticed a trip on its next look: 5 s while a child ran,
//! 30 s while it waited. "Within one event" was "within one poll", and a trip
//! that lands one second after a look takes the whole interval to be seen while a
//! child spends tokens.
//!
//! The monitor has published on `monitor_kill_switch` since 0075 and on
//! `monitor_events` since 0165 (`monitor::store::NOTIFY_CHANNELS`), and the only
//! listeners were the order gate's mirror and the desk's SSE. This is the third:
//! the driver sleeps on the channel and wakes when the monitor says something
//! changed, and what it does on waking is what it already did on every poll — it
//! re-reads the world and lets `execute::plan` decide. No decision moved: only
//! the moment the question is asked.
//!
//! # The poll stays, as the backstop
//!
//! A listener can fail to open, and a notification can be lost across a
//! reconnect. Neither may make the driver *slower than before*: [`Wake::sleep`]
//! takes the old interval as its ceiling, so the worst a dead listener does is
//! reproduce the old behaviour, and [`Woke::Timer`] says that is what happened.
//! The first failure is logged once per [`Wake`], not per sleep.

use std::time::Duration;

use sqlx::postgres::PgListener;
use sqlx::PgPool;

/// Why a [`Wake::sleep`] ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Woke {
    /// The ceiling elapsed. Either nothing happened or nothing was heard.
    Timer,
    /// The monitor published on this channel.
    Event(String),
}

/// A session's ear on the monitor.
pub struct Wake {
    listener: Option<PgListener>,
    /// Why there is no listener, once. `None` while listening.
    degraded: Option<String>,
}

impl Wake {
    /// Open the listener. A failure is recorded and the [`Wake`] still works — as
    /// a timer.
    pub async fn open(pg: &PgPool) -> Self {
        match monitor::store::MonitorStore::new(pg.clone())
            .listener()
            .await
        {
            Ok(l) => Self {
                listener: Some(l),
                degraded: None,
            },
            Err(e) => {
                tracing::warn!(error = %e, "session wake: no LISTEN on the monitor channels; \
                                            falling back to the poll");
                Self {
                    listener: None,
                    degraded: Some(e.to_string()),
                }
            }
        }
    }

    /// A [`Wake`] that never hears anything: the old poll, for a caller that has
    /// no pool to listen on.
    #[must_use]
    pub fn timer_only() -> Self {
        Self {
            listener: None,
            degraded: Some("timer only".into()),
        }
    }

    /// Whether the push half is working.
    #[must_use]
    pub fn is_listening(&self) -> bool {
        self.listener.is_some()
    }

    /// Why the push half is not working, if it is not.
    #[must_use]
    pub fn degraded(&self) -> Option<&str> {
        self.degraded.as_deref()
    }

    /// Sleep for at most `ceiling`, and less when the monitor says something
    /// changed.
    ///
    /// `monitor_lease` is the monitor's own heartbeat and is ignored — it fires
    /// on every renewal and is not a change in the world — so the sleep carries
    /// on for what remains of the ceiling.
    pub async fn sleep(&mut self, ceiling: Duration) -> Woke {
        let deadline = tokio::time::Instant::now() + ceiling;
        loop {
            let Some(listener) = self.listener.as_mut() else {
                tokio::time::sleep_until(deadline).await;
                return Woke::Timer;
            };
            match tokio::time::timeout_at(deadline, listener.recv()).await {
                Err(_) => return Woke::Timer,
                Ok(Ok(n)) if is_a_change(n.channel()) => {
                    return Woke::Event(n.channel().to_string())
                }
                Ok(Ok(_)) => {}
                Ok(Err(e)) => {
                    // The connection is gone and sqlx could not bring it back.
                    // Degrade to the timer for the rest of this wait.
                    tracing::warn!(error = %e, "session wake: the monitor listener died; polling");
                    self.degraded = Some(e.to_string());
                    self.listener = None;
                }
            }
        }
    }
}

/// Whether a notification on `channel` is the world changing.
///
/// The kill switch and the event log are; the lease heartbeat is not.
#[must_use]
pub fn is_a_change(channel: &str) -> bool {
    matches!(channel, "monitor_kill_switch" | "monitor_events")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_switch_and_the_event_log_wake_a_session_and_the_heartbeat_does_not() {
        assert!(is_a_change("monitor_kill_switch"));
        assert!(is_a_change("monitor_events"));
        assert!(!is_a_change("monitor_lease"));
    }

    #[test]
    fn every_channel_the_monitor_publishes_is_classified_here() {
        // A channel added to the monitor must be decided about: it either wakes a
        // session or it is a heartbeat. Left unclassified it would be a change the
        // driver sleeps through.
        for c in monitor::store::NOTIFY_CHANNELS {
            let known = is_a_change(c) || c == "monitor_lease";
            assert!(
                known,
                "`{c}` is published by the monitor and this module has no opinion"
            );
        }
    }

    #[tokio::test]
    async fn a_timer_only_wake_waits_the_ceiling_and_says_it_was_a_timer() {
        let mut w = Wake::timer_only();
        assert!(!w.is_listening());
        let started = std::time::Instant::now();
        assert_eq!(w.sleep(Duration::from_millis(30)).await, Woke::Timer);
        assert!(started.elapsed() >= Duration::from_millis(25));
    }
}
