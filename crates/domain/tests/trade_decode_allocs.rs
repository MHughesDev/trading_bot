//! Set C metric: heap allocations and latency of decoding one trade tick.
//!
//! The hot path's stage 2 calls `EventEnvelope::decode_payload::<TradePayload>()`
//! once per tick. This crate has `rkyv`, so it is the place that can build an
//! encoded tick and count what the decode costs. Printed with `--nocapture`.
//!
//! Own test binary, own allocator: the counter is thread-local, so a measurement
//! is not polluted by anything else running in parallel.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

use domain::{
    intern_instrument, intern_source, intern_venue,
    payloads::trade::{TradePayload, TradeSide},
    EventEnvelope,
};

thread_local! {
    static ALLOCS: Cell<u64> = const { Cell::new(0) };
}

struct Counting;

// SAFETY: delegates every call to `System`; the counter is a const-init
// thread-local `Cell`, which never allocates.
#[allow(unsafe_code)]
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        let _ = ALLOCS.try_with(|c| c.set(c.get() + 1));
        System.alloc(l)
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        System.dealloc(p, l)
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, n: usize) -> *mut u8 {
        let _ = ALLOCS.try_with(|c| c.set(c.get() + 1));
        System.realloc(p, l, n)
    }
}

#[global_allocator]
static COUNTING: Counting = Counting;

fn count() -> u64 {
    ALLOCS.with(Cell::get)
}

fn tick(trade_id: &str) -> EventEnvelope {
    let payload = TradePayload::new(
        "50000.25".parse().unwrap(),
        "0.01".parse().unwrap(),
        TradeSide::Buy,
        trade_id,
    );
    let bytes = rkyv::to_bytes::<rkyv::rancor::Error>(&payload)
        .unwrap()
        .into_vec();
    EventEnvelope::new(
        intern_instrument("BTC-USD"),
        intern_venue("kraken"),
        intern_source("kraken_ws"),
        1,
        1_700_000_000_000_000_000,
        bytes,
    )
}

#[test]
fn decoding_a_trade_tick_costs_one_allocation_for_its_trade_id() {
    const N: u64 = 20_000;
    // A real venue's trade id is a non-empty string; an empty one allocates nothing.
    let with_id = tick("1234567890");
    let without_id = tick("");

    let mut lat = Vec::with_capacity(N as usize);
    let a0 = count();
    for _ in 0..N {
        let t0 = std::time::Instant::now();
        let p = with_id.decode_payload::<TradePayload>().unwrap();
        lat.push(t0.elapsed().as_nanos() as u64);
        std::hint::black_box(p);
    }
    let per_tick = (count() - a0) as f64 / N as f64;

    let b0 = count();
    for _ in 0..N {
        std::hint::black_box(without_id.decode_payload::<TradePayload>().unwrap());
    }
    let per_tick_no_id = (count() - b0) as f64 / N as f64;

    lat.sort_unstable();
    let p = |q: f64| lat[((N as f64 * q) as usize).min(lat.len() - 1)] as f64 / 1000.0;
    println!(
        "SET-C DECODE profile={} ticks={N} allocs_per_decode={per_tick:.2} allocs_per_decode_empty_id={per_tick_no_id:.2} decode_us p50={:.2} p99={:.2}",
        if cfg!(debug_assertions) { "debug" } else { "release" },
        p(0.50),
        p(0.99),
    );
    // The only heap object a decode may create is the trade-id string. The
    // dedup key is a u64 (xxh3), the prices are inline decimals.
    assert!(
        per_tick <= 1.0,
        "decode allocates {per_tick} times per tick; budget is one (the trade id)"
    );
    assert!(
        per_tick_no_id < 0.01,
        "a decode with an empty trade id must not allocate"
    );
}
