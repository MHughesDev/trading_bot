//! Process-wide Prometheus metrics: a registry, call-site helpers and the
//! `/metrics` exporter (Set E 5.1).
//!
//! # Why not the `metrics` / `prometheus` crates
//!
//! The registry is ~200 lines of atomics and a text renderer, and the
//! exposition format is stable and small. Taking a dependency tree for it would
//! change the shared lockfile for every crate in the workspace; this is the
//! whole of what the platform needs. Declared in the Set E phase-5 ADR row.
//!
//! # Rules the registry enforces
//!
//! * **Cardinality.** Label *values* are capped per metric family
//!   ([`MAX_SERIES_PER_FAMILY`]); a family that would exceed it records into a
//!   single `{overflow="true"}` series and counts the overflow in
//!   `metrics_series_overflow_total`. Label by lane, venue, source or asset
//!   class — never by instrument id or order id.
//! * **Names** must match the Prometheus grammar and the family's type is fixed
//!   at first use; a later mismatched call is dropped and counted, never a
//!   panic on a hot path.
//! * Recording is a mutex-guarded map lookup plus an atomic add: cheap, but not
//!   free. Keep it off the rtrb hot-path stages (record per order, per publish
//!   batch, per gap — not per tick).

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

/// Most distinct label sets one metric family may hold.
pub const MAX_SERIES_PER_FAMILY: usize = 256;

/// Histogram buckets (seconds or milliseconds — the caller's unit, stated in
/// the metric name).
pub const DEFAULT_BUCKETS: &[f64] = &[
    0.001, 0.005, 0.01, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0, 30.0, 60.0,
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Counter,
    Gauge,
    Histogram,
}

impl Kind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Counter => "counter",
            Self::Gauge => "gauge",
            Self::Histogram => "histogram",
        }
    }
}

type Labels = Vec<(String, String)>;

#[derive(Debug)]
struct Hist {
    buckets: Vec<AtomicU64>,
    count: AtomicU64,
    /// f64 bits.
    sum: AtomicU64,
}

#[derive(Debug)]
enum Value {
    /// Counter (u64) or gauge (f64 bits).
    Scalar(AtomicU64),
    Histogram(Hist),
}

#[derive(Debug)]
struct Family {
    kind: Kind,
    help: String,
    series: BTreeMap<Labels, Arc<Value>>,
}

/// The registry. One per process ([`global`]); constructible for tests.
#[derive(Debug, Default)]
pub struct Registry {
    families: Mutex<BTreeMap<String, Family>>,
    dropped: AtomicU64,
    overflow: AtomicU64,
}

fn valid_name(name: &str) -> bool {
    let mut chars = name.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_alphabetic() || c == '_' || c == ':')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == ':')
}

fn escape(v: &str) -> String {
    v.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
}

fn add_f64(cell: &AtomicU64, by: f64) {
    let mut cur = cell.load(Ordering::Relaxed);
    loop {
        let next = (f64::from_bits(cur) + by).to_bits();
        match cell.compare_exchange_weak(cur, next, Ordering::Relaxed, Ordering::Relaxed) {
            Ok(_) => return,
            Err(actual) => cur = actual,
        }
    }
}

