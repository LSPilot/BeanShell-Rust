//! Small, fast non-cryptographic hasher (rustc-hash style) for the hot
//! variable maps. Script variable names are short and trusted, so SipHash is
//! unnecessary overhead on every lookup.

use std::collections::HashMap;
use std::hash::{BuildHasherDefault, Hasher};

#[derive(Default)]
pub struct FxHasher {
    hash: u64,
}

const SEED: u64 = 0x51_7c_c1_b7_27_22_0a_95;

impl FxHasher {
    #[inline]
    fn add(&mut self, value: u64) {
        self.hash = (self.hash.rotate_left(5) ^ value).wrapping_mul(SEED);
    }
}

impl Hasher for FxHasher {
    #[inline]
    fn write(&mut self, bytes: &[u8]) {
        let mut chunks = bytes.chunks_exact(8);
        for chunk in &mut chunks {
            let mut value = 0u64;
            for (i, byte) in chunk.iter().enumerate() {
                value |= (*byte as u64) << (8 * i);
            }
            self.add(value);
        }
        let rest = chunks.remainder();
        if !rest.is_empty() {
            let mut value = 0u64;
            for (i, byte) in rest.iter().enumerate() {
                value |= (*byte as u64) << (8 * i);
            }
            self.add(value);
        }
    }

    #[inline]
    fn write_u8(&mut self, value: u8) {
        self.add(value as u64);
    }

    #[inline]
    fn write_usize(&mut self, value: usize) {
        self.add(value as u64);
    }

    #[inline]
    fn finish(&self) -> u64 {
        self.hash
    }
}

pub type FxBuildHasher = BuildHasherDefault<FxHasher>;
pub type FxMap<K, V> = HashMap<K, V, FxBuildHasher>;
