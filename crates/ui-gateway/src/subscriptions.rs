//! Subscription registry: manages panel subscriptions with per-lane authorization.
//!
//! Private lanes (orders.*, positions.*, strategy.*) may only be subscribed by
//! the authenticated user that owns them.  Public lanes are shareable.

use std::str::FromStr;
use std::sync::Arc;

use dashmap::DashMap;
use domain::lanes::Lane;
use thiserror::Error;
use uuid::Uuid;

use demand_manager::DemandRegistry;

use crate::shaping::is_private_lane;

/// A registered panel subscription.
#[derive(Debug, Clone)]
pub struct Subscription {
    pub id: Uuid,
    pub panel_id: String,
    pub user_id: String,
    /// The lane string as presented by the panel (may be a virtual UI lane).
    pub lane: String,
    pub instrument: String,
    pub depth: Option<u32>,
    pub max_fps: Option<u32>,
    /// The real NATS lane used for demand tracking.
    demand_lane: Lane,
}

#[derive(Debug, Error)]
pub enum SubscriptionError {
    #[error("unauthorized: user {requesting_user} cannot subscribe to private lane {lane} owned by {owner}")]
    Unauthorized {
        lane: String,
        owner: String,
        requesting_user: String,
    },
    #[error("unknown lane: {0}")]
    UnknownLane(String),
}

/// Registry of active panel subscriptions, backed by the `DemandRegistry`.
pub struct SubscriptionRegistry {
    demand: Arc<DemandRegistry>,
    subs: DashMap<Uuid, Arc<Subscription>>,
}

impl SubscriptionRegistry {
    pub fn new(demand: Arc<DemandRegistry>) -> Self {
        Self {
            demand,
            subs: DashMap::new(),
        }
    }

    /// Register a subscription on behalf of `requesting_user_id`.
    ///
    /// Private lanes may only be subscribed by the user they are scoped to.
    #[allow(clippy::too_many_arguments)]
    pub fn subscribe(
        &self,
        panel_id: &str,
        user_id: &str,
        lane: &str,
        instrument: &str,
        requesting_user_id: &str,
        depth: Option<u32>,
        max_fps: Option<u32>,
    ) -> Result<Arc<Subscription>, SubscriptionError> {
        if is_private_lane(lane) && user_id != requesting_user_id {
            return Err(SubscriptionError::Unauthorized {
                lane: lane.to_owned(),
                owner: user_id.to_owned(),
                requesting_user: requesting_user_id.to_owned(),
            });
        }

        let demand_lane = map_to_nats_lane(lane)
            .ok_or_else(|| SubscriptionError::UnknownLane(lane.to_owned()))?;

        let sub = Arc::new(Subscription {
            id: Uuid::new_v4(),
            panel_id: panel_id.to_owned(),
            user_id: user_id.to_owned(),
            lane: lane.to_owned(),
            instrument: instrument.to_owned(),
            depth,
            max_fps,
            demand_lane: demand_lane.clone(),
        });

        self.demand.add(&demand_lane, instrument);
        self.subs.insert(sub.id, Arc::clone(&sub));
        Ok(sub)
    }

    /// Remove a subscription by its ID.  Returns `true` if it existed.
    pub fn remove(&self, sub_id: Uuid) -> bool {
        if let Some((_, sub)) = self.subs.remove(&sub_id) {
            self.demand.remove(&sub.demand_lane, &sub.instrument);
            true
        } else {
            false
        }
    }

    /// Remove all subscriptions for a user (e.g. on WS disconnect).
    pub fn remove_all_for_user(&self, user_id: &str) {
        let to_remove: Vec<Arc<Subscription>> = self
            .subs
            .iter()
            .filter(|s| s.user_id == user_id)
            .map(|s| Arc::clone(&s))
            .collect();
        for s in &to_remove {
            self.subs.remove(&s.id);
            self.demand.remove(&s.demand_lane, &s.instrument);
        }
    }

    /// Remove all subscriptions for a panel.
    pub fn remove_panel(&self, panel_id: &str, user_id: &str) {
        let to_remove: Vec<Arc<Subscription>> = self
            .subs
            .iter()
            .filter(|s| s.panel_id == panel_id && s.user_id == user_id)
            .map(|s| Arc::clone(&s))
            .collect();
        for s in &to_remove {
            self.subs.remove(&s.id);
            self.demand.remove(&s.demand_lane, &s.instrument);
        }
    }

    /// List all subscriptions for a user.
    pub fn list_for_user(&self, user_id: &str) -> Vec<Arc<Subscription>> {
        self.subs
            .iter()
            .filter(|s| s.user_id == user_id)
            .map(|s| Arc::clone(&s))
            .collect()
    }

    /// Return demand count for a lane/instrument pair (for testing).
    pub fn demand_count(&self, lane_str: &str, instrument: &str) -> u32 {
        if let Some(lane) = map_to_nats_lane(lane_str) {
            self.demand.count(&lane, instrument)
        } else {
            0
        }
    }
}

/// Map a UI-facing lane string to the real NATS `Lane` used for demand tracking.
///
/// `ui.orderbook.snapshot` is a virtual lane served by the gateway; its underlying
/// feed is `market.orderbook.l2`.
fn map_to_nats_lane(lane: &str) -> Option<Lane> {
    if lane == "ui.orderbook.snapshot" {
        return Some(Lane::MarketOrderbookL2);
    }
    Lane::from_str(lane).ok()
}
