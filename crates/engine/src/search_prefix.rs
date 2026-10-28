//! Exact, bounded memoization of the expensive Pokemon hash prefix.
//!
//! A hit compares every input field, including unused inline move slots.
//! It needs no mutation hooks and remains correct after direct public-field
//! writes, determinization, Transform, and switching between battles/dexes.
use std::cell::RefCell;
use std::hash::{Hash, Hasher};

use crate::dex::{SpeciesId, TypeId};
use crate::fxhash::SearchHasher;
use crate::state::{Gender, MoveSlots, PokeName};

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct PokemonHashPrefix {
    pub species: SpeciesId,
    pub base_species: SpeciesId,
    pub name: PokeName,
    pub level: u8,
    pub gender: Gender,
    pub happiness: u8,
    pub set_ivs: [i32; 6],
    pub set_evs: [i32; 6],
    pub base_move_slots: MoveSlots,
    pub hp_type: TypeId,
    pub hp_power: i32,
    pub base_hp_type: TypeId,
    pub base_hp_power: i32,
    pub base_stored_stats: [i32; 5],
    pub stored_stats: [i32; 5],
    pub base_maxhp: i32,
    pub maxhp: i32,
}

#[derive(Default)]
struct CountedHasher {
    inner: SearchHasher,
    words: u32,
}

macro_rules! counted_words {
    ($($method:ident: $ty:ty => $count:expr),* $(,)?) => { $(
        #[inline]
        fn $method(&mut self, value: $ty) {
            self.inner.$method(value);
            self.words += $count;
        }
    )* };
}

impl Hasher for CountedHasher {
    fn finish(&self) -> u64 {
        self.inner.finish()
    }
    #[inline]
    fn write(&mut self, bytes: &[u8]) {
        self.inner.write(bytes);
        self.words += bytes.len().div_ceil(8) as u32;
    }
    // Delegate signed methods too: WordHasher's default signed encoding
    // is not interchangeable with a cast followed by an unsigned write.
    counted_words! {
        write_u8:u8=>1, write_u16:u16=>1, write_u32:u32=>1,
        write_u64:u64=>1, write_u128:u128=>2, write_usize:usize=>1,
        write_i8:i8=>1, write_i16:i16=>1, write_i32:i32=>1,
        write_i64:i64=>1, write_i128:i128=>2, write_isize:isize=>1,
    }
}

#[derive(Clone, Copy)]
struct Entry {
    prefix: PokemonHashPrefix,
    multiplier: u64,
    offset: u64,
}

pub(crate) struct PrefixCache {
    entries: [Option<Entry>; 12],
    cursor: usize,
}

impl PrefixCache {
    const fn new() -> Self {
        Self {
            entries: [None; 12],
            cursor: 0,
        }
    }

    pub(crate) fn hash(&mut self, prefix: &PokemonHashPrefix, h: &mut SearchHasher) {
        let slot = self.cursor % self.entries.len();
        self.cursor += 1;
        let entry = &mut self.entries[slot];
        if !entry.as_ref().is_some_and(|old| old.prefix == *prefix) {
            let mut segment = CountedHasher::default();
            prefix.hash(&mut segment);
            *entry = Some(Entry {
                prefix: *prefix,
                multiplier: 0xf1357aea2e62a9c5u64.wrapping_pow(segment.words),
                offset: segment.finish().rotate_right(26),
            });
        }
        let entry = entry.as_ref().unwrap();
        h.append_segment(entry.multiplier, entry.offset);
    }
}

thread_local! {
    static CACHE: RefCell<PrefixCache> = const { RefCell::new(PrefixCache::new()) };
}

