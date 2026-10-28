//! FxHash-style word-folding hasher (rustc-hash algorithm): deterministic,
//! no dependencies, ~5x faster than SipHash on struct-walk hashing, quality
//! plenty for 64-bit search keys and search-internal maps. Not DoS-resistant
//! — never use for attacker-controlled map keys.
//!
//! Lives in its own module so search-tree maps (`crates/bot`) can share the
//! hasher `Battle::state_key` already uses instead of paying SipHash on every
//! node lookup.

use std::collections::{HashMap, HashSet};
use std::hash::BuildHasherDefault;

#[derive(Default)]
pub struct WordHasher<const POLYNOMIAL: bool>(u64);

pub type FxHasher = WordHasher<false>;
/// Fixed-width polynomial state hash; persistent fingerprints use `FxHasher`.
pub type SearchHasher = WordHasher<true>;

impl WordHasher<true> {
    /// Append an already hashed sequence without changing its word stream:
    /// folding n words maps state s to s*K^n + fold(words, 0).
    #[inline]
    pub(crate) fn append_segment(&mut self, multiplier: u64, offset: u64) {
        self.0 = self.0.wrapping_mul(multiplier).wrapping_add(offset);
    }
}

impl<const POLYNOMIAL: bool> WordHasher<POLYNOMIAL> {
    const K: u64 = 0x51_7c_c1_b7_27_22_0a_95;
    pub const M: u64 = 0xf1357aea2e62a9c5;
    pub const M_POW: [u64; 65] = {
        let mut pows = [1u64; 65];
        let mut i = 1;
        while i < 65 {
            pows[i] = pows[i - 1].wrapping_mul(Self::M);
            i += 1;
        }
        pows
    };

    #[inline(always)]
    fn add(&mut self, word: u64) {
        self.0 = if POLYNOMIAL {
            self.0.wrapping_add(word).wrapping_mul(Self::M)
        } else {
            (self.0.rotate_left(5) ^ word).wrapping_mul(Self::K)
        };
    }

    /// Execute `f` into a stack word batch and fold all emitted words in parallel
    /// using `M_POW` when `POLYNOMIAL == true`.
    /// Because `f` invokes the exact stdlib `Hash` methods and `batch.idx` reduces
    /// to a compile-time constant after inlining, LLVM constant-folds all static
    /// words into a single additive constant and parallelizes all runtime words.
    #[inline(always)]
    pub fn batch<const CAP: usize>(&mut self, f: impl FnOnce(&mut PolyBatch<CAP>)) {
        let mut b = PolyBatch::<CAP> {
            words: [0u64; CAP],
            idx: 0,
        };
        f(&mut b);
        let k = b.idx;
        if POLYNOMIAL {
            let mut acc = self.0.wrapping_mul(Self::M_POW[k]);
            let mut i = 0;
            while i < k {
                acc = acc.wrapping_add(b.words[i].wrapping_mul(Self::M_POW[k - i]));
                i += 1;
            }
            self.0 = acc;
        } else {
            let mut i = 0;
            while i < k {
                self.add(b.words[i]);
                i += 1;
            }
        }
    }
}

pub struct PolyBatch<const CAP: usize> {
    pub words: [u64; CAP],
    pub idx: usize,
}

impl<const CAP: usize> PolyBatch<CAP> {
    #[inline(always)]
    fn push(&mut self, word: u64) {
        self.words[self.idx] = word;
        self.idx += 1;
    }
}

impl<const CAP: usize> std::hash::Hasher for PolyBatch<CAP> {
    #[inline(always)]
    fn finish(&self) -> u64 {
        0
    }