impl Registry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    fn series(
        &self,
        name: &str,
        help: &str,
        kind: Kind,
        labels: &[(&str, &str)],
    ) -> Option<Arc<Value>> {
        if !valid_name(name) || labels.iter().any(|(k, _)| !valid_name(k)) {
            self.dropped.fetch_add(1, Ordering::Relaxed);
            return None;
        }
        let mut key: Labels = labels
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect();
        key.sort();
        let mut families = self
            .families
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let family = families.entry(name.to_owned()).or_insert_with(|| Family {
            kind,
            help: help.to_owned(),
            series: BTreeMap::new(),
        });
        if family.kind != kind {
            self.dropped.fetch_add(1, Ordering::Relaxed);
            return None;
        }
        if !family.series.contains_key(&key) && family.series.len() >= MAX_SERIES_PER_FAMILY {
            self.overflow.fetch_add(1, Ordering::Relaxed);
            key = vec![("overflow".to_owned(), "true".to_owned())];
        }
        let value = family
            .series
            .entry(key)
            .or_insert_with(|| {
                Arc::new(match kind {
                    Kind::Histogram => Value::Histogram(Hist {
                        buckets: DEFAULT_BUCKETS.iter().map(|_| AtomicU64::new(0)).collect(),
                        count: AtomicU64::new(0),
                        sum: AtomicU64::new(0f64.to_bits()),
                    }),
                    Kind::Gauge => Value::Scalar(AtomicU64::new(0f64.to_bits())),
                    Kind::Counter => Value::Scalar(AtomicU64::new(0)),
                })
            })
            .clone();
        Some(value)
    }

    /// Add `by` to a counter.
    pub fn counter_add(&self, name: &str, help: &str, labels: &[(&str, &str)], by: u64) {
        if let Some(v) = self.series(name, help, Kind::Counter, labels) {
            if let Value::Scalar(c) = &*v {
                c.fetch_add(by, Ordering::Relaxed);
            }
        }
    }

    /// Set a gauge.
    pub fn gauge_set(&self, name: &str, help: &str, labels: &[(&str, &str)], value: f64) {
        if let Some(v) = self.series(name, help, Kind::Gauge, labels) {
            if let Value::Scalar(g) = &*v {
                g.store(value.to_bits(), Ordering::Relaxed);
            }
        }
    }

    /// Record one observation in a histogram.
    pub fn histogram_observe(&self, name: &str, help: &str, labels: &[(&str, &str)], value: f64) {
        if let Some(v) = self.series(name, help, Kind::Histogram, labels) {
            if let Value::Histogram(h) = &*v {
                for (i, le) in DEFAULT_BUCKETS.iter().enumerate() {
                    if value <= *le {
                        h.buckets[i].fetch_add(1, Ordering::Relaxed);
                    }
                }
                h.count.fetch_add(1, Ordering::Relaxed);
                add_f64(&h.sum, value);
            }
        }
    }

    /// The current value of a counter series, if it exists. For tests and
    /// in-process health checks.
    #[must_use]
    pub fn counter_value(&self, name: &str, labels: &[(&str, &str)]) -> Option<u64> {
        let mut key: Labels = labels
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect();
        key.sort();
        let families = self
            .families
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        match families.get(name)?.series.get(&key).map(|v| &**v) {
            Some(Value::Scalar(c)) => Some(c.load(Ordering::Relaxed)),
            _ => None,
        }
    }

    /// Render every family in the Prometheus text exposition format (0.0.4).
    #[must_use]
    pub fn render(&self) -> String {
        let mut out = String::new();
        let families = self
            .families
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        for (name, family) in families.iter() {
            let _ = writeln!(out, "# HELP {name} {}", escape(&family.help));
            let _ = writeln!(out, "# TYPE {name} {}", family.kind.as_str());
            for (labels, value) in &family.series {
                let render_labels = |extra: Option<(&str, String)>| -> String {
                    let mut parts: Vec<String> = labels
                        .iter()
                        .map(|(k, v)| format!("{k}=\"{}\"", escape(v)))
                        .collect();
                    if let Some((k, v)) = extra {
                        parts.push(format!("{k}=\"{v}\""));
                    }
                    if parts.is_empty() {
                        String::new()
                    } else {
                        format!("{{{}}}", parts.join(","))
                    }
                };
                match &**value {
                    Value::Scalar(cell) => {
                        let v = cell.load(Ordering::Relaxed);
                        if family.kind == Kind::Counter {
                            let _ = writeln!(out, "{name}{} {v}", render_labels(None));
                        } else {
                            let _ = writeln!(
                                out,
                                "{name}{} {}",
                                render_labels(None),
                                f64::from_bits(v)
                            );
                        }
                    }
                    Value::Histogram(h) => {
                        for (i, le) in DEFAULT_BUCKETS.iter().enumerate() {
                            let _ = writeln!(
                                out,
                                "{name}_bucket{} {}",
                                render_labels(Some(("le", le.to_string()))),
                                h.buckets[i].load(Ordering::Relaxed)
                            );
                        }
                        let count = h.count.load(Ordering::Relaxed);
                        let _ = writeln!(
                            out,
                            "{name}_bucket{} {count}",
                            render_labels(Some(("le", "+Inf".to_owned())))
                        );
                        let _ = writeln!(
                            out,
                            "{name}_sum{} {}",
                            render_labels(None),
                            f64::from_bits(h.sum.load(Ordering::Relaxed))
                        );
                        let _ = writeln!(out, "{name}_count{} {count}", render_labels(None));
                    }
                }
            }
        }
        let _ = writeln!(out, "# HELP metrics_series_overflow_total Recordings folded into an overflow series by the cardinality cap.");
        let _ = writeln!(out, "# TYPE metrics_series_overflow_total counter");
        let _ = writeln!(
            out,
            "metrics_series_overflow_total {}",
            self.overflow.load(Ordering::Relaxed)
        );
        let _ = writeln!(out, "# HELP metrics_recordings_dropped_total Recordings refused for an invalid name or a type mismatch.");
        let _ = writeln!(out, "# TYPE metrics_recordings_dropped_total counter");
        let _ = writeln!(
            out,
            "metrics_recordings_dropped_total {}",
            self.dropped.load(Ordering::Relaxed)
        );
        out
    }
}

