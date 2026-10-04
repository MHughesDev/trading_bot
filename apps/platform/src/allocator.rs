//! The allocator: a trading strategy deployed on whatever a selection picks
//! (Set U U-6.4; FEAT-007 §4; ADR-0040 decision 4).
//!
//! One allocator per selection-bound deployment. On each rebalance it runs the
//! selection (journaled, as asked by the deployment), and applies
//! [`strategy_engine::allocation::rebalance`] — the function the portfolio
//! backtest applies — to what the deployment holds:
//!
//! - an **entrant** becomes a member: a child deployment of the trading strategy
//!   on that instrument, recorded and declared like any other, carrying a
//!   [`SleeveHandle`] with its share of the capital — the live-runner supervisor
//!   starts it within a second;
//! - a **leaver** is told how it leaves through its handle, and its runner — the
//!   only thing that owns its orders — carries it out on its next bar: `close`
//!   flattens through the runner's own gate, `hold` stops deciding,
//!   `let_strategy_exit` keeps deciding until its own rules have it flat;
//! - a **stayer** has its capital moved on its handle.
//!
//! Every change is journaled (`deployment_allocation_event`, 0148) with the
//! capital it carried and the selection run that caused it. Weights sum to one,
//! so the members' capital sums to the deployment's.
//!
//! The allocator talks to the world through three ports, so its logic is tested
//! without a database: [`SelectionPort`], [`MemberPort`], [`AllocationJournal`].

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use domain::strategy_graph::deployment::{OnRemoval, Rebalance};
use domain::strategy_graph::selection::Selection;
use rust_decimal::Decimal;
use strategy_engine::allocation::{self, Allocation, AllocationChange, Sleeve};
use strategy_engine::schedule::Cron;
use strategy_runtime::live::{
    LiveStrategyRegistry, MemberSelection, SelectionBinding, SleeveHandle, V3Deployment,
};
use tracing::{error, info, warn};
use uuid::Uuid;

/// Runs the deployment's selection.
#[async_trait]
pub trait SelectionPort: Send + Sync {
    /// The selection as of `at`, the journal id of the run, and the selection
    /// strategy's name (what `selection_attr` names it by, U-6.6).
    async fn select(
        &self,
        revision: &str,
        at: DateTime<Utc>,
    ) -> Result<(Selection, Option<Uuid>, String), String>;
}

/// Starts members and reports whether they still run.
#[async_trait]
pub trait MemberPort: Send + Sync {
    /// Record a member's child deployment and declare it with its sleeve.
    async fn start(
        &self,
        b: &SelectionBinding,
        instrument: &str,
        sleeve: Arc<SleeveHandle>,
    ) -> Result<Uuid, String>;
    /// Whether the member's runner is still declared.
    fn running(&self, child: Uuid) -> bool;
}

/// Where allocation changes are written.
#[async_trait]
pub trait AllocationJournal: Send + Sync {
    async fn record(
        &self,
        e: storage::postgres::deployments::AllocationEvent,
    ) -> Result<(), String>;
}

/// One member: its child deployment and the handle shared with its runner.
#[derive(Clone, Debug)]
pub struct Member {
    pub child: Uuid,
    pub sleeve: Arc<SleeveHandle>,
}

/// One selection-bound deployment's allocation, and the members holding it.
pub struct Allocator {
    b: Arc<SelectionBinding>,
    alloc: Allocation,
    members: BTreeMap<String, Member>,
}

fn change_kind(c: &AllocationChange) -> &'static str {
    match c {
        AllocationChange::Entered { .. } => "entered",
        AllocationChange::Left { .. } => "left",
        AllocationChange::Exiting { .. } => "exiting",
        AllocationChange::Reweighted { .. } => "reweighted",
    }
}

fn action_name(a: OnRemoval) -> &'static str {
    match a {
        OnRemoval::Close => "close",
        OnRemoval::LetStrategyExit => "let_strategy_exit",
        OnRemoval::Hold => "hold",
    }
}

impl Allocator {
    #[must_use]
    pub fn new(b: Arc<SelectionBinding>) -> Self {
        Self {
            b,
            alloc: Allocation::new(),
            members: BTreeMap::new(),
        }
    }

    /// An allocator picking up members that were already running (a restart).
    #[must_use]
    pub fn resumed(b: Arc<SelectionBinding>, members: Vec<(String, Member, Sleeve)>) -> Self {
        let mut a = Self::new(b);
        for (instrument, member, sleeve) in members {
            a.alloc.insert(instrument.clone(), sleeve);
            a.members.insert(instrument, member);
        }
        a
    }