    #[inline(always)]
    fn write(&mut self, bytes: &[u8]) {
        let len = bytes.len();
        let mut i = 0;
        while i + 8 <= len {
            let w = u64::from_le_bytes([
                bytes[i],
                bytes[i + 1],
                bytes[i + 2],
                bytes[i + 3],
                bytes[i + 4],
                bytes[i + 5],
                bytes[i + 6],
                bytes[i + 7],
            ]);
            self.push(w);
            i += 8;
        }
        let rem = &bytes[i..];
        let rem_len = rem.len();
        if rem_len != 0 {
            let tail = match rem_len {
                1 => rem[0] as u64,
                2 => u16::from_le_bytes([rem[0], rem[1]]) as u64,
                3 => (u16::from_le_bytes([rem[0], rem[1]]) as u64) | ((rem[2] as u64) << 16),
                4 => u32::from_le_bytes([rem[0], rem[1], rem[2], rem[3]]) as u64,
                5 => {
                    (u32::from_le_bytes([rem[0], rem[1], rem[2], rem[3]]) as u64)
                        | ((rem[4] as u64) << 32)
                }
                6 => {
                    (u32::from_le_bytes([rem[0], rem[1], rem[2], rem[3]]) as u64)
                        | ((u16::from_le_bytes([rem[4], rem[5]]) as u64) << 32)
                }
                7 => {
                    (u32::from_le_bytes([rem[0], rem[1], rem[2], rem[3]]) as u64)
                        | ((u16::from_le_bytes([rem[4], rem[5]]) as u64) << 32)
                        | ((rem[6] as u64) << 48)
                }
                _ => unreachable!(),
            };
            self.push(tail ^ (rem_len as u64));
        }
    }

    #[inline(always)]
    fn write_u8(&mut self, i: u8) {
        self.push(i as u64);
    }

    #[inline(always)]
    fn write_u16(&mut self, i: u16) {
        self.push(i as u64);
    }

    #[inline(always)]
    fn write_u32(&mut self, i: u32) {
        self.push(i as u64);
    }

    #[inline(always)]
    fn write_u64(&mut self, i: u64) {
        self.push(i);
    }

    #[inline(always)]
    fn write_u128(&mut self, i: u128) {
        self.push(i as u64);
        self.push((i >> 64) as u64);
    }

    #[inline(always)]
    fn write_usize(&mut self, i: usize) {
        self.push(i as u64);
    }

    #[inline(always)]
    fn write_i8(&mut self, i: i8) {
        self.push(i as u8 as u64);
    }

    #[inline(always)]
    fn write_i16(&mut self, i: i16) {
        self.push(i as u16 as u64);
    }

    #[inline(always)]
    fn write_i32(&mut self, i: i32) {
        self.push(i as u32 as u64);
    }

    #[inline(always)]
    fn write_i64(&mut self, i: i64) {
        self.push(i as u64);
    }

    #[inline(always)]
    fn write_i128(&mut self, i: i128) {
        self.write_u128(i as u128);
    }

    #[inline(always)]
    fn write_isize(&mut self, i: isize) {
        self.push(i as usize as u64);
    }
}

/// Extension trait for state hashers supporting parallel polynomial folding on
/// fixed-structure blocks while falling back to the exact stdlib `Hash` walk
/// for non-polynomial hashers (`FxHasher`, `DefaultHasher`).
pub trait StateHasher: std::hash::Hasher + Sized {
    #[inline(always)]
    fn hash_batch<const CAP: usize>(
        &mut self,
        _poly: impl FnOnce(&mut PolyBatch<CAP>),
        fallback: impl FnOnce(&mut Self),
    ) {
        fallback(self);
    }
}

impl StateHasher for WordHasher<false> {}

impl StateHasher for WordHasher<true> {
    #[inline(always)]
    fn hash_batch<const CAP: usize>(
        &mut self,
        poly: impl FnOnce(&mut PolyBatch<CAP>),
        _fallback: impl FnOnce(&mut Self),
    ) {
        self.batch(poly);
    }
}

impl StateHasher for std::collections::hash_map::DefaultHasher {}

pub(crate) struct PlainHasher<'a, H>(pub &'a mut H);

macro_rules! forward_plain_words {
    ($($method:ident: $ty:ty),* $(,)?) => { $(
        #[inline(always)]
        fn $method(&mut self, value: $ty) {
            self.0.$method(value);
        }
    )* };
}