pub(crate) fn with_cache<T>(f: impl FnOnce(Option<&mut PrefixCache>) -> T) -> T {
    CACHE.with(|cell| match cell.try_borrow_mut() {
        Ok(mut cache) => {
            cache.cursor = 0;
            f(Some(&mut cache))
        }
        Err(_) => f(None),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dex::MoveId;
    use crate::state::MoveSlot;

    #[test]
    fn counted_segments_preserve_every_hasher_write_and_nonzero_prefix() {
        for len in 0..65 {
            let mut direct = SearchHasher::default();
            direct.write_u64(0x8765432112345678);
            let mut combined = SearchHasher::default();
            combined.write_u64(0x8765432112345678);
            let mut segment = CountedHasher::default();
            macro_rules! feed {
                ($method:ident, $value:expr) => {{
                    direct.$method($value);
                    segment.$method($value);
                }};
            }
            let bytes: Vec<_> = (0..len).map(|i| (i * 73 + len) as u8).collect();
            feed!(write, &bytes);
            feed!(write_u8, len as u8);
            feed!(write_i8, -(len as i8));
            feed!(write_u16, u16::MAX);
            feed!(write_i16, i16::MIN);
            feed!(write_u32, u32::MAX);
            feed!(write_i32, i32::MIN);
            feed!(write_u64, u64::MAX);
            feed!(write_i64, i64::MIN);
            feed!(write_u128, u128::MAX);
            feed!(write_i128, i128::MIN);
            feed!(write_usize, usize::MAX);
            feed!(write_isize, isize::MIN);
            combined.append_segment(
                0xf1357aea2e62a9c5u64.wrapping_pow(segment.words),
                segment.finish().rotate_right(26),
            );
            direct.write_u64(19);
            combined.write_u64(19);
            assert_eq!(direct.finish(), combined.finish(), "length {len}");
        }
    }

    fn prefix() -> PokemonHashPrefix {
        let mut slots = MoveSlots::default();
        slots.push(MoveSlot {
            id: MoveId(1),
            pp: 5,
            maxpp: 10,
            ..Default::default()
        });
        PokemonHashPrefix {
            species: SpeciesId(1),
            base_species: SpeciesId(1),
            name: PokeName::new("test"),
            level: 50,
            gender: Gender::M,
            happiness: 255,
            set_ivs: [30; 6],
            set_evs: [255; 6],
            base_move_slots: slots,
            hp_type: TypeId(0),
            hp_power: 70,
            base_hp_type: TypeId(0),
            base_hp_power: 70,
            base_stored_stats: [100; 5],
            stored_stats: [100; 5],
            base_maxhp: 200,
            maxhp: 200,
        }
    }

    #[test]
    fn cache_rechecks_all_prefix_fields_and_handles_eviction() {
        let base = prefix();
        let mut variants = vec![base];
        macro_rules! change {
            ($field:ident, $value:expr) => {{
                let mut changed = base;
                changed.$field = $value;
                variants.push(changed);
            }};
        }
        change!(species, SpeciesId(2));
        change!(base_species, SpeciesId(2));
        change!(name, PokeName::new("other"));
        change!(level, 51);
        change!(gender, Gender::F);
        change!(happiness, 0);
        change!(set_ivs, [31; 6]);
        change!(set_evs, [0; 6]);
        change!(base_move_slots, MoveSlots::default());
        change!(hp_type, TypeId(1));
        change!(hp_power, 69);
        change!(base_hp_type, TypeId(2));
        change!(base_hp_power, 68);
        change!(base_stored_stats, [99; 5]);
        change!(stored_stats, [102; 5]);
        change!(base_maxhp, 201);
        change!(maxhp, 202);
        for i in 0..base.base_move_slots.len() {
            let mut slots = base.base_move_slots;
            slots[i].pp -= 1;
            change!(base_move_slots, slots);
        }
        let mut cache = PrefixCache::new();
        for round in 0..4 {
            // Alternate repeated hits, single-slot replacements, and wrapping
            // the fixed capacity while switching between different states.
            for (i, value) in variants.iter().chain(variants.iter().rev()).enumerate() {
                if round % 2 == 0 {
                    cache.cursor = 0;
                }
                let mut direct = SearchHasher::default();
                direct.write_u64(i as u64);
                value.hash(&mut direct);
                let mut cached = SearchHasher::default();
                cached.write_u64(i as u64);
                cache.hash(value, &mut cached);
                assert_eq!(direct.finish(), cached.finish());
            }
        }
    }

    #[test]
    fn reentrant_use_falls_back_without_corrupting_the_outer_cache() {
        with_cache(|outer| {
            assert!(outer.is_some());
            with_cache(|inner| assert!(inner.is_none()));
            let mut cached = SearchHasher::default();
            outer.unwrap().hash(&prefix(), &mut cached);
            let mut direct = SearchHasher::default();
            prefix().hash(&mut direct);
            assert_eq!(cached.finish(), direct.finish());
        });
    }
}
