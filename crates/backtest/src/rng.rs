//! A tiny deterministic PRNG for Study sampling and trade-resampling.
//!
//! Studies must be reproducible: the same `seed` must produce the same member
//! configs and the same bootstrap resamples on every machine, forever. We use a
//! self-contained SplitMix64 generator rather than pulling in `rand` so the
//! sequence is pinned to this code and cannot drift with a dependency bump.

/// A reproducible 64-bit generator (SplitMix64).
#[derive(Clone, Debug)]
pub struct DetRng {
    state: u64,
}

impl DetRng {
    /// Seed the generator. Seed `0` is remapped so it never produces a
    /// degenerate all-zero stream.
    #[must_use]
    pub fn new(seed: u64) -> Self {
        Self {
            state: seed ^ 0x9E37_79B9_7F4A_7C15,
        }
    }

    /// Next raw 64-bit value.
    pub fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform `f64` in `[0, 1)`.
    pub fn next_f64(&mut self) -> f64 {
        // 53 bits of mantissa → exact uniform in [0,1).
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }

    /// Uniform integer in `[0, n)`. Returns `0` when `n == 0`.
    pub fn below(&mut self, n: usize) -> usize {
        if n == 0 {
            return 0;
        }
        (self.next_u64() % n as u64) as usize
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic_for_a_seed() {
        let a: Vec<u64> = {
            let mut r = DetRng::new(42);
            (0..8).map(|_| r.next_u64()).collect()
        };
        let b: Vec<u64> = {
            let mut r = DetRng::new(42);
            (0..8).map(|_| r.next_u64()).collect()
        };
        assert_eq!(a, b);
    }

    #[test]
    fn different_seeds_differ() {
        let mut r1 = DetRng::new(1);
        let mut r2 = DetRng::new(2);
        assert_ne!(r1.next_u64(), r2.next_u64());
    }

    #[test]
    fn f64_in_unit_interval() {
        let mut r = DetRng::new(7);
        for _ in 0..1000 {
            let x = r.next_f64();
            assert!((0.0..1.0).contains(&x));
        }
    }

    #[test]
    fn below_respects_bound() {
        let mut r = DetRng::new(9);
        for _ in 0..1000 {
            assert!(r.below(5) < 5);
        }
        assert_eq!(r.below(0), 0);
    }
}