impl<H: std::hash::Hasher> std::hash::Hasher for PlainHasher<'_, H> {
    #[inline(always)]
    fn finish(&self) -> u64 {
        self.0.finish()
    }

    #[inline(always)]
    fn write(&mut self, bytes: &[u8]) {
        self.0.write(bytes);
    }

    forward_plain_words! {
        write_u8:u8, write_u16:u16, write_u32:u32, write_u64:u64, write_u128:u128, write_usize:usize,
        write_i8:i8, write_i16:i16, write_i32:i32, write_i64:i64, write_i128:i128, write_isize:isize,
    }
}

impl<H: std::hash::Hasher> StateHasher for PlainHasher<'_, H> {}

impl<const POLYNOMIAL: bool> std::hash::Hasher for WordHasher<POLYNOMIAL> {
    #[inline(always)]
    fn finish(&self) -> u64 {
        if POLYNOMIAL {
            self.0.rotate_left(26)
        } else {
            self.0
        }
    }

    #[inline(always)]
    fn write(&mut self, bytes: &[u8]) {
        let len = bytes.len();
        let mut i = 0;
        while i + 8 <= len {
            let w = u64::from_le_bytes([
                bytes[i],
                bytes[i + 1],
                bytes[i + 2],
                bytes[i + 3],
                bytes[i + 4],
                bytes[i + 5],
                bytes[i + 6],
                bytes[i + 7],
            ]);
            self.add(w);
            i += 8;
        }
        let rem = &bytes[i..];
        let rem_len = rem.len();
        if rem_len != 0 {
            let tail = match rem_len {
                1 => rem[0] as u64,
                2 => u16::from_le_bytes([rem[0], rem[1]]) as u64,
                3 => (u16::from_le_bytes([rem[0], rem[1]]) as u64) | ((rem[2] as u64) << 16),
                4 => u32::from_le_bytes([rem[0], rem[1], rem[2], rem[3]]) as u64,
                5 => {
                    (u32::from_le_bytes([rem[0], rem[1], rem[2], rem[3]]) as u64)
                        | ((rem[4] as u64) << 32)
                }
                6 => {
                    (u32::from_le_bytes([rem[0], rem[1], rem[2], rem[3]]) as u64)
                        | ((u16::from_le_bytes([rem[4], rem[5]]) as u64) << 32)
                }
                7 => {
                    (u32::from_le_bytes([rem[0], rem[1], rem[2], rem[3]]) as u64)
                        | ((u16::from_le_bytes([rem[4], rem[5]]) as u64) << 32)
                        | ((rem[6] as u64) << 48)
                }
                _ => unreachable!(),
            };
            self.add(tail ^ (rem_len as u64));
        }
    }

    #[inline(always)]
    fn write_u8(&mut self, i: u8) {
        self.add(i as u64);
    }

    #[inline(always)]
    fn write_u16(&mut self, i: u16) {
        self.add(i as u64);
    }

    #[inline(always)]
    fn write_u32(&mut self, i: u32) {
        self.add(i as u64);
    }

    #[inline(always)]
    fn write_u64(&mut self, i: u64) {
        self.add(i);
    }

    #[inline(always)]
    fn write_u128(&mut self, i: u128) {
        self.add(i as u64);
        self.add((i >> 64) as u64);
    }

    #[inline(always)]
    fn write_usize(&mut self, i: usize) {
        self.add(i as u64);
    }

    #[inline(always)]
    fn write_i8(&mut self, i: i8) {
        self.add(i as u8 as u64);
    }

    #[inline(always)]
    fn write_i16(&mut self, i: i16) {
        self.add(i as u16 as u64);
    }

    #[inline(always)]
    fn write_i32(&mut self, i: i32) {
        self.add(i as u32 as u64);
    }

    #[inline(always)]
    fn write_i64(&mut self, i: i64) {
        self.add(i as u64);
    }

    #[inline(always)]
    fn write_i128(&mut self, i: i128) {
        self.write_u128(i as u128);
    }

    #[inline(always)]
    fn write_isize(&mut self, i: isize) {
        self.add(i as usize as u64);
    }
}

pub type FxBuildHasher = BuildHasherDefault<FxHasher>;
pub type FxHashMap<K, V> = HashMap<K, V, FxBuildHasher>;
pub type FxHashSet<T> = HashSet<T, FxBuildHasher>;