    #[must_use]
    pub const fn allocation(&self) -> &Allocation {
        &self.alloc
    }

    #[must_use]
    pub const fn members(&self) -> &BTreeMap<String, Member> {
        &self.members
    }

    async fn journal(
        &self,
        journal: &dyn AllocationJournal,
        at: DateTime<Utc>,
        run: Option<Uuid>,
        change: &AllocationChange,
        child: Option<Uuid>,
    ) {
        let (weight, action) = match change {
            AllocationChange::Entered { weight, .. } | AllocationChange::Exiting { weight, .. } => {
                (*weight, None)
            }
            AllocationChange::Reweighted { to, .. } => (*to, None),
            AllocationChange::Left { action, .. } => {
                (Decimal::ZERO, Some(action_name(*action).to_string()))
            }
        };
        let e = storage::postgres::deployments::AllocationEvent {
            deployment_id: self.b.deployment_id,
            at,
            selection_run_id: run,
            change: change_kind(change).into(),
            instrument_id: change.instrument().to_string(),
            member_deployment_id: child,
            weight,
            capital: weight * self.b.capital,
            action,
        };
        if let Err(err) = journal.record(e).await {
            // The change happened; failing to write it down is loud, not fatal
            // to the members already trading.
            error!(deployment = %self.b.deployment_id, error = %err, "allocation change not journaled");
        }
    }

    /// Run the selection as of `at` and apply it. Returns the changes made.
    ///
    /// # Errors
    /// The selection could not be run; nothing changes.
    pub async fn rebalance(
        &mut self,
        at: DateTime<Utc>,
        selection: &dyn SelectionPort,
        members: &dyn MemberPort,
        journal: &dyn AllocationJournal,
    ) -> Result<Vec<AllocationChange>, String> {
        let (picked, run, name) = selection.select(&self.b.selection, at).await?;
        let rb = allocation::rebalance(
            &self.alloc,
            &picked,
            self.b.top_n,
            self.b.on_removal,
            &self.b.weighting,
        );
        let mut next = rb.next;
        let mut applied = Vec::with_capacity(rb.changes.len());
        for change in rb.changes {
            let child = match &change {
                AllocationChange::Entered { instrument, weight } => {
                    let sleeve = Arc::new(SleeveHandle::new(
                        self.b.deployment_id,
                        self.b.capital * *weight,
                    ));
                    match members
                        .start(&self.b, instrument, Arc::clone(&sleeve))
                        .await
                    {
                        Ok(child) => {
                            self.members
                                .insert(instrument.clone(), Member { child, sleeve });
                            Some(child)
                        }
                        Err(e) => {
                            // Not started, so not a member: its capital waits
                            // for the next rebalance.
                            warn!(deployment = %self.b.deployment_id, instrument, error = %e, "member could not start");
                            next.remove(instrument);
                            continue;
                        }
                    }
                }
                AllocationChange::Left { instrument, action } => {
                    let m = self.members.remove(instrument);
                    if let Some(m) = &m {
                        m.sleeve.retire(*action);
                    }
                    m.map(|m| m.child)
                }
                AllocationChange::Exiting { instrument, .. } => {
                    self.members.get(instrument).map(|m| {
                        m.sleeve.retire(OnRemoval::LetStrategyExit);
                        m.child
                    })
                }
                AllocationChange::Reweighted { instrument, to, .. } => {
                    self.members.get(instrument).map(|m| {
                        m.sleeve.set_capital(self.b.capital * *to);
                        // Picked again while exiting: it stays.
                        m.sleeve.reinstate();
                        m.child
                    })
                }
            };
            self.journal(journal, at, run, &change, child).await;
            applied.push(change);
        }
        self.alloc = next;
        // Every member reads what this selection said about it (U-6.6).
        for (instrument, m) in &self.members {
            m.sleeve.set_selection(MemberSelection {
                name: name.clone(),
                revision: picked.revision.clone(),
                attributes: allocation::member_attributes(&picked, instrument),
            });
        }
        Ok(applied)
    }

