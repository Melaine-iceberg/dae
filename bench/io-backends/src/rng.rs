//! A deterministic PRNG, so the random-read offsets are identical across
//! backends and across repeats. A real RNG crate would only add a dependency:
//! nothing here needs statistical quality, it needs reproducibility.
//!
//! SplitMix64 (Steele et al., used as the seeding step of xoshiro).

pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        // Any odd seed works; mixing it once keeps low-entropy seeds (0, 1, 2)
        // from producing correlated first outputs.
        Self(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1)
    }

    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform-ish value in `0..bound`. The modulo bias is irrelevant at these
    /// bounds and the operation is one instruction.
    pub fn below(&mut self, bound: u64) -> u64 {
        if bound == 0 {
            0
        } else {
            self.next_u64() % bound
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Rng;

    #[test]
    fn is_deterministic_and_in_bounds() {
        let mut a = Rng::new(7);
        let mut b = Rng::new(7);
        for _ in 0..1000 {
            // Every draw has to advance both streams the same number of times,
            // which is the whole point: the same pass must offer every backend
            // the same offsets. An assertion that draws from one stream only
            // would desynchronise them (and did, before this comment).
            assert_eq!(a.next_u64(), b.next_u64());
            let (left, right) = (a.below(4096), b.below(4096));
            assert_eq!(left, right);
            assert!(left < 4096);
            assert_eq!((a.below(0), b.below(0)), (0, 0));
        }
    }
}
