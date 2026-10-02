//! Bars and indicators, inside the engine (U-1.3).
//!
//! Every input keeps its recent closed bars and, for each indicator the plan
//! names on it, a column of values computed **once per bar** through
//! `features::runtime::value_at` — the one DATA-006 routine, unchanged. Nothing
//! outside the engine supplies an indicator value in any environment; there is no
//! public method that could. That is the structural fix for FEAT-007 §1 defect 2,
//! where paper trading had a slot for each indicator and nothing ever filled it.

use std::collections::VecDeque;
use std::sync::Arc;

use dataplane::feature::{Feature, FeatureRow};
use serde::{Deserialize, Serialize};

/// One closed bar.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Bar {
    /// The bar's close instant, ns since the epoch — when it became knowable.
    pub close_ns: i64,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: f64,
}

impl Bar {
    fn row(self) -> FeatureRow {
        FeatureRow {
            ts_ns: self.close_ns,
            open: self.open,
            high: self.high,
            low: self.low,
            close: self.close,
            volume: self.volume,
        }
    }
}

/// One input's history.
pub(crate) struct History {
    rows: VecDeque<FeatureRow>,
    rows_cap: usize,
    /// One column per plan indicator on this input; `None` where the indicator
    /// had no value (window incomplete, or a non-finite result).
    columns: Vec<VecDeque<Option<f64>>>,
    cols_cap: usize,
    impls: Vec<Arc<dyn Feature>>,
    pub(crate) bars_seen: u64,
    pub(crate) last: Option<Bar>,
}

impl History {
    /// `rows_cap`: the longest indicator lookback; `cols_cap`: the longest model
    /// window over this input.
    pub(crate) fn new(impls: Vec<Arc<dyn Feature>>, rows_cap: usize, cols_cap: usize) -> Self {
        let n = impls.len();
        Self {
            rows: VecDeque::with_capacity(rows_cap.max(1)),
            rows_cap: rows_cap.max(1),
            columns: (0..n)
                .map(|_| VecDeque::with_capacity(cols_cap.max(1)))
                .collect(),
            cols_cap: cols_cap.max(1),
            impls,
            bars_seen: 0,
            last: None,
        }
    }

    /// Append a closed bar and compute every column at it.
    pub(crate) fn push(&mut self, bar: Bar) {
        self.rows.push_back(bar.row());
        while self.rows.len() > self.rows_cap {
            self.rows.pop_front();
        }
        let rows = self.rows.make_contiguous();
        let decision = rows.len() - 1;
        for (i, f) in self.impls.iter().enumerate() {
            let v = features::runtime::value_at(f.as_ref(), rows, decision);
            let col = &mut self.columns[i];
            col.push_back(v);
            while col.len() > self.cols_cap {
                col.pop_front();
            }
        }
        self.bars_seen += 1;
        self.last = Some(bar);
    }

    /// The latest value of column `c`.
    pub(crate) fn latest(&self, c: usize) -> Option<f64> {
        self.columns
            .get(c)
            .and_then(|col| col.back().copied().flatten())
    }

    /// The last `n` values of column `c`, oldest first; `None` if fewer than `n`
    /// bars have been seen or any of them is absent.
    pub(crate) fn window(&self, c: usize, n: usize) -> Option<Vec<f64>> {
        let col = self.columns.get(c)?;
        if col.len() < n {
            return None;
        }
        col.iter().skip(col.len() - n).copied().collect()
    }
}