static GLOBAL: OnceLock<Registry> = OnceLock::new();

/// The process-wide registry every call site records into.
pub fn global() -> &'static Registry {
    GLOBAL.get_or_init(Registry::new)
}

/// Shorthand: increment a counter in the global registry.
pub fn inc(name: &str, help: &str, labels: &[(&str, &str)]) {
    global().counter_add(name, help, labels, 1);
}

/// Shorthand: set a gauge in the global registry.
pub fn set(name: &str, help: &str, labels: &[(&str, &str)], value: f64) {
    global().gauge_set(name, help, labels, value);
}

/// Shorthand: observe into a histogram in the global registry.
pub fn observe(name: &str, help: &str, labels: &[(&str, &str)], value: f64) {
    global().histogram_observe(name, help, labels, value);
}

/// Data-pipeline metric handles. Same signatures as the Phase-1 stubs they
/// replace, now recording into the global registry.
#[derive(Default, Clone, Copy)]
pub struct Metrics;

impl Metrics {
    pub fn new() -> Self {
        Self
    }

    pub fn increment_published(&self, lane: &str) {
        inc(
            "pipeline_events_published_total",
            "Events published to the bus, by lane.",
            &[("lane", lane)],
        );
    }

    pub fn increment_quarantined(&self, source: &str) {
        inc(
            "pipeline_events_quarantined_total",
            "Events that failed normalisation and went to quarantine, by source.",
            &[("source", source)],
        );
    }

    pub fn increment_gap_detected(&self, lane: &str) {
        inc(
            "pipeline_sequence_gaps_total",
            "Sequence gaps detected in an inbound feed, by lane.",
            &[("lane", lane)],
        );
    }
}

// ── The exporter ─────────────────────────────────────────────────────────────

/// Why the exporter would not start.
#[derive(Debug, thiserror::Error)]
pub enum ExporterError {
    #[error("refusing to serve /metrics on non-loopback address {0}; it is unauthenticated")]
    NotLoopback(std::net::SocketAddr),
    #[error("could not bind the metrics listener on {addr}: {source}")]
    Bind {
        addr: std::net::SocketAddr,
        source: std::io::Error,
    },
}