    /// Members whose runner stopped. An exiting member that stopped finished
    /// its exit and releases its weight; a member that stopped for any other
    /// reason (its runner could not start, or halted) is dropped as held — its
    /// position, if any, is left as it is. Returns the changes made.
    pub async fn reap(
        &mut self,
        at: DateTime<Utc>,
        members: &dyn MemberPort,
        journal: &dyn AllocationJournal,
    ) -> Vec<AllocationChange> {
        let stopped: Vec<(String, Member)> = self
            .members
            .iter()
            .filter(|(_, m)| !members.running(m.child))
            .map(|(k, m)| (k.clone(), m.clone()))
            .collect();
        let mut out = Vec::new();
        for (instrument, m) in stopped {
            self.members.remove(&instrument);
            let change = match allocation::finish_exit(&mut self.alloc, &instrument) {
                Some(c) => c,
                None => {
                    self.alloc.remove(&instrument);
                    AllocationChange::Left {
                        instrument: instrument.clone(),
                        action: OnRemoval::Hold,
                    }
                }
            };
            self.journal(journal, at, None, &change, Some(m.child))
                .await;
            out.push(change);
        }
        out
    }

    /// The deployment is stopping: every member stops deciding and keeps its
    /// position, as stopping a hand-bound deployment does.
    pub async fn stop_all(&mut self, at: DateTime<Utc>, journal: &dyn AllocationJournal) {
        let all: Vec<(String, Member)> = std::mem::take(&mut self.members).into_iter().collect();
        for (instrument, m) in all {
            m.sleeve.retire(OnRemoval::Hold);
            let change = AllocationChange::Left {
                instrument,
                action: OnRemoval::Hold,
            };
            self.journal(journal, at, None, &change, Some(m.child))
                .await;
        }
        self.alloc.clear();
    }
}

/// When the next rebalance is due after `after_ns`: on the deployment's own
/// schedule if it has one, else whenever the selection runs — on its own
/// schedule, or at the close of its timeframe.
#[must_use]
pub fn next_due(
    rebalance: &Rebalance,
    selection_schedule: Option<&str>,
    selection_timeframe_secs: u32,
    after_ns: i64,
) -> Option<i64> {
    match rebalance {
        Rebalance::Schedule { cron } => Cron::parse(cron).ok()?.next_after(after_ns),
        Rebalance::OnSelectionRun => match selection_schedule {
            Some(s) => Cron::parse(s).ok()?.next_after(after_ns),
            None => {
                let tf = i64::from(selection_timeframe_secs) * 1_000_000_000;
                Some((after_ns.div_euclid(tf) + 1) * tf)
            }
        },
    }
}

// ── the production ports ─────────────────────────────────────────────────────

/// Runs the selection through the one selection path and journals it as asked
/// by the deployment.
pub struct PgSelections {
    pub pg: sqlx::PgPool,
    pub ch_url: String,
    pub models: Option<backtest::runner::ModelsFactory>,
}

#[async_trait]
impl SelectionPort for PgSelections {
    async fn select(
        &self,
        revision: &str,
        at: DateTime<Utc>,
    ) -> Result<(Selection, Option<Uuid>, String), String> {
        let name = backtest::selection_run::load_selection(&self.pg, revision)
            .await
            .map(|(_, plan, _)| plan.name.clone())
            .map_err(|e| e.to_string())?;
        let report = backtest::selection_run::run_selection(
            &self.pg,
            &self.ch_url,
            revision,
            at,
            self.models.as_ref(),
        )
        .await
        .map_err(|e| e.to_string())?;
        let run = backtest::selection_run::journal(
            &self.pg,
            &report,
            backtest::selection_run::RequestedBy::Deployment,
        )
        .await
        .map_err(|e| e.to_string())?;
        Ok((report.run.selection, Some(run), name))
    }
}

/// Records members as child deployments and declares them in the live
/// registry, where the live-runner supervisor runs them.
pub struct PgMembers {
    pub pg: sqlx::PgPool,
    pub registry: Arc<LiveStrategyRegistry>,
}

#[async_trait]
impl MemberPort for PgMembers {
    async fn start(
        &self,
        b: &SelectionBinding,
        instrument: &str,
        sleeve: Arc<SleeveHandle>,
    ) -> Result<Uuid, String> {
        let child = Uuid::new_v4();
        let record = storage::postgres::deployments::NewDeployment {
            deployment_id: child,
            user_id: b.user_id.clone(),
            strategy_name: b.trading.name.clone(),
            revision_hash: b.trading_revision.clone(),
            definition_json: serde_json::to_value(&b.trading).unwrap_or_default(),
            instrument_id: instrument.to_string(),
            starting_capital: sleeve.capital(),
            model_pins: serde_json::json!([]),
            started_by: format!("allocator:{}", b.deployment_id),
            forward_deployment_id: None,
            target: None,
            parent_deployment_id: Some(b.deployment_id),
        };
        storage::postgres::deployments::start(&self.pg, &record)
            .await
            .map_err(|e| e.to_string())?;
        let declared = self.registry.register_v3(V3Deployment {
            deployment_id: child,
            user_id: b.user_id.clone(),
            instrument_id: instrument.to_string(),
            graph: b.trading.clone(),
            revision: b.trading_revision.clone(),
            starting_capital: sleeve.capital(),
            sleeve: Some(sleeve),
        });
        if let Err(e) = declared {
            let _ =
                storage::postgres::deployments::stop(&self.pg, child, &format!("not started: {e}"))
                    .await;
            return Err(e.to_string());
        }
        Ok(child)
    }

