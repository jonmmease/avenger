//! Helpers that the tests share.

#![allow(dead_code)]

/// A deterministic pseudo-random sequence, by xorshift from a nonzero seed.
pub struct Noise(pub u64);

impl Noise {
    fn step(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    /// The next number in [0, 1).
    pub fn unit(&mut self) -> f64 {
        (self.step() >> 11) as f64 / (1u64 << 53) as f64
    }

    /// The next integer below `below`.
    pub fn below(&mut self, below: u64) -> u64 {
        self.step() % below
    }
}
