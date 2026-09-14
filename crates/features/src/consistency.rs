//! Backfill-and-diff (SPEC §3.3, INV-14): recompute a logged serve from the data as
//! it stands now, through the same runtime, and classify every difference.
//!
//! Pure: the caller loads the bars. Both knowledge times travel with each diff —
//! the served one from the log, the recomputed one as the latest knowledge time of
//! any row inside the feature's window now.

use chrono::{DateTime, Utc};
use dataplane::feature::{diagnose, ConsistencyDiff, FeatureRow, ServeRecord, EVALUATE_FN};

use crate::runtime;

/// The diffs for one serve, and the features that could not be recomputed at all
/// (the window no longer exists in the data) — reported, never diagnosed as drift.
#[derive(Debug, Default)]
pub struct ServeDiff {
    pub diffs: Vec<ConsistencyDiff>,
    pub unrecomputable: Vec<String>,
}

/// Recompute every value in `serve` at `decision` over `rows` (ascending), with
/// `knowledge_ns[i]` the knowledge time of `rows[i]` as read now.
#[must_use]
pub fn diff_serve(serve: &ServeRecord, rows: &[FeatureRow], knowledge_ns: &[i64], decision: usize) -> ServeDiff {
    let mut out = ServeDiff::default();
    for (id, &served) in &serve.values {
        let Ok(feature) = runtime::feature(id) else {
            out.unrecomputable.push(id.clone());
            continue;
        };
        let (Ok(recomputed), Ok(again)) = (EVALUATE_FN(feature.as_ref(), rows, decision), EVALUATE_FN(feature.as_ref(), rows, decision)) else {
            out.unrecomputable.push(id.clone());
            continue;
        };
        let rerun_agrees = recomputed.to_bits() == again.to_bits();
        let lookback = feature.def().lookback_bars as usize;
        let lo = (decision + 1).saturating_sub(lookback);
        let recomputed_kt = knowledge_ns
            .get(lo..=decision)
            .and_then(|w| w.iter().max())
            .map_or(serve.knowledge_time, |n| DateTime::<Utc>::from_timestamp_nanos(*n));
        let served_hash = serve.code_hashes.get(id).map_or("", String::as_str);
        out.diffs.push(ConsistencyDiff {
            serve_id: serve.serve_id,
            feature_id: id.clone(),
            served_value: served,
            recomputed_value: recomputed,
            abs_diff: (served - recomputed).abs(),
            served_knowledge_time: serve.knowledge_time,
            recomputed_knowledge_time: recomputed_kt,
            diagnosis: diagnose(served, recomputed, serve.knowledge_time, recomputed_kt, served_hash, &feature.def().code_hash, rerun_agrees),
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use chrono::Duration;
    use dataplane::feature::{Diagnosis, FeatureRuntime, MemoryServeLog};

    use super::*;

    fn rows(n: usize) -> Vec<FeatureRow> {
        (0..n)
            .map(|i| {
                let c = 100.0 + (i as f64 * 0.37).sin() * 4.0;
                FeatureRow { ts_ns: i as i64 * 60_000_000_000, open: c, high: c + 1.0, low: c - 1.0, close: c, volume: 5.0 }
            })
            .collect()
    }

    fn serve(names: &[&str], data: &[FeatureRow], decision: usize, kt: DateTime<Utc>) -> ServeRecord {
        let log = std::sync::Arc::new(MemoryServeLog::default());
        let feats = names.iter().map(|n| runtime::feature(n).unwrap()).collect();
        let rt = FeatureRuntime::new("fs", feats, log.clone()).unwrap();
        rt.serve_live("t", 1, &data[..=decision], kt, kt).unwrap();
        let rec = log.0.lock().unwrap().pop().unwrap();
        rec
    }

    #[test]
    fn an_untouched_serve_matches_exactly() {
        let data = rows(200);
        let kt = Utc::now();
        let s = serve(&["ema_7", "rsi_14", "zscore_20"], &data, 150, kt);
        let d = diff_serve(&s, &data, &vec![kt.timestamp_nanos_opt().unwrap(); data.len()], 150);
        assert!(d.unrecomputable.is_empty());
        assert_eq!(d.diffs.len(), 3);
        assert!(d.diffs.iter().all(|x| x.diagnosis == Diagnosis::Match && x.abs_diff == 0.0));
    }

    #[test]
    fn a_restated_bar_that_arrived_later_is_late_arrival_not_drift() {
        let data = rows(200);
        let served_kt = Utc::now();
        let s = serve(&["rolling_mean_10"], &data, 150, served_kt);
        let mut now = data.clone();
        now[148].close += 3.0;
        let mut kn = vec![served_kt.timestamp_nanos_opt().unwrap(); data.len()];
        kn[148] = (served_kt + Duration::minutes(40)).timestamp_nanos_opt().unwrap();
        let d = diff_serve(&s, &now, &kn, 150);
        assert_eq!(d.diffs[0].diagnosis, Diagnosis::LateArrival);
        assert_eq!(d.diffs[0].recomputed_knowledge_time - d.diffs[0].served_knowledge_time, Duration::minutes(40));
    }

    #[test]
    fn a_changed_implementation_is_code_drift() {
        let data = rows(200);
        let kt = Utc::now();
        let mut s = serve(&["ema_7"], &data, 150, kt);
        s.code_hashes.insert("ema_7".into(), "sha256:old".into());
        s.values.insert("ema_7".into(), s.values["ema_7"] + 0.5);
        let d = diff_serve(&s, &data, &vec![kt.timestamp_nanos_opt().unwrap(); data.len()], 150);
        assert_eq!(d.diffs[0].diagnosis, Diagnosis::CodeDrift);
    }

    #[test]
    fn a_vanished_window_is_reported_not_diagnosed() {
        let data = rows(200);
        let kt = Utc::now();
        let s = serve(&["ema_21"], &data, 150, kt);
        let truncated = &data[140..];
        let d = diff_serve(&s, truncated, &vec![0; truncated.len()], 10);
        assert!(d.diffs.is_empty());
        assert_eq!(d.unrecomputable, vec!["ema_21".to_string()]);
        let unknown = ServeRecord { values: BTreeMap::from([("gone_feature".to_string(), 1.0)]), ..s };
        assert_eq!(diff_serve(&unknown, &data, &[], 150).unrecomputable, vec!["gone_feature".to_string()]);
    }
}