    fn running(&self, child: Uuid) -> bool {
        self.registry.is_v3_declared(child)
    }
}

pub struct PgAllocations(pub sqlx::PgPool);

#[async_trait]
impl AllocationJournal for PgAllocations {
    async fn record(
        &self,
        e: storage::postgres::deployments::AllocationEvent,
    ) -> Result<(), String> {
        storage::postgres::deployments::record_allocation(&self.0, &e)
            .await
            .map(|_| ())
            .map_err(|e| e.to_string())
    }
}

/// How long after a boundary the rebalance waits, so the bar that closed on it
/// has reached the store.
const GRACE: Duration = Duration::from_secs(90);

/// Start the allocators' supervisor: one allocator task per declared
/// selection-bound deployment, resumed at boot from the record.
pub fn spawn(
    pg: sqlx::PgPool,
    ch_url: String,
    models: Option<backtest::runner::ModelsFactory>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let registry = strategy_runtime::live::global();
        resume_recorded(&registry, &pg).await;
        let mut running: BTreeMap<Uuid, tokio::task::JoinHandle<()>> = BTreeMap::new();
        let mut seen = u64::MAX;
        loop {
            running.retain(|_, h| !h.is_finished());
            let generation = registry.selection_generation();
            if generation != seen {
                seen = generation;
                for b in registry.selection_snapshot() {
                    if running.contains_key(&b.deployment_id) {
                        continue;
                    }
                    let task = tokio::spawn(run_one(
                        Arc::clone(&b),
                        pg.clone(),
                        ch_url.clone(),
                        models.clone(),
                        Arc::clone(&registry),
                    ));
                    running.insert(b.deployment_id, task);
                }
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    })
}

/// Declare again every selection-bound deployment the record says is running.
async fn resume_recorded(registry: &LiveStrategyRegistry, pg: &sqlx::PgPool) {
    let rows = match storage::postgres::deployments::running(pg).await {
        Ok(r) => r,
        Err(e) => {
            error!(error = %e, "could not read running deployments; no selection-bound deployment resumed");
            return;
        }
    };
    for row in rows.into_iter().filter(|r| r.target.is_some()) {
        match binding_of(&row) {
            Ok(b) => {
                if let Err(e) = registry.register_selection(b) {
                    warn!(deployment = %row.deployment_id, error = %e, "selection-bound deployment not resumed");
                }
            }
            Err(e) => {
                error!(deployment = %row.deployment_id, error = %e, "recorded selection-bound deployment does not parse; closing it");
                let _ = storage::postgres::deployments::stop(
                    pg,
                    row.deployment_id,
                    &format!("does not parse at resume: {e}"),
                )
                .await;
            }
        }
    }
}

/// The binding a parent row records.
///
/// # Errors
/// A graph or target that does not parse.
pub fn binding_of(
    row: &storage::postgres::deployments::DeploymentRow,
) -> Result<SelectionBinding, String> {
    use domain::strategy_graph::deployment::DeploymentTarget;
    let trading = serde_json::from_value(row.definition_json.clone()).map_err(|e| e.to_string())?;
    let target: DeploymentTarget = serde_json::from_value(row.target.clone().unwrap_or_default())
        .map_err(|e| e.to_string())?;
    let DeploymentTarget::Selection {
        selection,
        top_n,
        rebalance,
        on_removal,
        weighting,
    } = target
    else {
        return Err("the target is not a selection".into());
    };
    Ok(SelectionBinding {
        deployment_id: row.deployment_id,
        user_id: row.user_id.clone(),
        trading,
        trading_revision: row.revision_hash.clone(),
        selection,
        top_n,
        rebalance,
        on_removal,
        weighting,
        capital: row.starting_capital,
    })
}

