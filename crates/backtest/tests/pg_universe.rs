//! Set U U-6.1 and U-6.3 against PostgreSQL: universes load from the record and
//! resolve point-in-time; selection runs are journaled append-only; and the
//! platform's role can reach both — the grant 0104 forgot.
//!
//! ```text
//! DATABASE_URL=postgres://…/scratch cargo test -j 2 -p backtest --test pg_universe -- --ignored
//! ```

use std::collections::BTreeMap;

use backtest::env::universe::{self, UniverseLoadError};
use backtest::selection_run::{self, RequestedBy, SelectionReport};
use chrono::{Duration, TimeZone, Utc};
use domain::strategy_graph::selection::{SelectedAsset, Selection};
use domain::strategy_graph::UniverseRef;
use strategy_engine::selection::SelectionRun;
use uuid::Uuid;

async fn pool() -> sqlx::PgPool {
    sqlx::PgPool::connect(&std::env::var("DATABASE_URL").expect("DATABASE_URL"))
        .await
        .expect("connects")
}

fn ns(t: chrono::DateTime<Utc>) -> i64 {
    t.timestamp_nanos_opt().unwrap()
}

async fn instrument(pg: &sqlx::PgPool, symbol: Option<&str>) -> i64 {
    let (id,): (i64,) = sqlx::query_as(
        "INSERT INTO dataplane.instrument (asset_class, first_seen) VALUES ('crypto', now()) RETURNING instrument_id",
    )
    .fetch_one(pg)
    .await
    .unwrap();
    if let Some(s) = symbol {
        sqlx::query(
            "INSERT INTO dataplane.instrument_symbol (instrument_id, venue_id, symbol, valid_from, knowledge_time) \
             VALUES ($1, 1, $2, '2020-01-01', '2020-01-01')",
        )
        .bind(id)
        .bind(s)
        .execute(pg)
        .await
        .unwrap();
    }
    id
}

#[tokio::test]
#[ignore = "needs DATABASE_URL with migrations through 0147"]
async fn a_named_universe_loads_from_the_record_and_resolves_point_in_time() {
    let pg = pool().await;
    let tag = Uuid::new_v4().simple().to_string();
    let name = format!("u6-{tag}");
    let sym = |s: &str| format!("{s}-{}", &tag[..8]);
    let a = instrument(&pg, Some(&sym("A"))).await;
    let b = instrument(&pg, Some(&sym("B"))).await;
    let c = instrument(&pg, Some(&sym("C"))).await;
    let nameless = instrument(&pg, None).await;
    sqlx::query("INSERT INTO dataplane.universe (name) VALUES ($1)")
        .bind(&name)
        .execute(&pg)
        .await
        .unwrap();
    let day = |d: i64| Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap() + Duration::days(d - 1);
    for (key, change, effective, recorded) in [
        (a, "added", day(1), day(1)),
        (c, "added", day(1), day(1)),
        (c, "removed", day(4), day(4)),
        (b, "added", day(6), day(6)),
        (nameless, "added", day(1), day(1)),
    ] {
        sqlx::query(
            "INSERT INTO dataplane.universe_membership_event (universe, instrument_id, change, effective, recorded_at, reason) \
             VALUES ($1, $2, $3, $4, $5, 'test')",
        )
        .bind(&name)
        .bind(key)
        .bind(change)
        .bind(effective)
        .bind(recorded)
        .execute(&pg)
        .await
        .unwrap();
    }

    let u = universe::load(&pg, &UniverseRef::Named { name: name.clone() })
        .await
        .expect("loads");
    assert!(u.point_in_time());
    let mut at3 = vec![format!("#{nameless}"), sym("A"), sym("C")];
    at3.sort();
    assert_eq!(
        u.members_at(ns(day(3))).unwrap(),
        at3,
        "C still a member; B not yet"
    );
    let at7 = u.members_at(ns(day(7))).unwrap();
    assert!(at7.contains(&sym("B")) && !at7.contains(&sym("C")));

    assert!(matches!(
        universe::load(
            &pg,
            &UniverseRef::Named {
                name: format!("{name}-nope")
            }
        )
        .await,
        Err(UniverseLoadError::UnknownUniverse(_))
    ));

    // An asset class: today's initialised instruments, flagged as not
    // point-in-time.
    let class = format!("class_{tag}");
    for s in ["X", "Y"] {
        sqlx::query(
            "INSERT INTO asset_lifecycle (symbol, asset_class, venue_id) VALUES ($1, $2, 'paper')",
        )
        .bind(sym(s))
        .bind(&class)
        .execute(&pg)
        .await
        .unwrap();
    }
    let by_class = universe::load(&pg, &UniverseRef::AssetClass { asset_class: class })
        .await
        .unwrap();
    assert!(!by_class.point_in_time());
    assert_eq!(by_class.members_at(0).unwrap(), [sym("X"), sym("Y")]);
}

#[tokio::test]
#[ignore = "needs DATABASE_URL with migrations through 0147"]
async fn a_selection_run_is_journaled_append_only_and_the_platform_role_can_reach_it() {
    let pg = pool().await;
    let strategy = format!("screen-{}", Uuid::new_v4());
    let selection = Selection {
        revision: "c".repeat(64),
        as_of: Utc.with_ymd_and_hms(2025, 3, 1, 0, 0, 0).unwrap(),
        universe_size: 3,
        selected: vec![SelectedAsset {
            instrument: "SOL-USD".into(),
            rank: 1,
            score: 2.0,
            attributes: BTreeMap::new(),
        }],
        excluded: vec![],
    };
    let run = SelectionRun {
        selection: selection.clone(),
        values: BTreeMap::new(),
        model_requests: vec![],
        model_responses: vec![],
    };
    let report = SelectionReport {
        strategy_ref: strategy.clone(),
        revision: selection.revision.clone(),
        as_of: selection.as_of,
        universe_point_in_time: true,
        digest: run.digest(),
        run,
    };
    let id = selection_run::journal(&pg, &report, RequestedBy::Agent)
        .await
        .unwrap();
    let rows = selection_run::list(&pg, Some(&strategy), 10).await.unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].run_id, id);
    assert_eq!(rows[0].requested_by, "agent");
    assert_eq!(rows[0].digest, report.digest);
    assert!(
        sqlx::query("UPDATE selection_run SET selected_count = 0 WHERE run_id = $1")
            .bind(id)
            .execute(&pg)
            .await
            .is_err(),
        "append-only"
    );

    // As the platform's own role, not the owner the other tests run as.
    let mut tx = pg.begin().await.unwrap();
    sqlx::query("SET LOCAL ROLE app_role")
        .execute(&mut *tx)
        .await
        .unwrap();
    for table in [
        "dataplane.universe",
        "dataplane.universe_membership_event",
        "selection_run",
        "deployment_allocation_event",
    ] {
        let q = format!("SELECT count(*) FROM {table}");
        let r: Result<(i64,), _> = sqlx::query_as(&q).fetch_one(&mut *tx).await;
        assert!(r.is_ok(), "app_role cannot read {table}: {r:?}");
    }
    tx.rollback().await.unwrap();
}
