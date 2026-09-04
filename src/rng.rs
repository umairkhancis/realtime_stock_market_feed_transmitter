//! A seeded PRNG, because `rand` is a dependency and the brief forbids those.
//!
//! This is SplitMix64 — the finalizer Java's `SplittableRandom` uses. Sixty-four
//! bits of state, one multiply-xor-shift chain per output, passes BigCrush.
//! It is not cryptographic and does not need to be: the only requirement is
//! that a given seed always produces the same market, so a CSV and the stream
//! generated from the same seed are the same data.

/// Deterministic, seedable, std-only.
#[derive(Debug, Clone)]
pub struct Rng {
    state: u64,
}

impl Rng {
    pub fn new(seed: u64) -> Self {
        // Avoid the all-zero state producing a degenerate first output.
        Rng {
            state: seed ^ 0x9E37_79B9_7F4A_7C15,
        }
    }

    #[inline]
    pub fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform in `[0, n)`. Lemire's multiply-shift — no modulo, no rejection
    /// loop; the residual bias is below 2^-64 relative and irrelevant here.
    #[inline]
    pub fn below(&mut self, n: u64) -> u64 {
        debug_assert!(n > 0, "below(0) has no valid output");
        ((self.next_u64() as u128 * n as u128) >> 64) as u64
    }

    /// Uniform in `[lo, hi]`, inclusive.
    #[inline]
    pub fn range(&mut self, lo: u64, hi: u64) -> u64 {
        debug_assert!(lo <= hi);
        lo + self.below(hi - lo + 1)
    }

    /// Uniform in `[0.0, 1.0)`, using the top 53 bits (an f64 mantissa).
    #[inline]
    pub fn unit(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 * (1.0 / (1u64 << 53) as f64)
    }

    /// `true` with probability `p`.
    #[inline]
    pub fn chance(&mut self, p: f64) -> bool {
        self.unit() < p
    }

    /// Standard normal, N(0, 1), via Box–Muller.
    ///
    /// Box–Muller wastes one of its two outputs, which costs nothing at this
    /// scale and keeps the generator stateless beyond `state` — caching the
    /// spare would make the output depend on how many times you'd called it
    /// before, which is a nasty thing to debug when a seed stops reproducing.
    pub fn normal(&mut self) -> f64 {
        // `ln(0)` is -inf; nudge off zero rather than looping.
        let u1 = self.unit().max(f64::MIN_POSITIVE);
        let u2 = self.unit();
        (-2.0 * u1.ln()).sqrt() * (std::f64::consts::TAU * u2).cos()
    }

    /// Picks an index in `[0, weights.len())` proportional to `weights`.
    /// Returns 0 if every weight is zero.
    pub fn weighted_index(&mut self, weights: &[f64]) -> usize {
        let total: f64 = weights.iter().sum();
        if !(total > 0.0) {
            return 0;
        }
        let mut target = self.unit() * total;
        for (i, &w) in weights.iter().enumerate() {
            target -= w;
            if target < 0.0 {
                return i;
            }
        }
        weights.len() - 1 // float rounding fell off the end
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_seed_same_stream() {
        let mut a = Rng::new(42);
        let mut b = Rng::new(42);
        let mut c = Rng::new(43);
        let sa: Vec<u64> = (0..64).map(|_| a.next_u64()).collect();
        let sb: Vec<u64> = (0..64).map(|_| b.next_u64()).collect();
        let sc: Vec<u64> = (0..64).map(|_| c.next_u64()).collect();
        assert_eq!(sa, sb, "a seed must reproduce its stream exactly");
        assert_ne!(sa, sc);
    }

    #[test]
    fn zero_seed_is_not_degenerate() {
        let mut r = Rng::new(0);
        let out: Vec<u64> = (0..8).map(|_| r.next_u64()).collect();
        assert!(out.iter().all(|&x| x != 0));
        assert!(out.windows(2).all(|w| w[0] != w[1]));
    }

    #[test]
    fn below_stays_in_range_and_covers_it() {
        let mut r = Rng::new(7);
        let mut seen = [false; 6];
        for _ in 0..10_000 {
            let v = r.below(6);
            assert!(v < 6);
            seen[v as usize] = true;
        }
        assert!(
            seen.iter().all(|&s| s),
            "every bucket should be hit in 10k draws"
        );
        assert_eq!(r.below(1), 0);
    }

    #[test]
    fn range_is_inclusive_on_both_ends() {
        let mut r = Rng::new(9);
        let mut lo_hit = false;
        let mut hi_hit = false;
        for _ in 0..10_000 {
            let v = r.range(10, 20);
            assert!((10..=20).contains(&v));
            lo_hit |= v == 10;
            hi_hit |= v == 20;
        }
        assert!(lo_hit && hi_hit);
        assert_eq!(r.range(5, 5), 5);
    }

    #[test]
    fn unit_is_in_the_half_open_interval() {
        let mut r = Rng::new(11);
        let mut sum = 0.0;
        const N: usize = 100_000;
        for _ in 0..N {
            let u = r.unit();
            assert!((0.0..1.0).contains(&u));
            sum += u;
        }
        let mean = sum / N as f64;
        assert!((mean - 0.5).abs() < 0.01, "mean {mean} is not ~0.5");
    }

    #[test]
    fn normal_has_the_right_moments() {
        let mut r = Rng::new(13);
        const N: usize = 200_000;
        let (mut sum, mut sq) = (0.0f64, 0.0f64);
        for _ in 0..N {
            let z = r.normal();
            assert!(z.is_finite(), "Box-Muller produced {z}");
            sum += z;
            sq += z * z;
        }
        let mean = sum / N as f64;
        let var = sq / N as f64 - mean * mean;
        assert!(mean.abs() < 0.02, "mean {mean} is not ~0");
        assert!((var - 1.0).abs() < 0.03, "variance {var} is not ~1");
    }

    #[test]
    fn weighted_index_respects_the_weights() {
        let mut r = Rng::new(17);
        let weights = [1.0, 3.0, 0.0];
        let mut counts = [0usize; 3];
        const N: usize = 100_000;
        for _ in 0..N {
            counts[r.weighted_index(&weights)] += 1;
        }
        assert_eq!(counts[2], 0, "a zero weight must never be picked");
        let ratio = counts[1] as f64 / counts[0] as f64;
        assert!((ratio - 3.0).abs() < 0.1, "ratio {ratio} is not ~3");
        assert_eq!(
            r.weighted_index(&[0.0, 0.0]),
            0,
            "all-zero weights must not panic"
        );
    }
}