/// Pick up the members a restart left running: re-declare each with a handle
/// at the capital the journal last gave it.
async fn resume_members(
    b: &Arc<SelectionBinding>,
    pg: &sqlx::PgPool,
    registry: &LiveStrategyRegistry,
) -> Allocator {
    let children = storage::postgres::deployments::running_children(pg, b.deployment_id)
        .await
        .unwrap_or_default();
    let latest: BTreeMap<String, (Decimal, Decimal, String)> =
        storage::postgres::deployments::latest_allocation(pg, b.deployment_id)
            .await
            .unwrap_or_default()
            .into_iter()
            .map(|(m, w, c, k)| (m, (w, c, k)))
            .collect();
    let mut members = Vec::new();
    for row in children {
        let (weight, capital, kind) = latest.get(&row.instrument_id).cloned().unwrap_or((
            Decimal::ZERO,
            row.starting_capital,
            "entered".into(),
        ));
        let sleeve = Arc::new(SleeveHandle::new(b.deployment_id, capital));
        let exiting = kind == "exiting";
        if exiting {
            sleeve.retire(OnRemoval::LetStrategyExit);
        }
        let declared = registry.register_v3(V3Deployment {
            deployment_id: row.deployment_id,
            user_id: row.user_id.clone(),
            instrument_id: row.instrument_id.clone(),
            graph: b.trading.clone(),
            revision: row.revision_hash.clone(),
            starting_capital: row.starting_capital,
            sleeve: Some(Arc::clone(&sleeve)),
        });
        if let Err(e) = declared {
            warn!(member = %row.deployment_id, error = %e, "member not resumed");
            continue;
        }
        members.push((
            row.instrument_id.clone(),
            Member {
                child: row.deployment_id,
                sleeve,
            },
            Sleeve { weight, exiting },
        ));
    }
    Allocator::resumed(Arc::clone(b), members)
}