/// Bind `addr` and serve `GET /metrics` from the global registry, one
/// listener per process (`observability::init_metrics`, matching `init`).
///
/// Loopback only: the exposition is unauthenticated, so it is scraped from the
/// host (or a sidecar), never exposed. Returns the bound address (useful with
/// port 0) once the listener is accepting.
///
/// # Errors
/// [`ExporterError`].
pub async fn serve(addr: std::net::SocketAddr) -> Result<std::net::SocketAddr, ExporterError> {
    if !addr.ip().is_loopback() {
        return Err(ExporterError::NotLoopback(addr));
    }
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .map_err(|source| ExporterError::Bind { addr, source })?;
    let bound = listener.local_addr().unwrap_or(addr);
    tokio::spawn(async move {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else {
                continue;
            };
            tokio::spawn(async move {
                use tokio::io::{AsyncReadExt, AsyncWriteExt};
                let mut buf = [0u8; 2048];
                let n = match tokio::time::timeout(
                    std::time::Duration::from_secs(5),
                    stream.read(&mut buf),
                )
                .await
                {
                    Ok(Ok(n)) => n,
                    _ => return,
                };
                let req = String::from_utf8_lossy(&buf[..n]);
                let line = req.lines().next().unwrap_or("");
                let response = if line.starts_with("GET /metrics ") || line == "GET /metrics" {
                    let body = global().render();
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: text/plain; version=0.0.4; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    )
                } else {
                    "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                        .to_owned()
                };
                let _ = stream.write_all(response.as_bytes()).await;
                let _ = stream.shutdown().await;
            });
        }
    });
    tracing::info!(%bound, "metrics exporter serving GET /metrics");
    Ok(bound)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counters_gauges_and_histograms_render_in_exposition_format() {
        let r = Registry::new();
        r.counter_add("orders_total", "Orders.", &[("outcome", "filled")], 2);
        r.counter_add("orders_total", "Orders.", &[("outcome", "filled")], 1);
        r.gauge_set("lag_ms", "Lag.", &[("lane", "trades")], 12.5);
        r.histogram_observe("save_seconds", "Save time.", &[], 0.02);
        let text = r.render();
        assert!(text.contains("# TYPE orders_total counter"), "{text}");
        assert!(
            text.contains("orders_total{outcome=\"filled\"} 3"),
            "{text}"
        );
        assert!(text.contains("lag_ms{lane=\"trades\"} 12.5"), "{text}");
        assert!(
            text.contains("save_seconds_bucket{le=\"0.05\"} 1"),
            "{text}"
        );
        assert!(
            text.contains("save_seconds_bucket{le=\"0.01\"} 0"),
            "{text}"
        );
        assert!(text.contains("save_seconds_count 1"), "{text}");
        assert_eq!(
            r.counter_value("orders_total", &[("outcome", "filled")]),
            Some(3)
        );
    }

    #[test]
    fn cardinality_is_capped_into_an_overflow_series() {
        let r = Registry::new();
        for i in 0..(MAX_SERIES_PER_FAMILY + 10) {
            r.counter_add("by_id_total", "x", &[("id", &i.to_string())], 1);
        }
        assert_eq!(
            r.counter_value("by_id_total", &[("overflow", "true")]),
            Some(10)
        );
        assert!(r.render().contains("metrics_series_overflow_total 10"));
    }

    #[test]
    fn a_type_mismatch_or_bad_name_is_dropped_not_a_panic() {
        let r = Registry::new();
        r.counter_add("x_total", "x", &[], 1);
        r.gauge_set("x_total", "x", &[], 5.0);
        r.counter_add("bad name", "x", &[], 1);
        assert_eq!(r.counter_value("x_total", &[]), Some(1));
        assert!(r.render().contains("metrics_recordings_dropped_total 2"));
    }

    #[test]
    fn label_values_are_escaped() {
        let r = Registry::new();
        r.counter_add("e_total", "x", &[("v", "a\"b\\c")], 1);
        assert!(r.render().contains(r#"e_total{v="a\"b\\c"} 1"#));
    }

    #[tokio::test]
    async fn the_exporter_serves_live_counters_over_http() {
        Metrics::new().increment_quarantined("exporter-test");
        let addr = serve("127.0.0.1:0".parse().unwrap()).await.expect("bound");
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let mut s = tokio::net::TcpStream::connect(addr).await.unwrap();
        s.write_all(b"GET /metrics HTTP/1.1\r\nHost: x\r\n\r\n")
            .await
            .unwrap();
        let mut body = String::new();
        s.read_to_string(&mut body).await.unwrap();
        assert!(body.starts_with("HTTP/1.1 200 OK"), "{body}");
        assert!(
            body.contains("pipeline_events_quarantined_total{source=\"exporter-test\"} 1"),
            "{body}"
        );
    }

    #[tokio::test]
    async fn the_exporter_refuses_a_network_address() {
        assert!(matches!(
            serve("0.0.0.0:0".parse().unwrap()).await,
            Err(ExporterError::NotLoopback(_))
        ));
    }
}