/// One selection-bound deployment, until it is withdrawn.
async fn run_one(
    b: Arc<SelectionBinding>,
    pg: sqlx::PgPool,
    ch_url: String,
    models: Option<backtest::runner::ModelsFactory>,
    registry: Arc<LiveStrategyRegistry>,
) {
    let id = b.deployment_id;
    let selections = PgSelections {
        pg: pg.clone(),
        ch_url,
        models,
    };
    let members = PgMembers {
        pg: pg.clone(),
        registry: Arc::clone(&registry),
    };
    let journal = PgAllocations(pg.clone());
    let (schedule, tf) = match backtest::selection_run::load_selection(&pg, &b.selection).await {
        Ok((g, plan, _)) => (g.triggers.schedule.clone(), plan.timeframe.seconds()),
        Err(e) => {
            error!(deployment = %id, error = %e, "selection does not load; deployment stopped");
            let _ = storage::postgres::deployments::stop(
                &pg,
                id,
                &format!("selection does not load: {e}"),
            )
            .await;
            registry.deregister_selection(id);
            return;
        }
    };
    let mut allocator = resume_members(&b, &pg, &registry).await;
    info!(deployment = %id, members = allocator.members().len(), "selection-bound deployment running on paper");
    // A deployment starts trading at once, not at the next boundary.
    let mut due = Some(Utc::now());
    loop {
        if let Some(at) = due {
            let wait = (at - Utc::now()).to_std().unwrap_or_default();
            let mut slept = Duration::ZERO;
            while slept < wait {
                if !registry.is_selection_declared(id) {
                    break;
                }
                let step = (wait - slept).min(Duration::from_secs(5));
                tokio::time::sleep(step).await;
                slept += step;
            }
        }
        if !registry.is_selection_declared(id) {
            allocator.stop_all(Utc::now(), &journal).await;
            info!(deployment = %id, "selection-bound deployment withdrawn; members stopped");
            return;
        }
        let now = Utc::now();
        allocator.reap(now, &members, &journal).await;
        match allocator
            .rebalance(now, &selections, &members, &journal)
            .await
        {
            Ok(changes) => info!(
                deployment = %id,
                changes = changes.len(),
                allocated = %allocator.allocation().values().map(|s| s.weight).sum::<Decimal>(),
                "rebalanced"
            ),
            Err(e) => {
                warn!(deployment = %id, error = %e, "selection could not run; holdings unchanged until the next rebalance")
            }
        }
        let next = next_due(
            &b.rebalance,
            schedule.as_deref(),
            tf,
            now.timestamp_nanos_opt().unwrap_or(0),
        );
        due = next.map(|ns| {
            DateTime::from_timestamp_nanos(ns)
                + chrono::Duration::from_std(GRACE).unwrap_or_default()
        });
        if due.is_none() {
            warn!(deployment = %id, "no further rebalance is scheduled; members keep running");
            // Still watch for withdrawal.
            due = Some(Utc::now() + chrono::Duration::hours(1));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use domain::strategy_graph::deployment::Weighting;
    use domain::strategy_graph::selection::SelectedAsset;
    use std::sync::Mutex;

    /// Selections scripted in order.
    struct Scripted(Mutex<Vec<Vec<&'static str>>>);

    #[async_trait]
    impl SelectionPort for Scripted {
        async fn select(
            &self,
            revision: &str,
            at: DateTime<Utc>,
        ) -> Result<(Selection, Option<Uuid>, String), String> {
            let next = self.0.lock().unwrap().remove(0);
            Ok((
                Selection {
                    revision: revision.into(),
                    as_of: at,
                    universe_size: 10,
                    selected: next
                        .iter()
                        .enumerate()
                        .map(|(i, m)| SelectedAsset {
                            instrument: (*m).into(),
                            rank: u32::try_from(i + 1).unwrap(),
                            score: 1.0,
                            attributes: BTreeMap::new(),
                        })
                        .collect(),
                    excluded: vec![],
                },
                Some(Uuid::new_v4()),
                "scripted".to_string(),
            ))
        }
    }

    #[derive(Default)]
    struct FakeMembers {
        started: Mutex<Vec<(String, Uuid)>>,
        stopped: Mutex<Vec<Uuid>>,
        refuse: Vec<&'static str>,
    }

    #[async_trait]
    impl MemberPort for FakeMembers {
        async fn start(
            &self,
            _b: &SelectionBinding,
            instrument: &str,
            _s: Arc<SleeveHandle>,
        ) -> Result<Uuid, String> {
            if self.refuse.contains(&instrument) {
                return Err("no instrument metadata".into());
            }
            let id = Uuid::new_v4();
            self.started.lock().unwrap().push((instrument.into(), id));
            Ok(id)
        }
        fn running(&self, child: Uuid) -> bool {
            !self.stopped.lock().unwrap().contains(&child)
        }
    }

    #[derive(Default)]
    struct Journal(Mutex<Vec<storage::postgres::deployments::AllocationEvent>>);

    #[async_trait]
    impl AllocationJournal for Journal {
        async fn record(
            &self,
            e: storage::postgres::deployments::AllocationEvent,
        ) -> Result<(), String> {
            self.0.lock().unwrap().push(e);
            Ok(())
        }
    }

    fn binding(on_removal: OnRemoval) -> Arc<SelectionBinding> {
        let trading: domain::strategy_graph::StrategyGraph =
            serde_json::from_value(serde_json::json!({
              "format": "tbot.strategy/3", "kind": "trading", "name": "t",
              "asset_class": "crypto_spot_cex", "timeframe": "1h",
              "nodes": [{"id":"go","op":"ge","args":{"a":"close","b":0}}],
              "decide": {"rules": [{"when":"go","then":{"kind":"hold"}}]}
            }))
            .unwrap();
        Arc::new(SelectionBinding {
            deployment_id: Uuid::new_v4(),
            user_id: "u".into(),
            trading_revision: "r".into(),
            trading,
            selection: "s".into(),
            top_n: 3,
            rebalance: Rebalance::OnSelectionRun,
            on_removal,
            weighting: Weighting::Equal,
            capital: Decimal::from(10_000),
        })
    }

    fn capital_held(a: &Allocator) -> Decimal {
        a.members().values().map(|m| m.sleeve.capital()).sum()
    }

    #[tokio::test]
    async fn membership_changes_start_and_stop_members_within_one_rebalance() {
        let b = binding(OnRemoval::Close);
        let sel = Scripted(Mutex::new(vec![vec!["A", "B", "C"], vec!["B", "C", "D"]]));
        let members = FakeMembers::default();
        let journal = Journal::default();
        let mut a = Allocator::new(Arc::clone(&b));

        a.rebalance(Utc::now(), &sel, &members, &journal)
            .await
            .unwrap();
        assert_eq!(a.members().keys().collect::<Vec<_>>(), ["A", "B", "C"]);
        assert_eq!(
            capital_held(&a),
            b.capital,
            "thirds of 10,000 sum to 10,000 exactly"
        );

        let a_sleeve = Arc::clone(&a.members()["A"].sleeve);
        let changes = a
            .rebalance(Utc::now(), &sel, &members, &journal)
            .await
            .unwrap();
        assert_eq!(
            a.members().keys().collect::<Vec<_>>(),
            ["B", "C", "D"],
            "one rebalance"
        );
        assert_eq!(
            a_sleeve.retiring(),
            Some(OnRemoval::Close),
            "the leaver is told to flatten"
        );
        assert!(changes.iter().any(
            |c| matches!(c, AllocationChange::Entered { instrument, .. } if instrument == "D")
        ));
        assert_eq!(capital_held(&a), b.capital);
        assert_eq!(members.started.lock().unwrap().len(), 4);
    }

    #[tokio::test]
    async fn every_change_is_journaled_with_its_capital_and_its_selection_run() {
        let b = binding(OnRemoval::Close);
        let sel = Scripted(Mutex::new(vec![vec!["A", "B"], vec!["B", "C"]]));
        let members = FakeMembers::default();
        let journal = Journal::default();
        let mut a = Allocator::new(Arc::clone(&b));
        let mut changes = a
            .rebalance(Utc::now(), &sel, &members, &journal)
            .await
            .unwrap();
        changes.extend(
            a.rebalance(Utc::now(), &sel, &members, &journal)
                .await
                .unwrap(),
        );
        let rows = journal.0.lock().unwrap();
        assert_eq!(rows.len(), changes.len(), "one row per change");
        let kinds: Vec<(&str, &str)> = rows
            .iter()
            .map(|e| (e.change.as_str(), e.instrument_id.as_str()))
            .collect();
        assert_eq!(
            kinds,
            [
                ("entered", "A"),
                ("entered", "B"),
                ("left", "A"),
                ("entered", "C")
            ],
            "B keeps its half, so it is not reweighted"
        );
        assert!(rows
            .iter()
            .all(|e| e.selection_run_id.is_some() && e.deployment_id == b.deployment_id));
        assert_eq!(rows[2].action.as_deref(), Some("close"));
        assert_eq!(rows[0].capital, Decimal::from(5_000));
        assert!(rows.iter().all(|e| e.member_deployment_id.is_some()));
    }

    #[tokio::test]
    async fn an_exit_keeps_its_capital_until_its_runner_stops_then_releases_it() {
        let b = binding(OnRemoval::LetStrategyExit);
        let sel = Scripted(Mutex::new(vec![vec!["A"], vec!["B"], vec!["B"]]));
        let members = FakeMembers::default();
        let journal = Journal::default();
        let mut a = Allocator::new(Arc::clone(&b));
        a.rebalance(Utc::now(), &sel, &members, &journal)
            .await
            .unwrap();
        a.rebalance(Utc::now(), &sel, &members, &journal)
            .await
            .unwrap();
        assert!(
            a.allocation()["A"].exiting,
            "A holds its capital while it exits"
        );
        assert!(!a.members().contains_key("B"), "nothing free for B yet");
        // The exiting runner reaches flat and stops.
        let a_child = a.members()["A"].child;
        members.stopped.lock().unwrap().push(a_child);
        let reaped = a.reap(Utc::now(), &members, &journal).await;
        assert!(matches!(
            &reaped[..],
            [AllocationChange::Left {
                action: OnRemoval::LetStrategyExit,
                ..
            }]
        ));
        a.rebalance(Utc::now(), &sel, &members, &journal)
            .await
            .unwrap();
        assert_eq!(capital_held(&a), b.capital, "B now holds what A released");
    }

    #[tokio::test]
    async fn a_member_that_cannot_start_is_not_a_member() {
        let b = binding(OnRemoval::Close);
        let sel = Scripted(Mutex::new(vec![vec!["A", "BAD"]]));
        let members = FakeMembers {
            refuse: vec!["BAD"],
            ..FakeMembers::default()
        };
        let journal = Journal::default();
        let mut a = Allocator::new(b);
        let changes = a
            .rebalance(Utc::now(), &sel, &members, &journal)
            .await
            .unwrap();
        assert_eq!(changes.len(), 1);
        assert!(!a.allocation().contains_key("BAD"));
        assert_eq!(
            journal.0.lock().unwrap().len(),
            1,
            "no row for what did not happen"
        );
    }

    #[tokio::test]
    async fn stopping_the_deployment_stops_every_member_holding_its_position() {
        let b = binding(OnRemoval::Close);
        let sel = Scripted(Mutex::new(vec![vec!["A", "B"]]));
        let members = FakeMembers::default();
        let journal = Journal::default();
        let mut a = Allocator::new(b);
        a.rebalance(Utc::now(), &sel, &members, &journal)
            .await
            .unwrap();
        let sleeves: Vec<_> = a
            .members()
            .values()
            .map(|m| Arc::clone(&m.sleeve))
            .collect();
        a.stop_all(Utc::now(), &journal).await;
        assert!(sleeves
            .iter()
            .all(|s| s.retiring() == Some(OnRemoval::Hold)));
        assert!(a.members().is_empty() && a.allocation().is_empty());
        assert_eq!(
            journal
                .0
                .lock()
                .unwrap()
                .iter()
                .filter(|e| e.change == "left")
                .count(),
            2
        );
    }

    #[test]
    fn a_rebalance_is_due_at_the_selections_close_or_on_a_schedule() {
        let day = 86_400;
        let midnight = 1_767_225_600_000_000_000_i64; // 2026-01-01T00:00Z
        let t = midnight + 3_600_000_000_000;
        assert_eq!(
            next_due(&Rebalance::OnSelectionRun, None, day, t),
            Some(midnight + 86_400_000_000_000)
        );
        assert_eq!(
            next_due(&Rebalance::OnSelectionRun, Some("0 12 * * *"), day, t),
            Some(midnight + 12 * 3_600_000_000_000)
        );
        assert_eq!(
            next_due(
                &Rebalance::Schedule {
                    cron: "0 0 * * 1".into()
                },
                Some("0 12 * * *"),
                day,
                t
            ),
            Some(midnight + 4 * 86_400_000_000_000),
            "2026-01-05 is the first Monday; the deployment's own schedule wins"
        );
    }

    /// The production member port: a member is recorded as a child of its
    /// deployment and declared with its sleeve, where the live-runner
    /// supervisor runs it; withdrawn, it no longer runs.
    #[tokio::test]
    #[ignore = "needs DATABASE_URL with migrations through 0148"]
    async fn a_member_is_recorded_under_its_deployment_and_declared_with_its_sleeve() {
        let pg = sqlx::PgPool::connect(&std::env::var("DATABASE_URL").unwrap())
            .await
            .unwrap();
        let b = binding(OnRemoval::Close);
        let user = format!("u-{}", Uuid::new_v4());
        let b = Arc::new(SelectionBinding {
            user_id: user.clone(),
            trading_revision: "d".repeat(64),
            selection: "e".repeat(64),
            ..(*b).clone()
        });
        storage::postgres::deployments::start(
            &pg,
            &storage::postgres::deployments::NewDeployment {
                deployment_id: b.deployment_id,
                user_id: user.clone(),
                strategy_name: b.trading.name.clone(),
                revision_hash: b.trading_revision.clone(),
                definition_json: serde_json::to_value(&b.trading).unwrap(),
                instrument_id: format!("selection:{}", b.selection),
                starting_capital: b.capital,
                model_pins: serde_json::json!([]),
                started_by: user.clone(),
                forward_deployment_id: None,
                target: Some(
                    serde_json::json!({"kind": "selection", "selection": b.selection, "top_n": 3}),
                ),
                parent_deployment_id: None,
            },
        )
        .await
        .expect("parent recorded");

        let registry = Arc::new(LiveStrategyRegistry::new());
        let port = PgMembers {
            pg: pg.clone(),
            registry: Arc::clone(&registry),
        };
        let sleeve = Arc::new(SleeveHandle::new(b.deployment_id, Decimal::from(2_500)));
        let child = port
            .start(&b, "SOL-USD", Arc::clone(&sleeve))
            .await
            .expect("started");
        assert!(port.running(child));
        let declared = registry.v3_snapshot();
        assert_eq!(declared.len(), 1);
        assert_eq!(declared[0].instrument_id, "SOL-USD");
        assert!(
            Arc::ptr_eq(declared[0].sleeve.as_ref().unwrap(), &sleeve),
            "the same handle"
        );
        let kids = storage::postgres::deployments::running_children(&pg, b.deployment_id)
            .await
            .unwrap();
        assert_eq!(kids.len(), 1);
        assert_eq!(kids[0].deployment_id, child);
        assert_eq!(kids[0].starting_capital, Decimal::from(2_500));

        // Resumed after a restart: the member comes back with its sleeve.
        registry.deregister_v3(child);
        assert!(!port.running(child));
        let resumed = resume_members(&b, &pg, &registry).await;
        assert_eq!(resumed.members().len(), 1);
        assert_eq!(
            resumed.members()["SOL-USD"].sleeve.capital(),
            Decimal::from(2_500)
        );
        assert!(registry.is_v3_declared(child));

        storage::postgres::deployments::stop(&pg, child, "test")
            .await
            .unwrap();
        storage::postgres::deployments::stop(&pg, b.deployment_id, "test")
            .await
            .unwrap();
    }
}
