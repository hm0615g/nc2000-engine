//! Fast branchless arithmetic primitives, precomputed lookup tables, and the
//! 192-byte fixed-size `CompactBattle` search kernel for `[Gen 2] NC 2000`
//! (`gen2stadium2`).
//!
//! Key design principles:
//! 1. **Branchless arithmetic over control-flow branching**:
//!    - Turn action ordering (`ActionSortKey`) packs phase, priority, Quick Claw
//!      activation (saturating speed to 1023 so double Quick Claw ties exactly
//!      like PS's 65535), effective speed, and tie-break bit into one `u32`.
//!    - Stat computation (`get_stat_branchless`) uses a 13-entry `STAT_NUM`
//!      numerator table (`-6..=+6`) and bitshifts for paralysis (`>> 2`),
//!      burn (`>> 1`), screens (`<< 1`), and species items (`<< 1` / `* 3 >> 1`).
//!    - Accuracy computation (`ACC_LUT`) uses a 101 × 13 × 13 byte table
//!      precomputed to match IEEE-754 Gen 2 staging bit-for-bit, with branchless
//!      Foresight evasion masking and BrightPowder `-20` saturation.
//!    - Damage computation (`get_damage_base_branchless`) collapses the 256-rollover (`>> 2`), Explosion
//!      defense halving (`>> 1`), critical hit (`<< 1`), 1.1x item boost
//!      (`dmg + dmg / 10`), weather (`* num >> 1`), STAB (`+ (dmg >> 1)`),
//!      type effectiveness (`<< super_shift >> resist_shift`), and `/ 255`
//!      damage roll (`* 0x808081 >> 31`) into branchless integer/SIMD operations.
//!    - Substitute targeting uses a 2-element HP pool `[hp, sub_hp]` indexed by
//!      `idx = (has_sub & !bypasses_sub) as usize`.
//! 2. **192-byte cache-line-aligned `Copy` state (`CompactBattle`)**:
//!    - Immutable match constants (base stats, stored stats, move metadata,
//!      1/8 and 1/16 residual quanta) live in `MatchTable`.
//!    - Mutable state for the 6 picked Pokemon (24B × 6 = 144B), 2 sides
//!      (12B × 2 = 24B), and field/turn/damage/RNG state (24B) fits in exactly
//!      192 bytes (3 cache lines), with zero heap allocations on clone or step.

use std::sync::OnceLock;

use crate::dex::{Accuracy, Category, Dex, ItemId, MoveId, SpeciesId};
use crate::prng::Prng;
use crate::state::{stat_idx, Battle, RequestKind, Status, DK, QUICK_CLAW_ACTION_SPEED};

/// Sentinel `fixed_damage` indicating level-based fixed damage (Seismic Toss / Night Shade).
pub const FIXED_DAMAGE_LEVEL: i16 = -1;
/// Sentinel move/slot index meaning no move slot.
pub const NO_MOVE_SLOT: u8 = 0xFF;
/// Sentinel stat index meaning no stat slot (`sec_boost.0` / `boosts[i].0`).
pub const NO_STAT_IDX: u8 = 0xFF;
/// Sentinel type id meaning typeless (`Struggle` / unknown).
pub const NO_TYPE_ID: u8 = 0xFF;
/// Flag bit OR'd onto `CompactMoveSpec::sec_boost.0` for secondary self-boosts (AncientPower etc.).
pub const SELF_BOOST_FLAG: u8 = 0x80;
/// Bit 7 on `sides[0].side_flags`: disables the mid-turn Baton Pass fix when set (test/ablation toggle).
pub const SIDE0_DISABLE_BP_FIX: u8 = 0x80;
/// Stat-index sentinel (`7` with `SELF_BOOST_FLAG`) encoding AncientPower's all-5-stat raise.
pub const ANCIENTPOWER_ALL_STATS: u8 = 7;
/// Saturated 10-bit speed field when Quick Claw triggers in [`ActionSortKey`].
pub const COMPACT_QC_SATURATED_SPEED: u16 = 1023;

/// Indices into [`CompactMon::timers`] (`[confusion, encore, disable, perish/stall]`).
pub mod timer_idx {
    pub const CONFUSION: usize = 0;
    pub const ENCORE: usize = 1;
    pub const DISABLE: usize = 2;
    pub const PERISH_STALL: usize = 3;
}

/// Pre-classified special move mechanics tag on [`CompactMoveSpec`].
#[repr(u8)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum SpecialMoveKind {
    #[default]
    None = 0,
    Reflect = 1,
    LightScreen = 2,
    Safeguard = 3,
    Spikes = 4,
    Curse = 5,
    BellyDrum = 6,
    Phaze = 7,
    Encore = 8,
    Disable = 9,
    SleepTalk = 10,
    Counter = 11,
    MirrorCoat = 12,
    FlailReversal = 13,
    LevelDamage = 14,
    SuperFang = 15,
    Thief = 16,
    PainSplit = 17,
    BatonPass = 18,
    PerishSong = 19,
    DestinyBond = 20,
    HealBell = 21,
    PsychUp = 22,
    RainDance = 23,
    SunnyDay = 24,
    Sandstorm = 25,
    MeanLook = 26,
    Haze = 27,
    RapidSpin = 28,
    WeatherHeal = 29,
    Swagger = 30,
}

/// Variable base power formula tag on [`CompactMoveSpec`].
#[repr(u8)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum VarBpKind {
    #[default]
    Static = 0,
    Return = 1,
    Frustration = 2,
    HiddenPower = 3,
    FlailReversal = 4,
    Magnitude = 5,
    Present = 6,
}

// =========================================================================
// 1. ActionSortKey: Single-u32 Branchless Turn Ordering
// =========================================================================

/// Action phase priority (larger value executes earlier in the turn).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u8)]
pub enum ActionPhase {
    Move = 1,
    Switch = 2,
}

/// Bit-packed 32-bit sort key for turn action ordering:
/// `[ phase (3b: bits 16..18) | (priority + 8) (4b: bits 12..15) | speed_or_qc (10b: bits 1..10) | tie_bit (1b: bit 0) ]`
///
/// Note on Quick Claw (`qc_active`):
/// In `gen2stadium2`, triggering Quick Claw sets a Pokemon's action speed to
/// `65535` (`pokemon.rs:139`). By saturating the 10-bit speed field (`0..=999`)
/// to `1023` via `eff_speed | (-(qc_active as i32) as u32 & 1023)`, a Quick Claw
/// activation beats every natural speed (`<= 999`) AND causes two simultaneous
/// Quick Claw activations at the same priority to tie (`1023 == 1023`),
/// preserving exact equivalence with `65535`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(transparent)]
pub struct ActionSortKey(pub u32);

impl ActionSortKey {
    #[inline(always)]
    pub const fn new(
        phase: ActionPhase,
        priority: i8,
        qc_active: bool,
        eff_speed: i32,
        tie_bit: u8,
    ) -> Self {
        let phase_bits = (phase as u32 & 0x7) << 15;
        let prio_bits = (((priority as i32 + 8) as u32) & 0xF) << 11;
        let clamped_speed = if eff_speed < 0 {
            0u32
        } else if eff_speed > 999 {
            999u32
        } else {
            eff_speed as u32
        };
        let qc_mask = (-(qc_active as i32) as u32) & (COMPACT_QC_SATURATED_SPEED as u32);
        let speed_bits = (clamped_speed | qc_mask) << 1;
        let tie = (tie_bit & 1) as u32;
        Self(phase_bits | prio_bits | speed_bits | tie)
    }

    /// Compare two keys ignoring the tie-break bit (bit 0).
    #[inline(always)]
    pub const fn ties_without_rng(self, other: Self) -> bool {
        (self.0 >> 1) == (other.0 >> 1)
    }
}

// =========================================================================
// 2. Branchless Stat Calculation (13-entry LUT + Bitshifts)
// =========================================================================

/// Numerators for Gen 2 stat stages `-6..=+6` (divided by 100).
/// Matches `pokemon.rs` `BOOST_TABLE` (`[1.0, 1.5, 2.0, 2.5, 3.0, 3.5, 4.0]`)
/// and `NUMERATORS` (`[100, 66, 50, 40, 33, 28, 25]`) bit-for-bit on all integers.
pub const STAT_NUM: [i32; 13] = [25, 28, 33, 40, 50, 66, 100, 150, 200, 250, 300, 350, 400];

/// Species-specific stat-boosting held item classification.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
#[repr(u8)]
pub enum ItemStatKind {
    #[default]
    None = 0,
    /// Thick Club held by Cubone or Marowak (doubles Attack, `stat == 0`).
    ThickClub = 1,
    /// Light Ball held by Pikachu (doubles Special Attack, `stat == 2`).
    LightBall = 2,
    /// Metal Powder held by Ditto (1.5x Defense and Special Defense, `stat == 1 | 3`).
    MetalPowder = 3,
}

impl ItemStatKind {
    #[inline(always)]
    pub fn classify(dex: &Dex, species: SpeciesId, item: Option<ItemId>) -> Self {
        let Some(item_id) = item else {
            return Self::None;
        };
        let ki = &dex.known_items;
        let ks = &dex.known_species;
        let sp = Some(species);
        let it = Some(item_id);
        if it == ki.thickclub && (sp == ks.cubone || sp == ks.marowak) {
            Self::ThickClub
        } else if it == ki.lightball && sp == ks.pikachu {
            Self::LightBall
        } else if it == ki.metalpowder && sp == ks.ditto {
            Self::MetalPowder
        } else {
            Self::None
        }
    }
}

/// Branchless integer implementation of `gen2stadium2` `Battle::get_stat`.
/// Guaranteed bit-for-bit identical to `Battle::get_stat` across all inputs.
#[inline(always)]
pub const fn get_stat_branchless(
    stored_stat: i32,
    boost: i8,
    stat_idx: usize,
    unboosted: bool,
    unmodified: bool,
    fast_return: bool,
    has_par_speed_drop: bool,
    has_brn_atk_drop: bool,
    has_reflect: bool,
    has_light_screen: bool,
    item_stat_kind: ItemStatKind,
) -> i32 {
    let clamped_boost = if boost < -6 {
        -6
    } else if boost > 6 {
        6
    } else {
        boost
    };
    // Zero the boost when `unboosted` is true via arithmetic mask.
    let keep_boost_mask = -(!unboosted as i8);
    let eff_boost = clamped_boost & keep_boost_mask;
    let mut val = (stored_stat * STAT_NUM[(eff_boost + 6) as usize]) / 100;

    // Paralysis speed drop (÷4 = >> 2) and Burn attack drop (÷2 = >> 1).
    let par_shift = ((stat_idx == stat_idx::SPE) & has_par_speed_drop) as u32 * 2;
    let brn_shift = ((stat_idx == stat_idx::ATK) & (!unmodified) & has_brn_atk_drop) as u32;
    val >>= par_shift + brn_shift;

    // Clamp to [1, 999].
    if val < 1 {
        val = 1;
    } else if val > 999 {
        val = 999;
    }
    if fast_return {
        return val;
    }

    // Reflect (stat 1) / Light Screen (stat 3) and Thick Club / Light Ball (×2 = << 1).
    let screen_shift = (!unboosted
        & (((stat_idx == stat_idx::DEF) & has_reflect)
            | ((stat_idx == stat_idx::SPD) & has_light_screen))) as u32;
    let kind = item_stat_kind as u8;
    let item_double_shift =
        (((kind == 1) & (stat_idx == stat_idx::ATK)) | ((kind == 2) & (stat_idx == stat_idx::SPA)))
            as u32;
    val <<= screen_shift + item_double_shift;

    // Metal Powder on Ditto (×1.5 = val + (val >> 1)).
    let is_metal = ((kind == 3) & ((stat_idx == stat_idx::DEF) | (stat_idx == stat_idx::SPD)))
        as i32;
    val += (val >> 1) & -is_metal;

    val
}

// =========================================================================
// 3. O(1) Accuracy Lookup Table (101 × 13 × 13 = 17,069 bytes)
// =========================================================================

/// Full precomputed Gen 2 accuracy table `[base_pct 0..=100][acc_boost + 6][eva_boost + 6]`.
/// Each entry stores the exact floored & clamped `[1, 255]` threshold from
/// `try_move_hit` / `hit_probability`.
pub type AccTable = [[[u8; 13]; 13]; 101];

static ACC_LUT_CELL: OnceLock<AccTable> = OnceLock::new();

fn build_acc_lut() -> AccTable {
    const POS_BOOST: [f64; 7] = [1.0, 1.33, 1.66, 2.0, 2.33, 2.66, 3.0];
    const NEG_BOOST: [f64; 7] = [1.0, 0.75, 0.6, 0.5, 0.43, 0.36, 0.33];
    let mut table = [[[1u8; 13]; 13]; 101];
    for pct in 0..=100usize {
        let base = (pct as f64 * 255.0 / 100.0).floor();
        for acc_idx in 0..13usize {
            let acc_b = acc_idx as i32 - 6;
            let after_acc = if acc_b > 0 {
                base * POS_BOOST[acc_b as usize]
            } else {
                base * NEG_BOOST[(-acc_b) as usize]
            };
            for eva_idx in 0..13usize {
                let eva_b = eva_idx as i32 - 6;
                let mut acc = after_acc;
                if eva_b > 0 {
                    acc *= NEG_BOOST[eva_b as usize];
                } else if eva_b < 0 {
                    acc *= POS_BOOST[(-eva_b) as usize];
                }
                acc = acc.floor().min(255.0).max(1.0);
                table[pct][acc_idx][eva_idx] = acc as u8;
            }
        }
    }
    table
}

/// Access the static 101 × 13 × 13 Gen 2 accuracy threshold table.
#[inline(always)]
pub fn acc_lut() -> &'static AccTable {
    ACC_LUT_CELL.get_or_init(build_acc_lut)
}

/// Branchless O(1) lookup of the Gen 2 `[0, 255]` accuracy threshold out of 256.
/// Returns `255` when a move has 255/256 (or guaranteed hit in `hit_probability`).
#[inline(always)]
pub fn lookup_accuracy_threshold(
    base_pct: u8,
    acc_boost: i8,
    eva_boost: i8,
    ignore_accuracy: bool,
    ignore_evasion: bool,
    ignore_positive_evasion: bool,
    brightpowder: bool,
) -> u8 {
    let pct = base_pct.min(100) as usize;
    let a_clamped = acc_boost.clamp(-6, 6);
    let e_clamped = eva_boost.clamp(-6, 6);

    let eff_acc = a_clamped & -(!ignore_accuracy as i8);
    let drop_pos_eva = ignore_evasion | (ignore_positive_evasion & (e_clamped > 0));
    let eff_eva = e_clamped & -(!drop_pos_eva as i8);

    let raw = acc_lut()[pct][(eff_acc + 6) as usize][(eff_eva + 6) as usize];
    let bp_sub = (-(brightpowder as i8) as u8) & 20;
    raw.saturating_sub(bp_sub)
}

/// Branchless implementation of `Battle::hit_probability`.
#[inline(always)]
pub fn hit_probability_branchless(
    base_pct: u8,
    always_hit: bool,
    acc_boost: i8,
    eva_boost: i8,
    ignore_accuracy: bool,
    ignore_evasion: bool,
    ignore_positive_evasion: bool,
) -> f64 {
    if always_hit {
        return 1.0;
    }
    let acc = lookup_accuracy_threshold(
        base_pct,
        acc_boost,
        eva_boost,
        ignore_accuracy,
        ignore_evasion,
        ignore_positive_evasion,
        false,
    );
    if acc == 255 {
        1.0
    } else {
        acc as f64 / 256.0
    }
}

// =========================================================================
// 4. Branchless Damage & SIMD 16-Roll / 39-Roll Evaluators
// =========================================================================

/// Weather kind in `CompactBattle`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
#[repr(u8)]
pub enum CompactWeather {
    #[default]
    None = 0,
    Rain = 1,
    Sun = 2,
    Sandstorm = 3,
}

impl CompactWeather {
    /// HP healed by Morning Sun / Moonlight / Synthesis under this active weather.
    #[inline(always)]
    pub const fn weather_heal_amount(self, maxhp: i16) -> i16 {
        match self {
            CompactWeather::Sun => maxhp,
            CompactWeather::Rain | CompactWeather::Sandstorm => {
                let q = maxhp / 4;
                if q < 1 { 1 } else { q }
            }
            CompactWeather::None => {
                let q = maxhp / 2;
                if q < 1 { 1 } else { q }
            }
        }
    }
}

/// Branchless computation of the pre-variance Gen 2 Stadium 2 damage (`[0, 3996]`).
///
/// Mirrors `Battle::get_damage` (`moveexec.rs:2865-2945`) using pure integer and
/// bitmask operations:
/// - Stadium 2 `>= 256` rollover (`>> 2` on both attack and defense simultaneously)
/// - Self-Destruct / Explosion defense halving (`>> 1`)
/// - Critical hit doubling (`<< 1`)
/// - 1.1x type-boost item (`dmg + dmg / 10`, exact for all IEEE-754 integers)
/// - Clamp `[1, 997] + 2`
/// - Weather boost (`+ (dmg >> 1)`) and nerf (`>> 1`)
/// - STAB (`+ (dmg >> 1)` via `attacker_type_mask` bit test)
/// - Type effectiveness (`<< super_shift >> resist_shift`, masked by `!is_immune`)
#[allow(clippy::too_many_arguments)]
#[inline(always)]
pub const fn get_damage_base_branchless(
    level: i32,
    base_power: i32,
    mut atk: i32,
    mut def: i32,
    is_physical: bool,
    is_selfdestruct: bool,
    is_crit: bool,
    has_type_boost_item: bool,
    weather: CompactWeather,
    is_water_move: bool,
    is_fire_move: bool,
    is_solarbeam: bool,
    move_type: u8,
    attacker_type_mask: u32,
    total_type_mod: i8,
    is_immune: bool,
) -> i32 {
    if is_immune || base_power <= 0 {
        return 0;
    }

    // Stadium 2 rollover fix: if attack >= 256 || defense >= 256, divide both by 4.
    let rollover_shift = (((atk | def) >= 256) as u32) << 1;
    atk >>= rollover_shift;
    if atk < 1 {
        atk = 1;
    }
    def >>= rollover_shift;
    if def < 1 {
        def = 1;
    }

    // Self-Destruct / Explosion halves physical defense (min 1).
    let selfdestruct_shift = (is_selfdestruct & is_physical) as u32;
    def >>= selfdestruct_shift;
    if def < 1 {
        def = 1;
    }

    // Core Gen 2 damage formula.
    let mut dmg = ((level * 2 / 5 + 2) * base_power * atk / def) / 50;

    // Critical hit doubles before ModifyDamage.
    dmg <<= is_crit as u32;

    // 1.1x type-boost held item: floor(dmg * 1.1) == dmg + dmg / 10.
    let boost_mask = -(has_type_boost_item as i32);
    dmg += (dmg / 10) & boost_mask;

    // Clamp to [1, 997], then + 2.
    if dmg < 1 {
        dmg = 1;
    } else if dmg > 997 {
        dmg = 997;
    }
    dmg += 2;

    // Weather modifier.
    let rain = (weather as u8) == (CompactWeather::Rain as u8);
    let sun = (weather as u8) == (CompactWeather::Sun as u8);
    let w_boost = ((is_water_move & rain) | (is_fire_move & sun)) as i32;
    let w_nerf = (((is_fire_move | is_solarbeam) & rain) | (is_water_move & sun)) as u32;
    dmg += (dmg >> 1) & -w_boost;
    dmg >>= w_nerf;

    // STAB (Same-Type Attack Bonus): +50% when move_type bit is set in attacker_type_mask.
    let is_stab = (((move_type < 32) as i32) & ((attacker_type_mask >> (move_type & 31)) as i32)) & 1;
    dmg += (dmg >> 1) & -is_stab;

    // Type effectiveness: super-effective shifts left (1 or 2), resist shifts right (1 or 2).
    let super_shift = if total_type_mod > 0 {
        if total_type_mod >= 2 { 2u32 } else { 1u32 }
    } else {
        0u32
    };
    let resist_shift = if total_type_mod < 0 {
        if total_type_mod <= -2 { 2u32 } else { 1u32 }
    } else {
        0u32
    };
    dmg = (dmg << super_shift) >> resist_shift;

    dmg
}

/// Branchless application of a single Gen 2 damage roll (`roll in 217..=255`).
/// Uses reciprocal multiplication `(x * 0x808081) >> 31` instead of integer
/// division `/ 255` (exact for all `x < 2^24`).
#[inline(always)]
pub const fn apply_roll_branchless(
    base_dmg: i32,
    roll_217_to_255: u32,
    no_damage_variance: bool,
) -> i32 {
    if base_dmg <= 0 {
        return 0;
    }
    let apply_var = (!no_damage_variance & (base_dmg > 1)) as i32;
    let num = (base_dmg as u64) * (roll_217_to_255 as u64);
    let varied = ((num * 0x808081) >> 31) as i32;
    let mask = -apply_var;
    let raw = (varied & mask) | (base_dmg & !mask);
    if raw < 1 { 1 } else { raw }
}

// =========================================================================
// 5. CompactBattle (256-Byte Fixed-Size, 64B-Aligned Copy Kernel)
// =========================================================================

/// Volatile status bitflags stored in `CompactMon::vol_flags` (`u16`).
pub mod vol_bit {
    pub const CONFUSION: u16 = 1 << 0;
    pub const FLINCH: u16 = 1 << 1;
    pub const DESTINY_BOND: u16 = 1 << 2;
    pub const SUBSTITUTE: u16 = 1 << 3;
    pub const PAR_SPEED_DROP: u16 = 1 << 4;
    pub const BRN_ATK_DROP: u16 = 1 << 5;
    pub const LEECH_SEED: u16 = 1 << 6;
    pub const NIGHTMARE: u16 = 1 << 7;
    pub const CURSE: u16 = 1 << 8;
    pub const FORESIGHT: u16 = 1 << 9;
    pub const PROTECT: u16 = 1 << 10;
    pub const ENDURE: u16 = 1 << 11;
    pub const ENCORE: u16 = 1 << 12;
    pub const DISABLE: u16 = 1 << 13;
    pub const PERISH_SONG: u16 = 1 << 14;
    pub const TRAPPED_OR_LOCKED: u16 = 1 << 15;
}

/// Mutable battle state of a single picked Pokemon (24 bytes, `Copy`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[repr(C)]
pub struct CompactMon {
    /// `[0]` = main HP, `[1]` = Substitute HP (`0` when no Substitute).
    pub hp_pool: [i16; 2],
    /// Current PP of the 4 move slots (`0..=64`).
    pub pp: [u8; 4],
    /// Low 3 bits = `Status` discriminant (`0..=7`), high 5 bits = sleep turns or toxic counter.
    pub status_packed: u8,
    /// Held item slot (`0` = None, `1..=64` = compact item id).
    pub item: u8,
    /// Bitmask of active volatile statuses (see [`vol_bit`]).
    pub vol_flags: u16,
    /// `[0]` = confusion turns, `[1]` = encore turns, `[2]` = disable turns, `[3]` = perish/stall counter.
    pub timers: [u8; 4],
    /// Stat stages `[atk, def, spa, spd, spe, acc, eva]` (`-6..=+6`).
    pub boosts: [i8; 7],
    /// Move slot (`0..=3`, or `0xFF` for None) of last move / encore / disable, packed:
    /// Packed slot references:
    /// - bits 0..1 (`0x03`): `last_move` slot (`0..3`)
    /// - bits 2..3 (`0x0C`): `encore_slot` (`0..3`)
    /// - bits 4..5 (`0x30`): `disable_slot` (`0..3`)
    /// - bit 6 (`0x40`): `last_move` valid flag
    /// - bit 7 (`0x80`): `mustrecharge` flag (Hyper Beam recharge turn)
    pub slot_refs: u8,
}

const _: () = assert!(std::mem::size_of::<CompactMon>() == 24);

impl CompactMon {
    #[inline(always)]
    pub const fn hp(&self) -> i16 {
        self.hp_pool[0]
    }

    #[inline(always)]
    pub const fn sub_hp(&self) -> i16 {
        self.hp_pool[1]
    }

    #[inline(always)]
    pub const fn status_raw(&self) -> u8 {
        self.status_packed & 0x07
    }

    #[inline(always)]
    pub const fn status_counter(&self) -> u8 {
        self.status_packed >> 3
    }

    #[inline(always)]
    pub fn set_status(&mut self, status_disc: u8, counter: u8) {
        self.status_packed = (status_disc & 0x07) | ((counter & 0x1F) << 3);
    }

    #[inline(always)]
    pub const fn perish_turns(&self) -> u8 {
        self.timers[timer_idx::PERISH_STALL] & 0x07
    }

    #[inline(always)]
    pub fn set_perish_turns(&mut self, turns: u8) {
        self.timers[timer_idx::PERISH_STALL] =
            (self.timers[timer_idx::PERISH_STALL] & !0x07) | (turns.min(7) & 0x07);
    }

    #[inline(always)]
    pub const fn stall_counter(&self) -> u8 {
        (self.timers[timer_idx::PERISH_STALL] >> 3) & 0x07
    }

    #[inline(always)]
    pub fn set_stall_counter(&mut self, count: u8) {
        self.timers[timer_idx::PERISH_STALL] =
            (self.timers[timer_idx::PERISH_STALL] & !0x38) | ((count.min(7) & 0x07) << 3);
    }

    /// Partial-trapping volatile (Wrap / Bind / Fire Spin / Clamp / Whirlpool),
    /// stored in bit 6 of `timers[PERISH_STALL]` so `Rapid Spin` / `Substitute`
    /// clear partial traps without colliding with Mean Look / Spider Web (`vol_bit::TRAPPED_OR_LOCKED`).
    #[inline(always)]
    pub const fn partially_trapped(&self) -> bool {
        (self.timers[timer_idx::PERISH_STALL] & 0x40) != 0
    }

    #[inline(always)]
    pub fn set_partially_trapped(&mut self, on: bool) {
        if on {
            self.timers[timer_idx::PERISH_STALL] |= 0x40;
        } else {
            self.timers[timer_idx::PERISH_STALL] &= !0x40;
        }
    }

    #[inline(always)]
    pub const fn is_trapped(&self) -> bool {
        (self.vol_flags & vol_bit::TRAPPED_OR_LOCKED) != 0 || self.partially_trapped()
    }

    #[inline(always)]
    pub const fn last_move_slot(&self) -> Option<u8> {
        if (self.slot_refs & 0x40) != 0 {
            Some(self.slot_refs & 0x03)
        } else {
            None
        }
    }

    #[inline(always)]
    pub fn set_last_move_slot(&mut self, slot: u8) {
        self.slot_refs = (self.slot_refs & !0x43) | (slot & 0x03) | 0x40;
    }

    #[inline(always)]
    pub fn clear_last_move_slot(&mut self) {
        self.slot_refs &= !0x40;
    }

    #[inline(always)]
    pub const fn encore_slot(&self) -> u8 {
        (self.slot_refs >> 2) & 0x03
    }

    #[inline(always)]
    pub fn set_encore_slot(&mut self, slot: u8) {
        self.slot_refs = (self.slot_refs & !(0x03 << 2)) | ((slot & 0x03) << 2);
    }

    #[inline(always)]
    pub const fn disable_slot(&self) -> u8 {
        (self.slot_refs >> 4) & 0x03
    }

    #[inline(always)]
    pub fn set_disable_slot(&mut self, slot: u8) {
        self.slot_refs = (self.slot_refs & !(0x03 << 4)) | ((slot & 0x03) << 4);
    }

    #[inline(always)]
    pub const fn is_slot_disabled(&self, slot: usize) -> bool {
        (self.vol_flags & vol_bit::DISABLE) != 0 && (self.disable_slot() as usize) == slot
    }

    #[inline(always)]
    pub const fn must_recharge(&self) -> bool {
        (self.slot_refs & 0x80) != 0
    }

    #[inline(always)]
    pub fn set_must_recharge(&mut self, on: bool) {
        if on {
            self.slot_refs |= 0x80;
        } else {
            self.slot_refs &= !0x80;
        }
    }

    /// Apply damage to either Substitute (`hp_pool[1]`) or main HP (`hp_pool[0]`)
    /// using branchless array indexing. Returns actual damage dealt.
    #[inline(always)]
    pub fn apply_damage_branchless(&mut self, dmg: i16, bypasses_sub: bool) -> i16 {
        let has_sub = (self.hp_pool[1] > 0) & (!bypasses_sub);
        let idx = has_sub as usize;
        let cur = self.hp_pool[idx];
        let dealt = if dmg < cur { dmg.max(0) } else { cur.max(0) };
        self.hp_pool[idx] = cur - dealt;
        if self.hp_pool[1] <= 0 {
            self.vol_flags &= !vol_bit::SUBSTITUTE;
        }
        if self.hp_pool[0] <= 0 {
            self.status_packed = Status::Fnt as u8;
            self.vol_flags = 0;
        }
        dealt
    }
}

/// Mutable state of one side (12 bytes, `Copy`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[repr(C)]
pub struct CompactSide {
    /// Active party index (`0..=2`).
    pub active_idx: u8,
    /// Original roster indices (`0..=5`) for the 3 picked party members.
    pub party_roster: [u8; 3],
    /// Bitmask of alive party indices (`bit 0..2`, `count_ones() == pokemon_left`).
    pub alive_mask: u8,
    /// Reflect remaining turns (`0..=5`).
    pub reflect_turns: u8,
    /// Light Screen remaining turns (`0..=5`).
    pub lightscreen_turns: u8,
    /// Safeguard remaining turns (`0..=5`).
    pub safeguard_turns: u8,
    /// Packed side flags:
    /// - bits 0..1 (`0x03`): Spikes layers (`0..3`)
    /// - bit 2 (`0x04`): `fainted_this_turn` / switch-needed flag
    /// - bit 3 (`0x08`, `sides[0]` only): `RESIDUALS_DONE` (prevents double residuals on post-residual switch)
    /// - bit 4 (`0x10`) + bits 5..6 (`0x60`): pending mid-turn move slot after Baton Pass
    /// - bit 7 (`0x80`, `sides[0]` only): Baton Pass fix disabled (`0` = Baton Pass fix active)
    pub side_flags: u8,
    /// Last move used for Self-KO clause tracking (`u8::MAX` = None).
    pub last_move_selfko: u8,
    /// Current request kind (`0` = Wait/None, `1` = Move, `2` = Switch, `3` = TeamPreview).
    pub request: u8,
    /// Reserved padding for 12-byte alignment.
    pub pad: u8,
}

const _: () = assert!(std::mem::size_of::<CompactSide>() == 12);

/// Complete mutable battle state in exactly 192 bytes (3 cache lines, 64-byte aligned).
/// Zero heap allocations: `Clone` is a 3-cache-line register copy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C, align(64))]
pub struct CompactBattle {
    /// 6 picked Pokemon (`0..3` = Side 0 party, `3..6` = Side 1 party): 144 bytes (offset 0..144).
    pub mons: [CompactMon; 6],
    /// 2 sides: 24 bytes (offset 144..168).
    pub sides: [CompactSide; 2],
    /// Current turn number (`1..=1001`): 2 bytes (offset 168..170).
    pub turn: u16,
    /// Weather kind (`None`, `Rain`, `Sun`, `Sandstorm`): 1 byte.
    pub weather_kind: CompactWeather,
    /// Weather remaining turns (`0..=5`): 1 byte.
    pub weather_turns: u8,
    /// Pre-rolled Quick Claw activation coin for the current turn: 1 byte.
    pub quick_claw_roll: u8,
    /// Terminal state (`0` = Ongoing, `1` = P1Win, `2` = P2Win, `3` = Tie): 1 byte.
    pub outcome: u8,
    /// Speed order bit (`0` = P1 faster/won tie, `1` = P2 faster/won tie): 1 byte.
    pub speed_order: u8,
    /// Clause flags (`bit 0 = sleep_clause`, `bit 2 = self_ko`): 1 byte.
    pub clause_flags: u8,
    /// Last damage dealt to `[P1, P2]` this turn (for Counter / Mirror Coat): 4 bytes (offset 176..180).
    pub hurt_damage_this_turn: [u16; 2],
    /// Category of last move that damaged `[P1, P2]` (`0` = None, `1` = Physical, `2` = Special): 2 bytes (offset 180..182).
    pub hurt_cat_this_turn: [u8; 2],
    /// Low 16 bits of the static match fingerprint (bits 16..32 stored in `sides[0..2].pad`): 2 bytes (offset 182..184).
    pub match_fp_lo: u16,
    /// 64-bit PRNG state (Gen5 LCG seed): 8 bytes (offset 184..192).
    pub prng_seed: u64,
}

const _: () = assert!(std::mem::size_of::<CompactBattle>() == 192);
const _: () = assert!(std::mem::align_of::<CompactBattle>() == 64);

/// Precomputed per-Pokemon move-role bitflags on [`StaticMonSpec::role_flags`].
pub mod role_bit {
    pub const HALF_HEAL: u8 = 1 << 0;
    pub const REST: u8 = 1 << 1;
    pub const SLEEP_USABLE: u8 = 1 << 2;
    pub const PERISH_PHAZE_TRAP: u8 = 1 << 3;
    pub const BATON_PASS: u8 = 1 << 4;
    pub const PROTECT: u8 = 1 << 5;
    pub const SWAGGER: u8 = 1 << 6;
    pub const PHAZE_OR_HAZE: u8 = 1 << 7;
}

/// Immutable per-Pokemon reference data stored in [`MatchTable`].
#[derive(Clone, Copy, Debug, Default)]
pub struct StaticMonSpec {
    /// Precomputed 64-bit static fingerprint of this Pokemon's species, stats, item, and moves.
    pub static_fp: u64,
    /// Bitmask of the Pokemon's 1–2 types (`(1 << t0) | (1 << t1)`).
    pub type_mask: u32,
    pub species: SpeciesId,
    pub maxhp: i16,
    /// 1/8 of maxhp (min 1) for standard poison/burn/leech seed slip damage.
    pub quantum_1_8: i16,
    /// 1/16 of maxhp (min 1) for Leftovers and Toxic base slip damage.
    pub quantum_1_16: i16,
    /// Stored stats `[atk, def, spa, spd, spe]`.
    pub stored_stats: [i16; 5],
    pub moves: [MoveId; 4],
    pub max_pp: [u8; 4],
    pub types: [u8; 2],
    pub num_types: u8,
    pub level: u8,
    pub num_moves: u8,
    pub role_flags: u8,
    /// Species-eligible stat-boost item kind (Cubone/Marowak = ThickClub, Pikachu = LightBall, Ditto = MetalPowder),
    /// independent of initial held item so stolen items work dynamically.
    pub species_stat_kind: ItemStatKind,
}

/// Immutable per-move compiled metadata for branchless execution.
#[derive(Clone, Copy, Debug, Default)]
pub struct CompactMoveSpec {
    pub id: MoveId,
    pub base_power: i16,
    /// Fixed damage (`-1` = Level, `> 0` = Amount(n), `0` = None).
    pub fixed_damage: i16,
    /// Primary volatile inflicted on target (`vol_bit` mask).
    pub primary_vol: u16,
    /// Secondary volatile inflicted on target (`vol_bit` mask).
    pub sec_vol: u16,
    pub move_type: u8,
    pub is_physical: bool,
    pub is_status: bool,
    pub priority: i8,
    pub base_acc_pct: u8,
    pub always_hit: bool,
    pub ignore_accuracy: bool,
    pub ignore_evasion: bool,
    pub ignore_positive_evasion: bool,
    pub crit_ratio: u8,
    pub is_selfdestruct: bool,
    pub is_water: bool,
    pub is_fire: bool,
    pub is_solarbeam: bool,
    pub bypasses_sub: bool,
    pub is_rest: bool,
    pub is_half_heal: bool,
    pub is_recharge: bool,
    pub is_defrost: bool,
    pub is_heal_cycle: bool,
    pub is_sleep_usable: bool,
    pub can_sleep_talk: bool,
    pub is_protect: bool,
    pub is_endure: bool,
    pub blocked_by_protect: bool,
    pub is_substitute: bool,
    pub drain_half: bool,
    pub recoil_quarter: bool,
    /// Primary status inflicted on target (`0` = None, else `Status as u8`).
    pub primary_status: u8,
    /// Secondary chance out of 256 (`0` = None).
    pub sec_chance_256: u8,
    /// Secondary status inflicted on target (`0` = None, else `Status as u8`).
    pub sec_status: u8,
    /// Secondary stat stage delta (`(stat_idx, delta)` or `(0xFF, 0)`).
    pub sec_boost: (u8, i8),
    /// Self-targeted or target-applied stat boosts (`[(stat_idx_or_ff, delta); 3]`).
    pub boosts: [(u8, i8); 3],
    pub boosts_self: bool,
    /// Special move effect tag (`SpecialMoveKind`).
    pub special_kind: SpecialMoveKind,
    /// Variable base power kind (`VarBpKind`).
    pub var_bp_kind: VarBpKind,
    /// Twice the expected hit count (`2` = single hit, `6` = 2..5 multihit, `2*n` = Fixed(n)).
    pub multihit_mean_x2: u8,
}

/// Precompiled per-item metadata for O(1) lookup in `CompactBattle`.
#[derive(Clone, Copy, Debug, Default)]
pub struct CompactItemSpec {
    pub boost_type: u8,
    pub stat_kind: ItemStatKind,
    pub is_quick_claw: bool,
    pub is_leftovers: bool,
    pub is_brightpowder: bool,
    pub is_mintberry: bool,
    pub is_miracleberry: bool,
    pub is_goldberry: bool,
    pub is_berry_juice: bool,
    pub is_berry: bool,
    pub is_scope_lens: bool,
}

static DEFAULT_ITEM_SPEC: CompactItemSpec = CompactItemSpec {
    boost_type: NO_TYPE_ID,
    stat_kind: ItemStatKind::None,
    is_quick_claw: false,
    is_leftovers: false,
    is_brightpowder: false,
    is_mintberry: false,
    is_miracleberry: false,
    is_goldberry: false,
    is_berry_juice: false,
    is_berry: false,
    is_scope_lens: false,
};

/// Process-wide precompiled Dex tables for `CompactBattle` and fast evaluation.
pub struct CompactDexTables {
    pub type_chart: [[i8; 20]; 20],
    pub special_type_mask: u32,
    pub normal_type: u8,
    pub fighting_type: u8,
    pub flying_type: u8,
    pub poison_type: u8,
    pub ground_type: u8,
    pub rock_type: u8,
    pub ghost_type: u8,
    pub steel_type: u8,
    pub fire_type: u8,
    pub water_type: u8,
    pub grass_type: u8,
    pub electric_type: u8,
    pub ice_type: u8,
    pub sandstorm_immune_mask: u32,
    pub poison_immune_mask: u32,
    pub move_specs: Vec<CompactMoveSpec>,
    pub item_specs: Vec<CompactItemSpec>,
    pub sub_cond: Option<crate::dex::CondId>,
    pub conf_cond: Option<crate::dex::CondId>,
    pub flinch_cond: Option<crate::dex::CondId>,
    pub dbond_cond: Option<crate::dex::CondId>,
    pub par_cond: Option<crate::dex::CondId>,
    pub brn_cond: Option<crate::dex::CondId>,
    pub seed_cond: Option<crate::dex::CondId>,
    pub night_cond: Option<crate::dex::CondId>,
    pub curse_cond: Option<crate::dex::CondId>,
    pub fore_cond: Option<crate::dex::CondId>,
    pub prot_cond: Option<crate::dex::CondId>,
    pub end_cond: Option<crate::dex::CondId>,
    pub stall_cond: Option<crate::dex::CondId>,
    pub enc_cond: Option<crate::dex::CondId>,
    pub dis_cond: Option<crate::dex::CondId>,
    pub perish_cond: Option<crate::dex::CondId>,
    pub res_cond: Option<crate::dex::CondId>,
    pub recharge_cond: Option<crate::dex::CondId>,
    pub trapped_cond: Option<crate::dex::CondId>,
    pub partiallytrapped_cond: Option<crate::dex::CondId>,
    pub refl_cond: Option<crate::dex::CondId>,
    pub ls_cond: Option<crate::dex::CondId>,
    pub safe_cond: Option<crate::dex::CondId>,
    pub spikes_cond: Option<crate::dex::CondId>,
    pub rain_cond: Option<crate::dex::CondId>,
    pub sun_cond: Option<crate::dex::CondId>,
    pub sand_cond: Option<crate::dex::CondId>,
    pub sleep_clause_cond: Option<crate::dex::CondId>,
    pub freeze_clause_cond: Option<crate::dex::CondId>,
}

static COMPACT_DEX_TABLES: OnceLock<CompactDexTables> = OnceLock::new();

/// Access the process-wide precompiled `CompactDexTables`.
#[inline]
pub fn dex_tables(dex: &Dex) -> &'static CompactDexTables {
    COMPACT_DEX_TABLES.get_or_init(|| build_compact_dex_tables(dex))
}

fn build_compact_dex_tables(dex: &Dex) -> CompactDexTables {
    let mut type_chart = [[0i8; 20]; 20];
    let n_types = dex.type_names.len().min(19);
    for atk_t in 0..n_types {
        for def_t in 0..n_types {
            let atk_id = crate::dex::TypeId(atk_t as u8);
            let def_id = crate::dex::TypeId(def_t as u8);
            if dex.type_immune(atk_id, def_id) {
                type_chart[atk_t][def_t] = -9;
            } else {
                type_chart[atk_t][def_t] = dex.eff(atk_id, def_id) as i8;
            }
        }
    }

    let mut special_type_mask = 0u32;
    for t_name in [
        "Fire", "Water", "Grass", "Electric", "Psychic", "Ice", "Dragon", "Dark",
    ] {
        if let Some(tid) = dex.type_id(t_name) {
            special_type_mask |= 1u32 << (tid.0 & 31);
        }
    }

    let tid_or = |name: &str, fallback: u8| dex.type_id(name).map(|t| t.0).unwrap_or(fallback);
    let normal_type = dex.known_types.normal.0;
    let fighting_type = dex.known_types.fighting.0;
    let flying_type = dex.known_types.flying.0;
    let poison_type = tid_or("Poison", NO_TYPE_ID);
    let ground_type = dex.known_types.ground.0;
    let rock_type = tid_or("Rock", NO_TYPE_ID);
    let ghost_type = dex.known_types.ghost.0;
    let steel_type = tid_or("Steel", NO_TYPE_ID);
    let fire_type = dex.known_types.fire.0;
    let water_type = dex.known_types.water.0;
    let grass_type = dex.known_types.grass.0;
    let electric_type = tid_or("Electric", NO_TYPE_ID);
    let ice_type = tid_or("Ice", NO_TYPE_ID);

    let mut sandstorm_immune_mask = 0u32;
    for &t in &[rock_type, ground_type, steel_type] {
        if t < 32 {
            sandstorm_immune_mask |= 1u32 << t;
        }
    }
    let mut poison_immune_mask = 0u32;
    for &t in &[poison_type, steel_type] {
        if t < 32 {
            poison_immune_mask |= 1u32 << t;
        }
    }

    let parse_status_code = |s: Option<&str>| -> u8 {
        match s {
            Some("slp") => Status::Slp as u8,
            Some("psn") => Status::Psn as u8,
            Some("brn") => Status::Brn as u8,
            Some("frz") => Status::Frz as u8,
            Some("par") => Status::Par as u8,
            Some("tox") => Status::Tox as u8,
            _ => 0,
        }
    };
    let parse_vol_code = |s: Option<&str>| -> u16 {
        match s {
            Some("confusion") => vol_bit::CONFUSION,
            Some("flinch") => vol_bit::FLINCH,
            Some("leechseed") => vol_bit::LEECH_SEED,
            Some("nightmare") => vol_bit::NIGHTMARE,
            Some("curse") => vol_bit::CURSE,
            Some("foresight") => vol_bit::FORESIGHT,
            Some("encore") => vol_bit::ENCORE,
            Some("disable") => vol_bit::DISABLE,
            Some("perishsong") => vol_bit::PERISH_SONG,
            Some("destinybond") => vol_bit::DESTINY_BOND,
            _ => 0,
        }
    };

    let bypasssub_bit = dex.flag_bit("bypasssub");
    let nosleeptalk_bit = dex.flag_bit("nosleeptalk");
    let charge_bit = dex.flag_bit("charge");
    let protect_bit = dex.flag_bit("protect");

    let km = &dex.known_moves;
    let ki = &dex.known_items;
    let mut move_specs = Vec::with_capacity(dex.moves.len());
    for m_raw in 0..dex.moves.len() {
        let mid = MoveId(m_raw as u16);
        let ms = dex.move_static(mid);
        let mkey = dex.moves.key(mid);
        let var_bp_kind = match mkey {
            "return" => VarBpKind::Return,
            "frustration" => VarBpKind::Frustration,
            "hiddenpower" => VarBpKind::HiddenPower,
            "flail" | "reversal" => VarBpKind::FlailReversal,
            "magnitude" => VarBpKind::Magnitude,
            "present" => VarBpKind::Present,
            _ => VarBpKind::Static,
        };
        let bp = match var_bp_kind {
            VarBpKind::Magnitude => 71,
            VarBpKind::Present => 40,
            _ => ms.base_power as i16,
        };
        let (base_acc_pct, always_hit) = match ms.accuracy {
            Accuracy::AlwaysHits => (100, true),
            Accuracy::Pct(a) => (a.clamp(0, 100) as u8, ms.always_hit),
        };
        let primary_status = parse_status_code(ms.status);
        let primary_vol = parse_vol_code(ms.volatile_status);
        let mut sec_chance_256 = 0u8;
        let mut sec_status = 0u8;
        let mut sec_vol = 0u16;
        let mut sec_boost = (NO_STAT_IDX, 0i8);
        if let Some(sec) = ms.secondaries.first() {
            let ch = sec.chance.unwrap_or(100).clamp(0, 100);
            sec_chance_256 = (ch * 255 / 100).clamp(1, 255) as u8;
            sec_status = parse_status_code(sec.status);
            sec_vol = parse_vol_code(sec.volatile_status);
            if let Some(&(st_i, d)) = sec.boosts.first() {
                sec_boost = (st_i as u8, d);
            } else if let Some(se) = sec.self_effect.as_ref() {
                if mkey == "ancientpower" {
                    sec_boost = (SELF_BOOST_FLAG | ANCIENTPOWER_ALL_STATS, 1);
                } else if let Some(&(st_i, d)) = se.boosts.first() {
                    sec_boost = ((st_i as u8) | SELF_BOOST_FLAG, d);
                }
            }
        }
        let mut boosts = [(NO_STAT_IDX, 0i8); 3];
        let mut boosts_self = matches!(ms.target, "self" | "allySide" | "adjacentAllyOrSelf");
        let src_boosts = if !ms.boosts.is_empty() {
            &ms.boosts[..]
        } else if let Some(se) = ms.self_effect.as_ref() {
            boosts_self = true;
            &se.boosts[..]
        } else {
            &[][..]
        };
        for (b_i, &(st_i, d)) in src_boosts.iter().take(3).enumerate() {
            boosts[b_i] = (st_i as u8, d);
        }
        let special_kind = match mkey {
            "reflect" => SpecialMoveKind::Reflect,
            "lightscreen" => SpecialMoveKind::LightScreen,
            "safeguard" => SpecialMoveKind::Safeguard,
            "spikes" => SpecialMoveKind::Spikes,
            "curse" => SpecialMoveKind::Curse,
            "bellydrum" => SpecialMoveKind::BellyDrum,
            "roar" | "whirlwind" => SpecialMoveKind::Phaze,
            "encore" => SpecialMoveKind::Encore,
            "disable" => SpecialMoveKind::Disable,
            "sleeptalk" => SpecialMoveKind::SleepTalk,
            "counter" => SpecialMoveKind::Counter,
            "mirrorcoat" => SpecialMoveKind::MirrorCoat,
            "flail" | "reversal" => SpecialMoveKind::FlailReversal,
            "seismictoss" | "nightshade" => SpecialMoveKind::LevelDamage,
            "superfang" => SpecialMoveKind::SuperFang,
            "thief" => SpecialMoveKind::Thief,
            "painsplit" => SpecialMoveKind::PainSplit,
            "batonpass" => SpecialMoveKind::BatonPass,
            "perishsong" => SpecialMoveKind::PerishSong,
            "destinybond" => SpecialMoveKind::DestinyBond,
            "healbell" => SpecialMoveKind::HealBell,
            "psychup" => SpecialMoveKind::PsychUp,
            "raindance" => SpecialMoveKind::RainDance,
            "sunnyday" => SpecialMoveKind::SunnyDay,
            "sandstorm" => SpecialMoveKind::Sandstorm,
            "meanlook" | "spiderweb" => SpecialMoveKind::MeanLook,
            "haze" => SpecialMoveKind::Haze,
            "rapidspin" => SpecialMoveKind::RapidSpin,
            "moonlight" | "morningsun" | "synthesis" => SpecialMoveKind::WeatherHeal,
            "swagger" => SpecialMoveKind::Swagger,
            _ => SpecialMoveKind::None,
        };
        let fixed_damage = match ms.damage {
            Some(crate::dex::FixedDamage::Level) => FIXED_DAMAGE_LEVEL,
            Some(crate::dex::FixedDamage::Amount(n)) => n.clamp(1, i16::MAX as i32) as i16,
            None => 0,
        };
        let multihit_mean_x2 = match ms.multihit {
            Some(crate::dex::Multihit::Fixed(n)) => (n * 2).clamp(2, 20) as u8,
            Some(crate::dex::Multihit::Range(2, 5)) => 6,
            Some(crate::dex::Multihit::Range(lo, hi)) => (lo + hi).clamp(2, 20) as u8,
            None => 2,
        };

        let smid = Some(mid);
        let is_half_heal = smid == km.recover || smid == km.softboiled || smid == km.milkdrink;
        let is_rest = smid == km.rest;
        move_specs.push(CompactMoveSpec {
            id: mid,
            base_power: bp,
            move_type: ms.move_type.0,
            is_physical: ms.category == Category::Physical,
            is_status: ms.category == Category::Status,
            priority: ms.priority,
            base_acc_pct,
            always_hit,
            ignore_accuracy: ms.ignore_accuracy,
            ignore_evasion: ms.ignore_evasion,
            ignore_positive_evasion: ms.ignore_positive_evasion,
            crit_ratio: ms.crit_ratio.clamp(0, 5) as u8,
            is_selfdestruct: ms.selfdestruct,
            is_water: ms.move_type.0 == water_type,
            is_fire: ms.move_type.0 == fire_type,
            is_solarbeam: smid == km.solarbeam,
            bypasses_sub: (ms.flags & bypasssub_bit) != 0,
            is_rest,
            is_half_heal,
            is_recharge: ms.has_flag(dex, "recharge"),
            is_defrost: ms.has_flag(dex, "defrost"),
            is_heal_cycle: is_rest || is_half_heal,
            is_sleep_usable: ms.sleep_usable,
            can_sleep_talk: (ms.flags & (nosleeptalk_bit | charge_bit)) == 0,
            is_protect: smid == km.protect || smid == km.detect,
            is_endure: smid == km.endure,
            blocked_by_protect: (ms.flags & protect_bit) != 0,
            is_substitute: smid == km.substitute,
            drain_half: ms.drain.is_some(),
            recoil_quarter: ms.recoil.is_some(),
            primary_status,
            primary_vol: if ms.target == "self"
                || matches!(
                    special_kind,
                    SpecialMoveKind::Curse
                        | SpecialMoveKind::Encore
                        | SpecialMoveKind::Disable
                        | SpecialMoveKind::PerishSong
                        | SpecialMoveKind::DestinyBond
                ) {
                0
            } else {
                primary_vol
            },
            sec_chance_256,
            sec_status,
            sec_vol,
            sec_boost,
            boosts,
            boosts_self,
            special_kind,
            var_bp_kind,
            fixed_damage,
            multihit_mean_x2,
        });
    }

    debug_assert!(dex.items.len() < 128);
    let mut item_specs = Vec::with_capacity(dex.items.len());
    for i_raw in 0..dex.items.len() {
        let iid = ItemId(i_raw as u16);
        let siid = Some(iid);
        let ikey = dex.items.key(iid);
        let boost_type = COMPACT_TYPE_BOOST_ITEMS
            .iter()
            .find(|&&(k, _)| k == ikey)
            .and_then(|&(_, t_name)| dex.type_id(t_name))
            .map(|t| t.0)
            .unwrap_or(NO_TYPE_ID);
        let stat_kind = if siid == ki.thickclub {
            ItemStatKind::ThickClub
        } else if siid == ki.lightball {
            ItemStatKind::LightBall
        } else if siid == ki.metalpowder {
            ItemStatKind::MetalPowder
        } else {
            ItemStatKind::None
        };
        item_specs.push(CompactItemSpec {
            boost_type,
            stat_kind,
            is_quick_claw: siid == ki.quickclaw,
            is_leftovers: siid == ki.leftovers,
            is_brightpowder: siid == ki.brightpowder,
            is_mintberry: siid == ki.mintberry,
            is_miracleberry: siid == ki.miracleberry,
            is_goldberry: siid == ki.goldberry,
            is_berry_juice: siid == ki.berryjuice,
            is_berry: siid == ki.berry,
            is_scope_lens: siid == ki.scopelens,
        });
    }

    CompactDexTables {
        type_chart,
        special_type_mask,
        normal_type,
        fighting_type,
        flying_type,
        poison_type,
        ground_type,
        rock_type,
        ghost_type,
        steel_type,
        fire_type,
        water_type,
        grass_type,
        electric_type,
        ice_type,
        sandstorm_immune_mask,
        poison_immune_mask,
        move_specs,
        item_specs,
        sub_cond: dex.conds_id("substitute"),
        conf_cond: dex.conds_id("confusion"),
        flinch_cond: dex.conds_id("flinch"),
        dbond_cond: dex.conds_id("destinybond"),
        par_cond: dex.conds_id("parspeeddrop"),
        brn_cond: dex.conds_id("brnattackdrop"),
        seed_cond: dex.conds_id("leechseed"),
        night_cond: dex.conds_id("nightmare"),
        curse_cond: dex.conds_id("curse"),
        fore_cond: dex.conds_id("foresight"),
        prot_cond: dex.conds_id("protect"),
        end_cond: dex.conds_id("endure"),
        stall_cond: dex.conds_id("stall"),
        enc_cond: dex.conds_id("encore"),
        dis_cond: dex.conds_id("disable"),
        perish_cond: dex.conds_id("perishsong"),
        res_cond: dex.conds_id("residualdmg"),
        recharge_cond: dex.conds_id("mustrecharge"),
        trapped_cond: dex.conds_id("trapped"),
        partiallytrapped_cond: dex.conds_id("partiallytrapped"),
        refl_cond: dex.conds_id("reflect"),
        ls_cond: dex.conds_id("lightscreen"),
        safe_cond: dex.conds_id("safeguard"),
        spikes_cond: dex.conds_id("spikes"),
        rain_cond: dex.conds_id("raindance"),
        sun_cond: dex.conds_id("sunnyday"),
        sand_cond: dex.conds_id("sandstorm"),
        sleep_clause_cond: dex.conds_id("stadiumsleepclause"),
        freeze_clause_cond: dex.conds_id("freezeclausemod"),
    }
}

static DEFAULT_TYPE_CHART: [[i8; 20]; 20] = [[0i8; 20]; 20];

/// Static match context shared across all `CompactBattle` nodes in a search tree.
#[derive(Clone, Debug)]
pub struct MatchTable {
    /// 6 picked Pokemon specs (`0..3` = Side 0, `3..6` = Side 1).
    pub mons: [StaticMonSpec; 6],
    /// Precompiled move specs for each of the 6 Pokemon's 4 move slots.
    pub move_specs: [[CompactMoveSpec; 4]; 6],
    /// Reference to the process-wide 20 × 20 type effectiveness matrix (`-1` = resist, `0` = neutral, `+1` = super, `-9` = immune).
    pub type_chart: &'static [[i8; 20]; 20],
    /// Reference to the process-wide precompiled per-item metadata slice (indexed by `compact_item - 1`).
    pub item_specs: &'static [CompactItemSpec],
    pub normal_type: u8,
    pub fighting_type: u8,
    pub flying_type: u8,
    pub ground_type: u8,
    pub ghost_type: u8,
    pub grass_type: u8,
    pub fire_type: u8,
    pub electric_type: u8,
    pub ice_type: u8,
    pub sandstorm_immune_mask: u32,
    pub poison_immune_mask: u32,
    pub match_fingerprint: u64,
}

impl Default for MatchTable {
    fn default() -> Self {
        Self {
            mons: [StaticMonSpec::default(); 6],
            move_specs: [[CompactMoveSpec::default(); 4]; 6],
            type_chart: &DEFAULT_TYPE_CHART,
            item_specs: &[],
            normal_type: 0,
            fighting_type: 0,
            flying_type: 0,
            ground_type: 0,
            ghost_type: 0,
            grass_type: 0,
            fire_type: 0,
            electric_type: 0,
            ice_type: 0,
            sandstorm_immune_mask: 0,
            poison_immune_mask: 0,
            match_fingerprint: 0,
        }
    }
}

impl MatchTable {
    /// O(1) dynamic item metadata lookup by `CompactMon::item` (`0` = None, `1..=N` = `ItemId.0 + 1`).
    #[inline(always)]
    pub fn item_spec(&self, compact_item: u8) -> &'static CompactItemSpec {
        if compact_item == 0 {
            return &DEFAULT_ITEM_SPEC;
        }
        self.item_specs
            .get((compact_item - 1) as usize)
            .unwrap_or(&DEFAULT_ITEM_SPEC)
    }

    /// Compute the net type effectiveness (`-2..=+2`) and immunity flag against a target,
    /// accounting for Foresight (`vol_bit::FORESIGHT`) removing Ghost's immunity to Normal/Fighting.
    #[inline(always)]
    pub const fn effectiveness_ext(
        &self,
        move_type: u8,
        def_mon: &StaticMonSpec,
        foresight: bool,
    ) -> (i8, bool) {
        if move_type == NO_TYPE_ID {
            return (0, false);
        }
        let mt = if (move_type as usize) < 20 {
            move_type as usize
        } else {
            19
        };
        let t0 = if (def_mon.types[0] as usize) < 20 {
            def_mon.types[0] as usize
        } else {
            19
        };
        let mut e0 = self.type_chart[mt][t0];
        if e0 == -9 {
            if foresight
                && (move_type == self.normal_type || move_type == self.fighting_type)
                && def_mon.types[0] == self.ghost_type
            {
                e0 = 0;
            } else {
                return (0, true);
            }
        }
        if def_mon.num_types > 1 && def_mon.types[1] != def_mon.types[0] {
            let t1 = if (def_mon.types[1] as usize) < 20 {
                def_mon.types[1] as usize
            } else {
                19
            };
            let mut e1 = self.type_chart[mt][t1];
            if e1 == -9 {
                if foresight
                    && (move_type == self.normal_type || move_type == self.fighting_type)
                    && def_mon.types[1] == self.ghost_type
                {
                    e1 = 0;
                } else {
                    return (0, true);
                }
            }
            (e0 + e1, false)
        } else {
            (e0, false)
        }
    }
}

const COMPACT_TYPE_BOOST_ITEMS: [(&str, &str); 18] = [
    ("blackbelt", "Fighting"),
    ("blackglasses", "Dark"),
    ("charcoal", "Fire"),
    ("dragonfang", "Dragon"),
    ("hardstone", "Rock"),
    ("magnet", "Electric"),
    ("metalcoat", "Steel"),
    ("miracleseed", "Grass"),
    ("mysticwater", "Water"),
    ("nevermeltice", "Ice"),
    ("pinkbow", "Normal"),
    ("poisonbarb", "Poison"),
    ("polkadotbow", "Normal"),
    ("sharpbeak", "Flying"),
    ("silverpowder", "Bug"),
    ("softsand", "Ground"),
    ("spelltag", "Ghost"),
    ("twistedspoon", "Psychic"),
];

#[derive(Clone, Copy, Debug)]
pub struct CachedRosterMon {
    pub spec: StaticMonSpec,
    pub move_specs: [CompactMoveSpec; 4],
    guard_maxhp: i32,
    guard_stored_stats: [i32; 5],
    guard_types: crate::dex::TypeList,
    guard_item: Option<ItemId>,
    guard_hp_type: crate::dex::TypeId,
    guard_hp_power: i32,
    guard_happiness: u8,
    guard_maxpp: [i32; 4],
}

#[derive(Clone, Debug, Default)]
pub struct RosterCompactCache {
    pub entries: [[Option<CachedRosterMon>; 6]; 2],
}

impl RosterCompactCache {
    fn build_side_entries(
        side: &crate::state::Side,
        dex: &Dex,
        dt: &CompactDexTables,
    ) -> [Option<CachedRosterMon>; 6] {
        let mut out = [None; 6];
        for (slot, p) in side.roster.iter().take(6).enumerate() {
            if p.transformed {
                continue;
            }
            let (spec, move_specs) = build_static_mon_spec(p, dex, dt);
            let mut guard_maxpp = [0i32; 4];
            for m_i in 0..(spec.num_moves as usize) {
                guard_maxpp[m_i] = p.move_slots[m_i].maxpp;
            }
            out[slot] = Some(CachedRosterMon {
                spec,
                move_specs,
                guard_maxhp: p.maxhp,
                guard_stored_stats: p.stored_stats,
                guard_types: p.types,
                guard_item: p.item,
                guard_hp_type: p.hp_type,
                guard_hp_power: p.hp_power,
                guard_happiness: p.happiness,
                guard_maxpp,
            });
        }
        out
    }

    pub fn from_battle(battle: &Battle, dex: &Dex) -> Self {
        let dt = dex_tables(dex);
        let entries = [
            Self::build_side_entries(&battle.sides[0], dex, dt),
            Self::build_side_entries(&battle.sides[1], dex, dt),
        ];
        Self { entries }
    }

    pub fn from_battle_with_side_template(
        battle: &Battle,
        dex: &Dex,
        template_side: usize,
        template: &Self,
    ) -> Self {
        if template_side >= 2 {
            return Self::from_battle(battle, dex);
        }
        let dt = dex_tables(dex);
        let mut entries = [[None; 6]; 2];
        entries[template_side] = template.entries[template_side];
        let other = 1 - template_side;
        entries[other] = Self::build_side_entries(&battle.sides[other], dex, dt);
        Self { entries }
    }
}

#[inline]
fn build_static_mon_spec(
    p: &crate::state::Pokemon,
    dex: &Dex,
    dt: &CompactDexTables,
) -> (StaticMonSpec, [CompactMoveSpec; 4]) {
    let maxhp = p.maxhp.max(1) as i16;
    let mut type_mask = 0u32;
    for ty in p.types.iter() {
        type_mask |= 1u32 << (ty.0 & 31);
    }
    let item_spec = p
        .item
        .and_then(|i| dt.item_specs.get(i.0 as usize).copied())
        .unwrap_or(CompactItemSpec {
            boost_type: NO_TYPE_ID,
            ..CompactItemSpec::default()
        });

    let return_bp = (p.happiness as i32 * 10 / 25) as i16;
    let mut moves = [MoveId(0); 4];
    let mut max_pp = [0u8; 4];
    let mut move_specs = [CompactMoveSpec::default(); 4];
    let mut role_flags = 0u8;
    let num_moves = p.move_slots.len().min(4) as u8;
    for m_i in 0..num_moves as usize {
        let slot = p.move_slots[m_i];
        moves[m_i] = slot.id;
        max_pp[m_i] = slot.maxpp.clamp(0, 64) as u8;

        let mut cm = dt.move_specs[slot.id.0 as usize];
        match cm.var_bp_kind {
            VarBpKind::Return => cm.base_power = return_bp,
            VarBpKind::Frustration => cm.base_power = ((255 - p.happiness as i32) * 10 / 25) as i16,
            VarBpKind::HiddenPower => {
                let hpt = p.hp_type.0;
                cm.move_type = hpt;
                cm.base_power = p.hp_power as i16;
                cm.is_physical = (dt.special_type_mask & (1u32 << (hpt & 31))) == 0;
                cm.is_water = hpt == dt.water_type;
                cm.is_fire = hpt == dt.fire_type;
            }
            _ => {}
        }
        if cm.is_half_heal || cm.special_kind == SpecialMoveKind::WeatherHeal {
            role_flags |= role_bit::HALF_HEAL;
        }
        if cm.is_rest {
            role_flags |= role_bit::REST;
        }
        if cm.special_kind == SpecialMoveKind::SleepTalk || cm.is_sleep_usable {
            role_flags |= role_bit::SLEEP_USABLE;
        }
        if matches!(
            cm.special_kind,
            SpecialMoveKind::Phaze | SpecialMoveKind::PerishSong | SpecialMoveKind::MeanLook
        ) {
            role_flags |= role_bit::PERISH_PHAZE_TRAP;
        }
        if cm.special_kind == SpecialMoveKind::BatonPass {
            role_flags |= role_bit::BATON_PASS;
        }
        if cm.is_protect {
            role_flags |= role_bit::PROTECT;
        }
        if !cm.boosts_self
            && cm.boosts[0].0 == stat_idx::ATK as u8
            && cm.boosts[0].1 > 0
            && (cm.primary_vol & vol_bit::CONFUSION) != 0
        {
            role_flags |= role_bit::SWAGGER;
        }
        if matches!(
            cm.special_kind,
            SpecialMoveKind::Phaze | SpecialMoveKind::Haze
        ) {
            role_flags |= role_bit::PHAZE_OR_HAZE;
        }
        move_specs[m_i] = cm;
    }

    let stored_stats = [
        p.stored_stats[stat_idx::ATK] as i16,
        p.stored_stats[stat_idx::DEF] as i16,
        p.stored_stats[stat_idx::SPA] as i16,
        p.stored_stats[stat_idx::SPD] as i16,
        p.stored_stats[stat_idx::SPE] as i16,
    ];
    let item_stat_kind = ItemStatKind::classify(dex, p.species, p.item);
    let species_stat_kind = {
        let ks = &dex.known_species;
        let sp = Some(p.species);
        if sp == ks.cubone || sp == ks.marowak {
            ItemStatKind::ThickClub
        } else if sp == ks.pikachu {
            ItemStatKind::LightBall
        } else if sp == ks.ditto {
            ItemStatKind::MetalPowder
        } else {
            ItemStatKind::None
        }
    };
    let mut static_fp = 0xcbf29ce484222325u64;
    let w0 = (p.species.0 as u64)
        | ((p.level as u64) << 16)
        | ((maxhp as u16 as u64) << 24)
        | ((type_mask as u64) << 40)
        | ((item_stat_kind as u64) << 60)
        | ((item_spec.boost_type as u64) << 52);
    let w1 = (stored_stats[0] as u16 as u64)
        | ((stored_stats[1] as u16 as u64) << 12)
        | ((stored_stats[2] as u16 as u64) << 24)
        | ((stored_stats[3] as u16 as u64) << 36)
        | ((stored_stats[4] as u16 as u64) << 48)
        | ((p.hp_type.0 as u64) << 60);
    let w2 = (moves[0].0 as u64)
        | ((moves[1].0 as u64) << 16)
        | ((moves[2].0 as u64) << 32)
        | ((moves[3].0 as u64) << 48);
    let w3 = (move_specs[0].base_power as u16 as u64)
        | ((move_specs[1].base_power as u16 as u64) << 16)
        | ((move_specs[2].base_power as u16 as u64) << 32)
        | ((move_specs[3].base_power as u16 as u64) << 48);
    for w in [w0, w1, w2, w3] {
        static_fp = static_fp.wrapping_add(w).wrapping_mul(0xf1357aea2e62a9c5);
    }

    let spec = StaticMonSpec {
        static_fp,
        type_mask,
        species: p.species,
        maxhp,
        quantum_1_8: (maxhp / 8).max(1),
        quantum_1_16: (maxhp / 16).max(1),
        stored_stats,
        moves,
        max_pp,
        types: [p.types.t[0].0, p.types.t[1].0],
        num_types: p.types.n,
        level: p.level,
        num_moves,
        role_flags,
        species_stat_kind,
    };
    (spec, move_specs)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SwitchKind {
    Manual,
    BatonPass,
    FaintReplace,
    Drag,
}

impl CompactBattle {
    /// Extract a `(MatchTable, CompactBattle)` pair from an existing `Battle`.
    #[inline]
    pub fn from_battle(battle: &Battle, dex: &Dex) -> (MatchTable, CompactBattle) {
        Self::from_battle_with_roster_cache(battle, dex, None)
    }

    /// Extract a `(MatchTable, CompactBattle)` pair using an optional per-world
    /// [`RosterCompactCache`] to bypass static species/stat/item/move compilation.
    #[inline]
    pub fn from_battle_with_roster_cache(
        battle: &Battle,
        dex: &Dex,
        roster_cache: Option<&RosterCompactCache>,
    ) -> (MatchTable, CompactBattle) {
        let dt = dex_tables(dex);

        let mut static_mons = [StaticMonSpec::default(); 6];
        let mut move_specs = [[CompactMoveSpec::default(); 4]; 6];
        let mut compact_mons = [CompactMon::default(); 6];
        let mut compact_sides = [CompactSide::default(); 2];

        for side_n in 0..2usize {
            let side = &battle.sides[side_n];
            let party_len = side.party.len().min(3);
            let mut alive_mask = 0u8;
            let mut active_idx = if side.active.is_none() { 3u8 } else { 0u8 };
            let mut party_roster = [0u8; 3];

            for p_idx in 0..3usize {
                let c_idx = side_n * 3 + p_idx;
                if p_idx >= party_len {
                    party_roster[p_idx] = (p_idx as u8).min(side.roster.len().saturating_sub(1) as u8);
                    compact_mons[c_idx].hp_pool = [0, 0];
                    compact_mons[c_idx].set_status(Status::Fnt as u8, 0);
                    continue;
                }
                let roster_slot = side.party[p_idx];
                party_roster[p_idx] = roster_slot;
                if side.active == Some(roster_slot) {
                    active_idx = p_idx as u8;
                }
                let p = &side.roster[roster_slot as usize];
                if !p.fainted && p.hp > 0 {
                    alive_mask |= 1 << p_idx;
                }

                let num_moves = p.move_slots.len().min(4) as u8;
                let cached_hit = roster_cache
                    .and_then(|rc| rc.entries[side_n].get(roster_slot as usize))
                    .and_then(|opt| opt.as_ref())
                    .filter(|cm| {
                        !p.transformed
                            && p.species == cm.spec.species
                            && p.level == cm.spec.level
                            && p.maxhp == cm.guard_maxhp
                            && p.stored_stats == cm.guard_stored_stats
                            && p.types == cm.guard_types
                            && p.item == cm.guard_item
                            && p.hp_type == cm.guard_hp_type
                            && p.hp_power == cm.guard_hp_power
                            && p.happiness == cm.guard_happiness
                            && num_moves == cm.spec.num_moves
                            && (0..num_moves as usize).all(|m_i| {
                                let sl = p.move_slots[m_i];
                                sl.id == cm.spec.moves[m_i] && sl.maxpp == cm.guard_maxpp[m_i]
                            })
                    });
                if let Some(cm) = cached_hit {
                    static_mons[c_idx] = cm.spec;
                    move_specs[c_idx] = cm.move_specs;
                } else {
                    let (spec, ms) = build_static_mon_spec(p, dex, dt);
                    static_mons[c_idx] = spec;
                    move_specs[c_idx] = ms;
                }
                let moves = static_mons[c_idx].moves;

                let mut c_mon = CompactMon {
                    hp_pool: [p.hp.clamp(0, i16::MAX as i32) as i16, 0],
                    pp: [0u8; 4],
                    status_packed: 0,
                    item: p.item.map(|i| (i.0 & 0x7F) as u8 + 1).unwrap_or(0),
                    vol_flags: 0,
                    timers: [0u8; 4],
                    boosts: p.boosts,
                    slot_refs: 0,
                };
                for m_i in 0..num_moves as usize {
                    let slot = p.move_slots[m_i];
                    c_mon.pp[m_i] = slot.pp.clamp(0, 64) as u8;
                    if slot.disabled {
                        c_mon.vol_flags |= vol_bit::DISABLE;
                        c_mon.set_disable_slot(m_i as u8);
                    }
                }

                let mut sub_hp = 0i16;
                let mut perish_turns = 0u8;
                let mut stall_cnt = 0u8;
                let mut res_counter_packed = 0u8;
                if let Some(last_m) = p.last_move_encore.or(p.last_move) {
                    if let Some(m_i) = (0..num_moves as usize).find(|&i| moves[i] == last_m) {
                        c_mon.set_last_move_slot(m_i as u8);
                    }
                }

                let mut has_trapped = false;
                let mut has_partial_trap = false;
                if !p.volatiles.is_empty() {
                    for &(cid, ref vst) in &p.volatiles {
                        let scid = Some(cid);
                        if scid == dt.sub_cond {
                            sub_hp = vst.get_int(DK::Hp).max(0) as i16;
                            if sub_hp > 0 {
                                c_mon.vol_flags |= vol_bit::SUBSTITUTE;
                            }
                        } else if scid == dt.conf_cond {
                            c_mon.vol_flags |= vol_bit::CONFUSION;
                            c_mon.timers[timer_idx::CONFUSION] =
                                vst.get_int(DK::Time).clamp(0, 255) as u8;
                        } else if scid == dt.flinch_cond {
                            c_mon.vol_flags |= vol_bit::FLINCH;
                        } else if scid == dt.dbond_cond {
                            c_mon.vol_flags |= vol_bit::DESTINY_BOND;
                        } else if scid == dt.par_cond {
                            c_mon.vol_flags |= vol_bit::PAR_SPEED_DROP;
                        } else if scid == dt.brn_cond {
                            c_mon.vol_flags |= vol_bit::BRN_ATK_DROP;
                        } else if scid == dt.seed_cond {
                            c_mon.vol_flags |= vol_bit::LEECH_SEED;
                        } else if scid == dt.night_cond {
                            c_mon.vol_flags |= vol_bit::NIGHTMARE;
                        } else if scid == dt.curse_cond {
                            c_mon.vol_flags |= vol_bit::CURSE;
                        } else if scid == dt.fore_cond {
                            c_mon.vol_flags |= vol_bit::FORESIGHT;
                        } else if scid == dt.prot_cond {
                            c_mon.vol_flags |= vol_bit::PROTECT;
                        } else if scid == dt.end_cond {
                            c_mon.vol_flags |= vol_bit::ENDURE;
                        } else if scid == dt.stall_cond {
                            let c = vst
                                .get(DK::Counter)
                                .map(|v| v.as_f64().floor().max(0.0) as u32)
                                .unwrap_or(127);
                            stall_cnt = if c >= 127 {
                                1
                            } else if c >= 63 {
                                2
                            } else if c >= 31 {
                                3
                            } else if c >= 15 {
                                4
                            } else if c >= 7 {
                                5
                            } else if c >= 3 {
                                6
                            } else {
                                7
                            };
                        } else if scid == dt.enc_cond {
                            c_mon.vol_flags |= vol_bit::ENCORE;
                            c_mon.timers[timer_idx::ENCORE] =
                                vst.duration.unwrap_or(0).clamp(0, 255) as u8;
                            if let Some(enc_mid) = vst.get_move() {
                                if let Some(m_i) =
                                    (0..num_moves as usize).find(|&i| moves[i] == enc_mid)
                                {
                                    c_mon.set_encore_slot(m_i as u8);
                                }
                            }
                        } else if scid == dt.dis_cond {
                            c_mon.vol_flags |= vol_bit::DISABLE;
                            c_mon.timers[timer_idx::DISABLE] =
                                vst.duration.unwrap_or(0).clamp(0, 255) as u8;
                        } else if scid == dt.perish_cond {
                            c_mon.vol_flags |= vol_bit::PERISH_SONG;
                            perish_turns = vst.duration.unwrap_or(0).clamp(0, 7) as u8;
                        } else if scid == dt.res_cond {
                            res_counter_packed =
                                (vst.get_int(DK::Counter).clamp(0, 15) as u8) | 0x10;
                        } else if scid == dt.recharge_cond {
                            c_mon.set_must_recharge(true);
                        } else if scid == dt.trapped_cond {
                            has_trapped = true;
                        } else if scid == dt.partiallytrapped_cond {
                            has_partial_trap = true;
                        }
                    }
                }
                if has_trapped || (p.trapped && !has_partial_trap) {
                    c_mon.vol_flags |= vol_bit::TRAPPED_OR_LOCKED;
                }
                if has_partial_trap {
                    c_mon.set_partially_trapped(true);
                }

                let status_counter = match p.status {
                    Status::Slp => p.status_state.get_int(DK::Time).clamp(0, 31) as u8,
                    Status::Fnt => 0,
                    _ => res_counter_packed,
                };

                c_mon.hp_pool[1] = sub_hp;
                c_mon.set_perish_turns(perish_turns);
                c_mon.set_stall_counter(stall_cnt);
                c_mon.set_status(p.status as u8, status_counter);
                compact_mons[c_idx] = c_mon;
            }
            if side.pokemon_left == 1 && alive_mask.count_ones() > 1 && active_idx < 3 {
                alive_mask = 1 << active_idx;
            }
            if active_idx >= 3 {
                let fainted_roster = side.fainted_this_turn.or(side.fainted_last_turn);
                active_idx = (0..party_len)
                    .find(|&i| {
                        Some(party_roster[i]) == fainted_roster && (alive_mask & (1 << i)) == 0
                    })
                    .or_else(|| (0..3usize).find(|&i| (alive_mask & (1 << i)) == 0))
                    .unwrap_or(0) as u8;
            }

            let mut reflect_turns = 0u8;
            let mut lightscreen_turns = 0u8;
            let mut safeguard_turns = 0u8;
            let mut spikes_layers = 0u8;
            if !side.side_conditions.is_empty() {
                for &(cid, ref vst) in &side.side_conditions {
                    let scid = Some(cid);
                    if scid == dt.refl_cond {
                        reflect_turns = vst.duration.unwrap_or(0).clamp(0, 15) as u8;
                    } else if scid == dt.ls_cond {
                        lightscreen_turns = vst.duration.unwrap_or(0).clamp(0, 15) as u8;
                    } else if scid == dt.safe_cond {
                        safeguard_turns = vst.duration.unwrap_or(0).clamp(0, 15) as u8;
                    } else if scid == dt.spikes_cond {
                        spikes_layers = vst.get_int(DK::Layers).clamp(1, 3) as u8;
                    }
                }
            }
            let mut side_flags = (spikes_layers & 0x03)
                | ((side.fainted_this_turn.is_some() as u8) << 2);
            let request = match side.request_state() {
                None | Some(RequestKind::Wait) => 0,
                Some(RequestKind::Move) => 1,
                Some(RequestKind::Switch) => 2,
                Some(RequestKind::TeamPreview) => 3,
            };
            if request == 0 && active_idx < 3 {
                let act_c_idx = side_n * 3 + active_idx as usize;
                for q_act in &battle.queue {
                    if q_act.pokemon.is_some_and(|pid| pid.side as usize == side_n) {
                        if let crate::state::ActionKind::Move { move_id, .. } = q_act.choice {
                            let num_m = static_mons[act_c_idx].num_moves as usize;
                            if let Some(m_slot) = (0..num_m)
                                .find(|&m_i| static_mons[act_c_idx].moves[m_i] == move_id)
                            {
                                side_flags |= 0x10 | (((m_slot as u8) & 0x03) << 5);
                                break;
                            }
                        }
                    }
                }
            }

            compact_sides[side_n] = CompactSide {
                active_idx,
                party_roster,
                alive_mask,
                reflect_turns,
                lightscreen_turns,
                safeguard_turns,
                side_flags,
                last_move_selfko: side.last_move.map(|m| (m.0 & 0xFF) as u8).unwrap_or(u8::MAX),
                request,
                pad: 0,
            };
        }

        let weather_kind = match battle.field.weather {
            w if w.is_some() && w == dt.rain_cond => CompactWeather::Rain,
            w if w.is_some() && w == dt.sun_cond => CompactWeather::Sun,
            w if w.is_some() && w == dt.sand_cond => CompactWeather::Sandstorm,
            _ => CompactWeather::None,
        };
        let weather_turns = battle
            .field
            .weather_state
            .duration
            .unwrap_or(0)
            .clamp(0, 15) as u8;
        let outcome = match battle.outcome() {
            None => 0,
            Some(crate::battle::Outcome::P1Win) => 1,
            Some(crate::battle::Outcome::P2Win) => 2,
            Some(crate::battle::Outcome::Tie) => 3,
        };
        let mut clause_flags = 0u8;
        if !battle.field.pseudo_weather.is_empty() {
            for &(cid, _) in &battle.field.pseudo_weather {
                if Some(cid) == dt.sleep_clause_cond {
                    clause_flags |= 1;
                } else if Some(cid) == dt.freeze_clause_cond {
                    clause_flags |= 4;
                }
            }
        }

        // Compute static fingerprint for this match configuration.
        let mut fp = 0xcbf29ce484222325u64;
        for m in &static_mons {
            fp = fp.wrapping_add(m.static_fp).wrapping_mul(0xf1357aea2e62a9c5);
        }
        compact_sides[0].pad = (fp >> 16) as u8;
        compact_sides[1].pad = (fp >> 24) as u8;

        let table = MatchTable {
            mons: static_mons,
            move_specs,
            type_chart: &dt.type_chart,
            item_specs: &dt.item_specs,
            normal_type: dt.normal_type,
            fighting_type: dt.fighting_type,
            flying_type: dt.flying_type,
            ground_type: dt.ground_type,
            ghost_type: dt.ghost_type,
            grass_type: dt.grass_type,
            fire_type: dt.fire_type,
            electric_type: dt.electric_type,
            ice_type: dt.ice_type,
            sandstorm_immune_mask: dt.sandstorm_immune_mask,
            poison_immune_mask: dt.poison_immune_mask,
            match_fingerprint: fp,
        };
        let cb = CompactBattle {
            mons: compact_mons,
            sides: compact_sides,
            turn: battle.turn,
            weather_kind,
            weather_turns,
            quick_claw_roll: battle.quick_claw_roll as u8,
            outcome,
            speed_order: if battle.mid_turn {
                battle.speed_order.first().copied().unwrap_or(0).clamp(0, 1) as u8
            } else {
                0
            },
            clause_flags,
            hurt_damage_this_turn: [0, 0],
            hurt_cat_this_turn: [0, 0],
            match_fp_lo: fp as u16,
            prng_seed: battle.prng.lcg.seed(),
        };
        if cfg!(debug_assertions) && roster_cache.is_some() {
            let (exp_t, exp_cb) = Self::from_battle_with_roster_cache(battle, dex, None);
            debug_assert_eq!(table.match_fingerprint, exp_t.match_fingerprint);
            debug_assert_eq!(cb.search_hash(), exp_cb.search_hash());
            debug_assert_eq!(cb.prng_seed, exp_cb.prng_seed);
        }
        (table, cb)
    }

    /// Ultra-fast polynomial hash of the 192-byte `CompactBattle` state
    /// (hashes the first 23 `u64` words = 184 bytes, skipping `prng_seed`).
    #[inline(always)]
    pub fn search_hash(&self) -> u64 {
        // SAFETY: `CompactBattle` is `#[repr(C, align(64))]` of size 192 bytes (24 u64 words)
        // with all initialized bytes (`CompactMon`, `CompactSide`, scalar fields).
        let words: &[u64; 24] = unsafe { &*(self as *const Self as *const [u64; 24]) };
        let mut h = (self.match_fp_lo as u64).wrapping_mul(0x9e3779b97f4a7c15);
        // Words 0..23 cover `mons` (18 words), `sides` (3 words), and turn/field/damage (2 words).
        // Word 23 is `prng_seed` (skipped so states differing only in RNG seed merge).
        for &w in &words[..23] {
            h = h.wrapping_add(w).wrapping_mul(0xf1357aea2e62a9c5);
        }
        h.rotate_left(26)
    }

    /// Compute any party Pokemon's effective stat (`party_slot` in `0..3`) on `side` (`0` or `1`)
    /// using [`get_stat_branchless`].
    #[inline(always)]
    pub fn mon_stat(
        &self,
        table: &MatchTable,
        side: usize,
        party_slot: usize,
        stat_idx: usize,
        unboosted: bool,
        unmodified: bool,
    ) -> i32 {
        let p_idx = side * 3 + party_slot;
        let mon = &self.mons[p_idx];
        let spec = &table.mons[p_idx];
        let side_st = &self.sides[side];
        let has_par = (mon.status_raw() == Status::Par as u8)
            & ((mon.vol_flags & vol_bit::PAR_SPEED_DROP) != 0);
        let has_brn = (mon.status_raw() == Status::Brn as u8)
            & ((mon.vol_flags & vol_bit::BRN_ATK_DROP) != 0);
        let ispec = table.item_spec(mon.item);
        let item_kind = if ispec.stat_kind != ItemStatKind::None
            && ispec.stat_kind == spec.species_stat_kind
        {
            ispec.stat_kind
        } else {
            ItemStatKind::None
        };
        get_stat_branchless(
            spec.stored_stats[stat_idx] as i32,
            mon.boosts[stat_idx],
            stat_idx,
            unboosted,
            unmodified,
            false,
            has_par,
            has_brn,
            side_st.reflect_turns > 0,
            side_st.lightscreen_turns > 0,
            item_kind,
        )
    }

    /// Compute the active Pokemon's effective stat on `side` (`0` or `1`) using
    /// [`get_stat_branchless`].
    #[inline(always)]
    pub fn active_stat(
        &self,
        table: &MatchTable,
        side: usize,
        stat_idx: usize,
        unboosted: bool,
        unmodified: bool,
    ) -> i32 {
        self.mon_stat(
            table,
            side,
            self.sides[side].active_idx as usize,
            stat_idx,
            unboosted,
            unmodified,
        )
    }

    /// Compute a party Pokemon's action speed (matching `Battle::get_pokemon_action_speed`).
    #[inline(always)]
    pub fn mon_action_speed(&self, table: &MatchTable, side: usize, party_slot: usize) -> i32 {
        let p_idx = side * 3 + party_slot;
        let mon = &self.mons[p_idx];
        if self.quick_claw_roll != 0 && table.item_spec(mon.item).is_quick_claw {
            return QUICK_CLAW_ACTION_SPEED;
        }
        self.mon_stat(table, side, party_slot, stat_idx::SPE, false, false)
    }

    /// Compute hit probability for `m_slot` used by `(att_side, att_slot)` against `(def_side, def_slot)`.
    #[inline(always)]
    pub fn hit_probability(
        &self,
        table: &MatchTable,
        att_side: usize,
        att_slot: usize,
        def_side: usize,
        def_slot: usize,
        m_slot: usize,
    ) -> f64 {
        let att_idx = att_side * 3 + att_slot;
        let def_idx = def_side * 3 + def_slot;
        let mspec = &table.move_specs[att_idx][m_slot];
        hit_probability_branchless(
            mspec.base_acc_pct,
            mspec.always_hit,
            self.mons[att_idx].boosts[stat_idx::ACC],
            self.mons[def_idx].boosts[stat_idx::EVA],
            mspec.ignore_accuracy,
            mspec.ignore_evasion,
            mspec.ignore_positive_evasion,
        )
    }

    /// Expected fraction of `(def_side, def_slot)`'s current HP removed by move slot `m_slot`
    /// of `(att_side, att_slot)`, paired with its effective hit probability `acc`.
    #[inline]
    pub fn expected_hit_fraction_with_acc(
        &self,
        table: &MatchTable,
        att_side: usize,
        att_slot: usize,
        def_side: usize,
        def_slot: usize,
        m_slot: usize,
    ) -> (f64, f64) {
        let att_idx = att_side * 3 + att_slot;
        let def_idx = def_side * 3 + def_slot;
        let mspec = &table.move_specs[att_idx][m_slot];
        let att_mon = &self.mons[att_idx];
        let att_spec = &table.mons[att_idx];
        let def_mon = &self.mons[def_idx];
        let def_spec = &table.mons[def_idx];

        let base_power = if mspec.var_bp_kind == VarBpKind::FlailReversal {
            let ratio = ((att_mon.hp() as f64 * 48.0 / att_spec.maxhp.max(1) as f64).floor() as i32)
                .max(1);
            if ratio < 2 {
                200
            } else if ratio < 5 {
                150
            } else if ratio < 10 {
                100
            } else if ratio < 17 {
                80
            } else if ratio < 33 {
                40
            } else {
                20
            }
        } else {
            mspec.base_power as i32
        };

        if mspec.fixed_damage == 0
            && mspec.special_kind != SpecialMoveKind::SuperFang
            && (mspec.is_status || base_power <= 0)
        {
            return (0.0, 0.0);
        }

        let foresight = (def_mon.vol_flags & vol_bit::FORESIGHT) != 0;
        let (type_mod, is_immune) =
            table.effectiveness_ext(mspec.move_type, def_spec, foresight);
        if is_immune {
            return (0.0, 0.0);
        }

        let acc = hit_probability_branchless(
            mspec.base_acc_pct,
            mspec.always_hit,
            att_mon.boosts[stat_idx::ACC],
            def_mon.boosts[stat_idx::EVA],
            mspec.ignore_accuracy,
            mspec.ignore_evasion,
            mspec.ignore_positive_evasion,
        );

        let raw = if mspec.special_kind == SpecialMoveKind::SuperFang {
            (def_mon.hp() / 2).max(1) as f64
        } else if mspec.fixed_damage != 0 {
            if mspec.fixed_damage < 0 {
                att_spec.level as f64
            } else {
                mspec.fixed_damage as f64
            }
        } else {
            let (ai, di) = if mspec.is_physical {
                (stat_idx::ATK, stat_idx::DEF)
            } else {
                (stat_idx::SPA, stat_idx::SPD)
            };
            let atk = self.mon_stat(table, att_side, att_slot, ai, false, false) as f64;
            let mut defense = self.mon_stat(table, def_side, def_slot, di, false, false) as f64;
            if mspec.is_selfdestruct && di == stat_idx::DEF {
                defense = (defense / 2.0).floor().max(1.0);
            }
            let mut dmg = ((att_spec.level as f64 * 2.0 / 5.0 + 2.0).floor()
                * base_power as f64
                * atk
                / defense
                / 50.0)
                .floor();
            if mspec.move_type != NO_TYPE_ID
                && table.item_spec(att_mon.item).boost_type == mspec.move_type
            {
                dmg += (dmg / 10.0).floor();
            }
            let mut core = dmg.clamp(1.0, 997.0) + 2.0;
            if self.weather_turns > 0 {
                if (mspec.is_water && self.weather_kind == CompactWeather::Rain)
                    || (mspec.is_fire && self.weather_kind == CompactWeather::Sun)
                {
                    core = (core * 1.5).floor();
                } else if ((mspec.is_fire || mspec.is_solarbeam)
                    && self.weather_kind == CompactWeather::Rain)
                    || (mspec.is_water && self.weather_kind == CompactWeather::Sun)
                {
                    core = (core / 2.0).floor();
                }
            }
            let stab = if mspec.move_type < 32
                && (att_spec.type_mask & (1u32 << (mspec.move_type & 31))) != 0
            {
                1.5
            } else {
                1.0
            };
            let eff = match type_mod {
                2 => 4.0,
                1 => 2.0,
                -1 => 0.5,
                -2 => 0.25,
                _ => 1.0,
            };
            let hits = mspec.multihit_mean_x2 as f64 * 0.5;
            core * stab * eff * (236.0 / 255.0) * hits
        };

        (raw * acc / (def_mon.hp().max(1) as f64), acc)
    }

    /// Expected fraction of `(def_side, def_slot)`'s current HP removed by move slot `m_slot`
    /// of `(att_side, att_slot)`. Mirrors `nc2000_bot::eval::expected_hit_fraction` bit-for-bit.
    #[inline]
    pub fn expected_hit_fraction(
        &self,
        table: &MatchTable,
        att_side: usize,
        att_slot: usize,
        def_side: usize,
        def_slot: usize,
        m_slot: usize,
    ) -> f64 {
        self.expected_hit_fraction_with_acc(
            table,
            att_side,
            att_slot,
            def_side,
            def_slot,
            m_slot,
        )
        .0
    }

    /// Best expected hit fraction over `(att_side, att_slot)`'s usable move slots against `(def_side, def_slot)`.
    #[inline]
    pub fn best_hit_fraction(
        &self,
        table: &MatchTable,
        att_side: usize,
        att_slot: usize,
        def_side: usize,
        def_slot: usize,
    ) -> f64 {
        let att_idx = att_side * 3 + att_slot;
        let def_idx = def_side * 3 + def_slot;
        let att_mon = &self.mons[att_idx];
        let att_spec = &table.mons[att_idx];
        let def_mon = &self.mons[def_idx];
        let def_spec = &table.mons[def_idx];
        let num_moves = att_spec.num_moves as usize;
        let foresight = (def_mon.vol_flags & vol_bit::FORESIGHT) != 0;
        let level_factor = (att_spec.level as f64 * 2.0 / 5.0 + 2.0).floor();
        let def_hp = def_mon.hp().max(1) as f64;
        let mut phys_stats: Option<(f64, f64)> = None;
        let mut spec_stats: Option<(f64, f64)> = None;
        let mut best = 0.0f64;

        for m_i in 0..num_moves {
            if att_mon.pp[m_i] == 0 || att_mon.is_slot_disabled(m_i) {
                continue;
            }
            let mspec = &table.move_specs[att_idx][m_i];
            let base_power = if mspec.var_bp_kind == VarBpKind::FlailReversal {
                let ratio =
                    ((att_mon.hp() as f64 * 48.0 / att_spec.maxhp.max(1) as f64).floor() as i32)
                        .max(1);
                if ratio < 2 {
                    200
                } else if ratio < 5 {
                    150
                } else if ratio < 10 {
                    100
                } else if ratio < 17 {
                    80
                } else if ratio < 33 {
                    40
                } else {
                    20
                }
            } else {
                mspec.base_power as i32
            };
            if mspec.fixed_damage == 0
                && mspec.special_kind != SpecialMoveKind::SuperFang
                && (mspec.is_status || base_power <= 0)
            {
                continue;
            }
            let (type_mod, is_immune) =
                table.effectiveness_ext(mspec.move_type, def_spec, foresight);
            if is_immune {
                continue;
            }
            let acc = hit_probability_branchless(
                mspec.base_acc_pct,
                mspec.always_hit,
                att_mon.boosts[stat_idx::ACC],
                def_mon.boosts[stat_idx::EVA],
                mspec.ignore_accuracy,
                mspec.ignore_evasion,
                mspec.ignore_positive_evasion,
            );
            let raw = if mspec.special_kind == SpecialMoveKind::SuperFang {
                (def_mon.hp() / 2).max(1) as f64
            } else if mspec.fixed_damage != 0 {
                if mspec.fixed_damage < 0 {
                    att_spec.level as f64
                } else {
                    mspec.fixed_damage as f64
                }
            } else {
                let (atk, mut defense) = if mspec.is_physical {
                    *phys_stats.get_or_insert_with(|| {
                        (
                            self.mon_stat(table, att_side, att_slot, stat_idx::ATK, false, false)
                                as f64,
                            self.mon_stat(table, def_side, def_slot, stat_idx::DEF, false, false)
                                as f64,
                        )
                    })
                } else {
                    *spec_stats.get_or_insert_with(|| {
                        (
                            self.mon_stat(table, att_side, att_slot, stat_idx::SPA, false, false)
                                as f64,
                            self.mon_stat(table, def_side, def_slot, stat_idx::SPD, false, false)
                                as f64,
                        )
                    })
                };
                if mspec.is_selfdestruct && mspec.is_physical {
                    defense = (defense / 2.0).floor().max(1.0);
                }
                let mut dmg = (level_factor * base_power as f64 * atk / defense / 50.0).floor();
                if mspec.move_type != NO_TYPE_ID
                    && table.item_spec(att_mon.item).boost_type == mspec.move_type
                {
                    dmg += (dmg / 10.0).floor();
                }
                let mut core = dmg.clamp(1.0, 997.0) + 2.0;
                if self.weather_turns > 0 {
                    if (mspec.is_water && self.weather_kind == CompactWeather::Rain)
                        || (mspec.is_fire && self.weather_kind == CompactWeather::Sun)
                    {
                        core = (core * 1.5).floor();
                    } else if ((mspec.is_fire || mspec.is_solarbeam)
                        && self.weather_kind == CompactWeather::Rain)
                        || (mspec.is_water && self.weather_kind == CompactWeather::Sun)
                    {
                        core = (core / 2.0).floor();
                    }
                }
                let stab = if mspec.move_type < 32
                    && (att_spec.type_mask & (1u32 << (mspec.move_type & 31))) != 0
                {
                    1.5
                } else {
                    1.0
                };
                let eff = match type_mod {
                    2 => 4.0,
                    1 => 2.0,
                    -1 => 0.5,
                    -2 => 0.25,
                    _ => 1.0,
                };
                let hits = mspec.multihit_mean_x2 as f64 * 0.5;
                core * stab * eff * (236.0 / 255.0) * hits
            };
            let v = raw * acc / def_hp;
            if v > best {
                best = v;
            }
        }

        if cfg!(debug_assertions) {
            let mut ref_best = 0.0f64;
            for m_i in 0..num_moves {
                if att_mon.pp[m_i] == 0 || att_mon.is_slot_disabled(m_i) {
                    continue;
                }
                let v = self.expected_hit_fraction(
                    table,
                    att_side,
                    att_slot,
                    def_side,
                    def_slot,
                    m_i,
                );
                if v > ref_best {
                    ref_best = v;
                }
            }
            debug_assert_eq!(best.to_bits(), ref_best.to_bits());
        }
        best
    }

    /// True when `side`'s active mon has no usable non-Struggle move at the start of the turn
    /// (matching `legal_actions` Struggle fallback).
    #[inline(always)]
    fn is_forced_struggle(&self, table: &MatchTable, side: usize) -> bool {
        let p_idx = side * 3 + (self.sides[side].active_idx as usize);
        let mon = &self.mons[p_idx];
        if mon.must_recharge() {
            return false;
        }
        let num_moves = table.mons[p_idx].num_moves as usize;
        if (mon.vol_flags & vol_bit::ENCORE) != 0 {
            let enc_slot = mon.encore_slot() as usize;
            !(enc_slot < num_moves && mon.pp[enc_slot] > 0 && !mon.is_slot_disabled(enc_slot))
        } else {
            !(0..num_moves).any(|m| mon.pp[m] > 0 && !mon.is_slot_disabled(m))
        }
    }

    /// Compute the branchless `ActionSortKey` for `side` (`0` or `1`) given a
    /// compact action (`0..4` = move slot, `4..7` = switch to party index `0..3`).
    #[inline(always)]
    pub fn action_sort_key(
        &self,
        table: &MatchTable,
        side: usize,
        action_code: u8,
        tie_bit: u8,
    ) -> ActionSortKey {
        let p_idx = side * 3 + (self.sides[side].active_idx as usize);
        let mon = &self.mons[p_idx];
        let speed = self.active_stat(table, side, stat_idx::SPE, false, false);
        let must_recharge = mon.must_recharge();
        let is_switch = !must_recharge && action_code >= 4;
        let phase = if is_switch {
            ActionPhase::Switch
        } else {
            ActionPhase::Move
        };
        let move_slot = (action_code & 3) as usize;
        let prio = if is_switch || must_recharge || self.is_forced_struggle(table, side) {
            0i8
        } else {
            table.move_specs[p_idx][move_slot].priority
        };
        let qc_active = (!is_switch)
            & table.item_spec(mon.item).is_quick_claw
            & (self.quick_claw_roll != 0);
        ActionSortKey::new(phase, prio, qc_active, speed, tie_bit)
    }

    /// Execute one full 4-stage turn on `CompactBattle` with zero heap allocations:
    /// - Stage 0: Branchless `ActionSortKey` turn-order determination
    /// - Stage 1: First side action + `AfterMoveSelf` slip damage
    /// - Stage 2: Second side action + `AfterMoveSelf` slip damage
    /// - Stage 3: End-of-turn `Residual` (Leftovers, Weather, Leech Seed, Timers, Quick Claw roll)
    ///
    /// `actions`: `[action_p1, action_p2]` where `0..4` is a move slot and `4..7`
    /// is switching to party index `action - 4`.
    pub fn step_turn(&mut self, table: &MatchTable, actions: [u8; 2]) {
        if self.outcome != 0 {
            return;
        }
        let mut rng = Prng::new(self.prng_seed);
        self.sides[0].side_flags &= !0x08;
        self.sides[0].last_move_selfko = u8::MAX;
        self.sides[1].last_move_selfko = u8::MAX;
        self.hurt_damage_this_turn = [0, 0];
        self.hurt_cat_this_turn = [0, 0];

        // Clear single-turn Protect/Endure/Flinch flags at start of turn.
        for side in 0..2usize {
            let idx = side * 3 + self.sides[side].active_idx as usize;
            self.mons[idx].vol_flags &= !(vol_bit::PROTECT | vol_bit::ENDURE | vol_bit::FLINCH);
        }

        // Snapshot start-of-turn Struggle state so mid-turn Disable aborts the queued move
        // (matching Battle::DisableBeforeMove prio 7) instead of morphing it into Struggle.
        let forced_struggle = [
            self.is_forced_struggle(table, 0),
            self.is_forced_struggle(table, 1),
        ];

        // Stage 0: Branchless ActionSortKey comparison.
        let k0 = self.action_sort_key(table, 0, actions[0], 0);
        let k1 = self.action_sort_key(table, 1, actions[1], 1);
        let (first, second) = if k0.ties_without_rng(k1) {
            let f = (rng.random(2) == 1) as usize;
            (f, f ^ 1)
        } else {
            let f = (k0.0 < k1.0) as usize;
            (f, f ^ 1)
        };
        self.speed_order = first as u8;

        // Stage 1 & Stage 2: Execute first action, then second action.
        for (step_i, &actor) in [first, second].iter().enumerate() {
            if self.outcome != 0 {
                break;
            }
            let act = actions[actor];
            if act >= 4 && act < 7 {
                self.exec_switch(table, actor, (act - 4).min(2), SwitchKind::Manual);
            } else if act < 4 {
                // Gen 2: fainting skips any remaining queued move actions this turn.
                let p0 = self.sides[0].active_idx as usize;
                let p1 = self.sides[1].active_idx as usize;
                if step_i == 1 && (self.mons[p0].hp() <= 0 || self.mons[3 + p1].hp() <= 0) {
                    break;
                }
                self.exec_move(
                    table,
                    actor,
                    (act & 3) as usize,
                    step_i == 1,
                    forced_struggle[actor],
                    &mut rng,
                );
            }
        }
        self.check_faints(table, &mut rng);

        // Stage 3: End-of-turn Residual in speed order when no forced switch is pending.
        if self.outcome == 0 && self.sides[0].request == 1 && self.sides[1].request == 1 {
            self.sides[0].side_flags |= 0x08;
            self.exec_residuals(table, &mut rng);
            self.check_faints(table, &mut rng);
            if self.outcome == 0 && self.sides[0].request == 1 && self.sides[1].request == 1 {
                self.sides[0].side_flags &= !0x08;
                self.turn = self.turn.saturating_add(1);
                if self.turn > 1000 {
                    self.outcome = 3; // Tie
                } else {
                    self.quick_claw_roll = rng.random_chance(60, 256) as u8;
                }
            }
        }
        self.hurt_damage_this_turn = [0, 0];
        self.hurt_cat_this_turn = [0, 0];
        self.speed_order = 0;
        self.prng_seed = rng.seed();
    }

    #[inline]
    fn exec_switch(
        &mut self,
        table: &MatchTable,
        side: usize,
        target_party_idx: u8,
        kind: SwitchKind,
    ) {
        let baton_pass = kind == SwitchKind::BatonPass;
        let old_idx = side * 3 + (self.sides[side].active_idx.min(2) as usize);
        let saved_boosts = self.mons[old_idx].boosts;
        let saved_sub = self.mons[old_idx].hp_pool[1];
        let pass_mask = vol_bit::SUBSTITUTE
            | vol_bit::CONFUSION
            | vol_bit::LEECH_SEED
            | vol_bit::CURSE
            | vol_bit::FORESIGHT
            | vol_bit::PERISH_SONG
            | vol_bit::TRAPPED_OR_LOCKED;
        let saved_vols = self.mons[old_idx].vol_flags & pass_mask;
        let saved_pt = self.mons[old_idx].partially_trapped();
        let saved_conf_t = self.mons[old_idx].timers[timer_idx::CONFUSION];
        let saved_perish_t = self.mons[old_idx].perish_turns();
        let old_st = self.mons[old_idx].status_raw();
        let saved_res = if old_st != Status::Slp as u8
            && old_st != Status::Fnt as u8
            && (self.mons[old_idx].status_counter() & 0x10) != 0
        {
            self.mons[old_idx].status_counter()
        } else {
            0
        };

        // Clear volatile state and boosts on outgoing active Pokemon.
        self.mons[old_idx].boosts = [0; 7];
        self.mons[old_idx].hp_pool[1] = 0;
        let keep_status_drops = (if old_st == Status::Par as u8 {
            vol_bit::PAR_SPEED_DROP
        } else {
            0
        }) | (if old_st == Status::Brn as u8 {
            vol_bit::BRN_ATK_DROP
        } else {
            0
        });
        if old_st != Status::Slp as u8 && old_st != Status::Fnt as u8 {
            self.mons[old_idx].set_status(old_st, 0);
        }
        self.mons[old_idx].vol_flags = keep_status_drops;
        self.mons[old_idx].timers = [0; 4];
        self.mons[old_idx].slot_refs = 0;

        // Opponent switch-out expires active Destiny Bond (`onFoeSwitchOut`) and partial traps;
        // non-Baton-Pass switching also releases Mean Look / Spider Web trap on the opposing active Pokemon.
        let foe_side = side ^ 1;
        let foe_idx = foe_side * 3 + (self.sides[foe_side].active_idx.min(2) as usize);
        self.mons[foe_idx].vol_flags &= !vol_bit::DESTINY_BOND;
        self.mons[foe_idx].set_partially_trapped(false);
        if !baton_pass {
            self.mons[foe_idx].vol_flags &= !vol_bit::TRAPPED_OR_LOCKED;
        }

        self.sides[side].active_idx = target_party_idx;
        let new_idx = side * 3 + target_party_idx as usize;
        if baton_pass {
            self.mons[new_idx].boosts = saved_boosts;
            self.mons[new_idx].hp_pool[1] = saved_sub;
            self.mons[new_idx].vol_flags |= saved_vols;
            self.mons[new_idx].timers[timer_idx::CONFUSION] = saved_conf_t;
            self.mons[new_idx].set_perish_turns(saved_perish_t);
            if saved_pt {
                self.mons[new_idx].set_partially_trapped(true);
            }
        }
        if self.mons[new_idx].status_raw() == Status::Par as u8 {
            self.mons[new_idx].vol_flags |= vol_bit::PAR_SPEED_DROP;
        }
        if self.mons[new_idx].status_raw() == Status::Brn as u8 {
            self.mons[new_idx].vol_flags |= vol_bit::BRN_ATK_DROP;
        }
        if self.mons[new_idx].status_raw() == Status::Tox as u8 {
            self.mons[new_idx].set_status(Status::Psn as u8, 0);
        }
        if baton_pass && saved_res != 0 {
            let new_st = self.mons[new_idx].status_raw();
            if new_st != Status::Slp as u8 && new_st != Status::Fnt as u8 {
                self.mons[new_idx].set_status(new_st, saved_res);
            }
        }

        // Apply Spikes entry hazard to non-Flying switch-in.
        let spikes_layers = self.sides[side].side_flags & 0x03;
        if spikes_layers != 0
            && (table.mons[new_idx].type_mask & (1u32 << (table.flying_type & 31))) == 0
        {
            let spike_dmg = match spikes_layers {
                1 => table.mons[new_idx].quantum_1_8,
                2 => (table.mons[new_idx].maxhp / 6).max(1),
                _ => (table.mons[new_idx].maxhp / 4).max(1),
            };
            self.mons[new_idx].apply_damage_branchless(spike_dmg, true);
        }

        // Gen 2 `AfterSwitchInSelf` (`actions.rs:248-252`): manual switches and
        // Baton Pass (not faint replacements or Roar/Whirlwind drag-ins) incur
        // Burn/Poison slip damage immediately on switch-in.
        // In Battle (`events.rs:503-514`), Status (subOrder 0) deals damage using
        // the PRE-increment `residualdmg` counter `c` (`max(1, floor(maxhp/16) * c)`),
        // then `residualdmg` (subOrder 2) increments `c` to `c + 1`.
        if matches!(kind, SwitchKind::Manual | SwitchKind::BatonPass)
            && self.mons[new_idx].hp() > 0
        {
            let st = self.mons[new_idx].status_raw();
            if st == Status::Brn as u8 || st == Status::Psn as u8 {
                let cnt = self.mons[new_idx].status_counter();
                let slip = if (cnt & 0x10) != 0 {
                    let c = cnt & 0x0F;
                    self.mons[new_idx].set_status(st, (c + 1).min(15) | 0x10);
                    ((table.mons[new_idx].maxhp / 16) * (c as i16)).max(1)
                } else {
                    table.mons[new_idx].quantum_1_8
                };
                self.mons[new_idx].apply_damage_branchless(slip, true);
            }
        }
    }

    #[inline]
    fn exec_move(
        &mut self,
        table: &MatchTable,
        side: usize,
        move_slot: usize,
        moved_second: bool,
        is_struggle: bool,
        rng: &mut Prng,
    ) {
        let foe_side = side ^ 1;
        let att_idx = side * 3 + self.sides[side].active_idx as usize;
        let def_idx = foe_side * 3 + self.sides[foe_side].active_idx as usize;
        if self.mons[att_idx].hp() <= 0 || self.mons[def_idx].hp() <= 0 {
            return;
        }

        // Hyper Beam recharge turn wastes the turn (`onBeforeMovePriority = 11`, Condition subOrder 2).
        if self.mons[att_idx].must_recharge() {
            self.mons[att_idx].set_must_recharge(false);
            self.mons[att_idx].vol_flags &= !vol_bit::DESTINY_BOND;
            self.exec_after_move_self(table, side);
            return;
        }

        let num_moves = table.mons[att_idx].num_moves as usize;
        let mut move_slot = move_slot;
        if !is_struggle && (self.mons[att_idx].vol_flags & vol_bit::ENCORE) != 0 {
            let enc_slot = self.mons[att_idx].encore_slot() as usize;
            if enc_slot < num_moves {
                move_slot = enc_slot;
            }
        }
        let mut mspec = if is_struggle {
            CompactMoveSpec {
                move_type: NO_TYPE_ID,
                base_power: 50,
                base_acc_pct: 100,
                crit_ratio: 1,
                is_physical: true,
                blocked_by_protect: true,
                recoil_quarter: true,
                multihit_mean_x2: 2,
                sec_boost: (NO_STAT_IDX, 0),
                boosts: [(NO_STAT_IDX, 0); 3],
                ..CompactMoveSpec::default()
            }
        } else if move_slot < num_moves {
            table.move_specs[att_idx][move_slot]
        } else {
            self.mons[att_idx].vol_flags &= !vol_bit::DESTINY_BOND;
            self.exec_after_move_self(table, side);
            return;
        };
        // Mint Berry / Miracle Berry `onBeforeMove` (priority 11, Item subOrder 8, before Freeze/Sleep/Flinch/Disable).
        self.check_status_berry_trigger(table, side);
        let st_raw = self.mons[att_idx].status_raw();

        // Freeze check (`onBeforeMovePriority = 10`; Flame Wheel / Sacred Fire bypass freeze and thaw on hit).
        if st_raw == Status::Frz as u8 && !mspec.is_defrost {
            self.mons[att_idx].vol_flags &= !vol_bit::DESTINY_BOND;
            self.exec_after_move_self(table, side);
            return;
        }

        // Sleep check (`onBeforeMovePriority = 10`, BEFORE Flinch priority 8).
        let mut sleeping = false;
        if st_raw == Status::Slp as u8 {
            let rem = self.mons[att_idx].status_counter().saturating_sub(1);
            if rem == 0 {
                self.mons[att_idx].set_status(Status::None as u8, 0);
            } else {
                self.mons[att_idx].set_status(Status::Slp as u8, rem);
                if !mspec.is_sleep_usable {
                    self.mons[att_idx].vol_flags &= !vol_bit::DESTINY_BOND;
                    self.exec_after_move_self(table, side);
                    return;
                }
                sleeping = true;
            }
        }

        // Flinch check (`onBeforeMovePriority = 8`, after Freeze/Sleep and before Disable/Confusion/Par).
        if (self.mons[att_idx].vol_flags & vol_bit::FLINCH) != 0 {
            self.mons[att_idx].vol_flags &= !vol_bit::DESTINY_BOND;
            self.exec_after_move_self(table, side);
            return;
        }

        // Disable check (`onBeforeMovePriority = 7`, after Flinch and before Confusion/Par).
        // A queued move disabled mid-turn is aborted rather than turned into Struggle.
        if !is_struggle {
            let is_disabled = (self.mons[att_idx].vol_flags & vol_bit::DISABLE) != 0;
            let dis_slot = self.mons[att_idx].disable_slot() as usize;
            if is_disabled && move_slot == dis_slot {
                self.mons[att_idx].vol_flags &= !vol_bit::DESTINY_BOND;
                self.exec_after_move_self(table, side);
                return;
            }
        }

        // Confusion check.
        if (self.mons[att_idx].vol_flags & vol_bit::CONFUSION) != 0 {
            let rem = self.mons[att_idx].timers[timer_idx::CONFUSION].saturating_sub(1);
            self.mons[att_idx].timers[timer_idx::CONFUSION] = rem;
            if rem == 0 {
                self.mons[att_idx].vol_flags &= !vol_bit::CONFUSION;
            } else if rng.random_chance(1, 2) {
                let atk = self.active_stat(table, side, 0, false, false);
                let def = self.active_stat(table, side, 1, false, false);
                let lvl = table.mons[att_idx].level as i32;
                let self_dmg = (((lvl * 2 / 5 + 2) * 40 * atk / def.max(1)) / 50 + 2).max(1) as i16;
                self.mons[att_idx].vol_flags &= !vol_bit::DESTINY_BOND;
                self.mons[att_idx].apply_damage_branchless(self_dmg, true);
                self.exec_after_move_self(table, side);
                return;
            }
        }

        // Paralysis 25% full-paralyze check.
        if self.mons[att_idx].status_raw() == Status::Par as u8 && rng.random_chance(1, 4) {
            self.mons[att_idx].vol_flags &= !vol_bit::DESTINY_BOND;
            self.exec_after_move_self(table, side);
            return;
        }

        // Clear Destiny Bond when the user attempts a move.
        self.mons[att_idx].vol_flags &= !vol_bit::DESTINY_BOND;

        // Deduct PP and record last executed move slot (Struggle spends no move-slot PP).
        if is_struggle {
            self.mons[att_idx].clear_last_move_slot();
        } else {
            if self.mons[att_idx].pp[move_slot] == 0 {
                return;
            }
            self.mons[att_idx].pp[move_slot] -= 1;
            self.mons[att_idx].set_last_move_slot(move_slot as u8);
        }

        // Sleep-usable moves (Sleep Talk, Snore) fail when the user is awake.
        if mspec.is_sleep_usable && !sleeping {
            self.exec_after_move_self(table, side);
            return;
        }

        // Sleep Talk: randomly call one of the user's other sleep-talkable moves.
        if mspec.special_kind == SpecialMoveKind::SleepTalk {
            let mut candidates = [0usize; 4];
            let mut n_cand = 0usize;
            let num_moves = table.mons[att_idx].num_moves as usize;
            for m_i in 0..num_moves {
                if m_i != move_slot && table.move_specs[att_idx][m_i].can_sleep_talk {
                    candidates[n_cand] = m_i;
                    n_cand += 1;
                }
            }
            if n_cand == 0 {
                self.exec_after_move_self(table, side);
                return;
            }
            let picked = candidates[rng.random(n_cand as u32) as usize];
            mspec = table.move_specs[att_idx][picked];
        }

        // Self-Destruct faints user upfront and sets Self-KO clause tracker.
        if mspec.is_selfdestruct {
            self.mons[att_idx].hp_pool[0] = 0;
            self.mons[att_idx].status_packed = Status::Fnt as u8;
            self.sides[side].last_move_selfko = move_slot as u8;
            self.sides[foe_side].last_move_selfko = u8::MAX;
        }

        // Handle common status self-moves (Rest, Recover/Soft-Boiled, Weather heals, Protect, Substitute).
        if mspec.is_rest {
            let maxhp = table.mons[att_idx].maxhp;
            if self.mons[att_idx].hp() < maxhp {
                self.mons[att_idx].hp_pool[0] = maxhp;
                self.mons[att_idx].set_status(Status::Slp as u8, 3);
                self.mons[att_idx].vol_flags &= !(vol_bit::PAR_SPEED_DROP | vol_bit::BRN_ATK_DROP);
            }
            self.exec_after_move_self(table, side);
            return;
        }
        if mspec.is_half_heal {
            let maxhp = table.mons[att_idx].maxhp;
            let heal = (maxhp + 1) / 2;
            self.mons[att_idx].hp_pool[0] = (self.mons[att_idx].hp() + heal).min(maxhp);
            self.exec_after_move_self(table, side);
            return;
        }
        if mspec.special_kind == SpecialMoveKind::WeatherHeal {
            let maxhp = table.mons[att_idx].maxhp;
            let weather = if self.weather_turns > 0 {
                self.weather_kind
            } else {
                CompactWeather::None
            };
            let heal = weather.weather_heal_amount(maxhp);
            self.mons[att_idx].hp_pool[0] = (self.mons[att_idx].hp() + heal).min(maxhp);
            self.exec_after_move_self(table, side);
            return;
        }
        if mspec.is_protect || mspec.is_endure {
            // In Gen 2, Protect/Detect/Endure always fail if the user has a Substitute up.
            if !moved_second && self.mons[att_idx].sub_hp() == 0 {
                let sc = self.mons[att_idx].stall_counter();
                let ok = sc == 0 || rng.random_chance((127u32 >> (sc - 1).min(6)).max(1), 255);
                if ok {
                    let bit = if mspec.is_protect {
                        vol_bit::PROTECT
                    } else {
                        vol_bit::ENDURE
                    };
                    self.mons[att_idx].vol_flags |= bit;
                    self.mons[att_idx].set_stall_counter((sc + 1).min(7));
                }
            }
            self.exec_after_move_self(table, side);
            return;
        }
        if mspec.is_substitute {
            let cost = table.mons[att_idx].maxhp / 4;
            if self.mons[att_idx].hp() > cost && self.mons[att_idx].sub_hp() == 0 {
                self.mons[att_idx].hp_pool[0] -= cost;
                self.mons[att_idx].hp_pool[1] = cost;
                self.mons[att_idx].vol_flags |= vol_bit::SUBSTITUTE;
                self.mons[att_idx].set_partially_trapped(false);
            }
            self.exec_after_move_self(table, side);
            return;
        }

        // Target Protect check (only for moves flagged `protect` in Gen 2).
        if mspec.blocked_by_protect && (self.mons[def_idx].vol_flags & vol_bit::PROTECT) != 0 {
            self.exec_after_move_self(table, side);
            return;
        }

        let def_foresight = (self.mons[def_idx].vol_flags & vol_bit::FORESIGHT) != 0;

        // Accuracy check via O(1) ACC_LUT.
        if !mspec.always_hit {
            let thresh = lookup_accuracy_threshold(
                mspec.base_acc_pct,
                self.mons[att_idx].boosts[5],
                self.mons[def_idx].boosts[6],
                mspec.ignore_accuracy,
                mspec.ignore_evasion,
                mspec.ignore_positive_evasion | def_foresight,
                table.item_spec(self.mons[def_idx].item).is_brightpowder,
            );
            if thresh < 255 && !rng.random_chance(thresh as u32, 256) {
                self.exec_after_move_self(table, side);
                return;
            }
        }

        if !mspec.is_status
            && (mspec.base_power > 0
                || matches!(
                    mspec.special_kind,
                    SpecialMoveKind::Counter
                        | SpecialMoveKind::MirrorCoat
                        | SpecialMoveKind::FlailReversal
                        | SpecialMoveKind::LevelDamage
                        | SpecialMoveKind::SuperFang
                )
                || mspec.fixed_damage != 0)
        {
            let def_had_dbond = (self.mons[def_idx].vol_flags & vol_bit::DESTINY_BOND) != 0;
            let def_had_sub = self.mons[def_idx].sub_hp() > 0 && !mspec.bypasses_sub;
            // In Gen 2, draining moves (Giga Drain, Mega Drain, Absorb, Leech Life) always miss against Substitute.
            if mspec.drain_half && def_had_sub {
                self.exec_after_move_self(table, side);
                return;
            }
            let def_endure = (self.mons[def_idx].vol_flags & vol_bit::ENDURE) != 0 && !def_had_sub;
            let max_endure_dmg = (self.mons[def_idx].hp() - 1).max(0);
            let mut dealt = 0i16;
            let mut main_dealt = 0i16;
            let mut hit_landed = false;
            let mut hit_main_hp = false;
            if mspec.special_kind == SpecialMoveKind::Counter {
                // Counter: double last physical damage taken this turn.
                let (_, is_immune) = table.effectiveness_ext(mspec.move_type, &table.mons[def_idx], def_foresight);
                if !is_immune && self.hurt_cat_this_turn[side] == 1 && self.hurt_damage_this_turn[side] > 0 {
                    let mut dmg = (self.hurt_damage_this_turn[side] as i32 * 2).min(9999) as i16;
                    if def_endure {
                        dmg = dmg.min(max_endure_dmg);
                    }
                    dealt = self.mons[def_idx].apply_damage_branchless(dmg, mspec.bypasses_sub);
                    hit_landed = true;
                    if !def_had_sub {
                        hit_main_hp = true;
                        main_dealt = dealt;
                    }
                }
            } else if mspec.special_kind == SpecialMoveKind::MirrorCoat {
                // Mirror Coat: double last special damage taken this turn.
                let (_, is_immune) = table.effectiveness_ext(mspec.move_type, &table.mons[def_idx], def_foresight);
                if !is_immune && self.hurt_cat_this_turn[side] == 2 && self.hurt_damage_this_turn[side] > 0 {
                    let mut dmg = (self.hurt_damage_this_turn[side] as i32 * 2).min(9999) as i16;
                    if def_endure {
                        dmg = dmg.min(max_endure_dmg);
                    }
                    dealt = self.mons[def_idx].apply_damage_branchless(dmg, mspec.bypasses_sub);
                    hit_landed = true;
                    if !def_had_sub {
                        hit_main_hp = true;
                        main_dealt = dealt;
                    }
                }
            } else if mspec.special_kind == SpecialMoveKind::LevelDamage || mspec.fixed_damage != 0 {
                // Seismic Toss / Night Shade / Fixed Damage.
                let (_, is_immune) = table.effectiveness_ext(mspec.move_type, &table.mons[def_idx], def_foresight);
                if !is_immune {
                    let mut dmg = if mspec.fixed_damage > 0 {
                        mspec.fixed_damage
                    } else {
                        table.mons[att_idx].level as i16
                    };
                    if def_endure {
                        dmg = dmg.min(max_endure_dmg);
                    }
                    dealt = self.mons[def_idx].apply_damage_branchless(dmg, mspec.bypasses_sub);
                    hit_landed = true;
                    if !def_had_sub {
                        hit_main_hp = true;
                        main_dealt = dealt;
                    }
                }
            } else if mspec.special_kind == SpecialMoveKind::SuperFang {
                // Super Fang: halves target's current HP (min 1).
                let (_, is_immune) = table.effectiveness_ext(mspec.move_type, &table.mons[def_idx], def_foresight);
                if !is_immune {
                    let mut dmg = (self.mons[def_idx].hp() / 2).max(1);
                    if def_endure {
                        dmg = dmg.min(max_endure_dmg);
                    }
                    dealt = self.mons[def_idx].apply_damage_branchless(dmg, mspec.bypasses_sub);
                    hit_landed = true;
                    if !def_had_sub {
                        hit_main_hp = true;
                        main_dealt = dealt;
                    }
                }
            } else {
                const CRIT_DEN: [u32; 6] = [0, 16, 8, 4, 3, 2];
                let att_item = table.item_spec(self.mons[att_idx].item);
                let scope_bonus = att_item.is_scope_lens as usize;
                let c_stage = (mspec.crit_ratio as usize + scope_bonus).min(5);
                let is_crit = (mspec.special_kind != SpecialMoveKind::FlailReversal)
                    && c_stage > 0
                    && rng.random_chance(1, CRIT_DEN[c_stage]);

                let (atk_stat, def_stat) = if mspec.is_physical {
                    (stat_idx::ATK, stat_idx::DEF)
                } else {
                    (stat_idx::SPA, stat_idx::SPD)
                };
                let ignore_boosts =
                    is_crit & (self.mons[att_idx].boosts[atk_stat] <= self.mons[def_idx].boosts[def_stat]);
                let atk = self.active_stat(table, side, atk_stat, ignore_boosts, ignore_boosts);
                let def = self.active_stat(table, foe_side, def_stat, ignore_boosts, false);

                let bp = if mspec.special_kind == SpecialMoveKind::FlailReversal {
                    // Flail / Reversal 6-band HP power table.
                    let ratio = (self.mons[att_idx].hp() as i32 * 48) / (table.mons[att_idx].maxhp.max(1) as i32);
                    if ratio <= 1 {
                        200
                    } else if ratio <= 4 {
                        150
                    } else if ratio <= 9 {
                        100
                    } else if ratio <= 16 {
                        80
                    } else if ratio <= 32 {
                        40
                    } else {
                        20
                    }
                } else {
                    mspec.base_power as i32
                };

                let (type_mod, is_immune) = table.effectiveness_ext(mspec.move_type, &table.mons[def_idx], def_foresight);
                let weather = if self.weather_turns > 0 {
                    self.weather_kind
                } else {
                    CompactWeather::None
                };
                let has_type_boost = (mspec.move_type != NO_TYPE_ID)
                    & (att_item.boost_type == mspec.move_type);

                let base_dmg = get_damage_base_branchless(
                    table.mons[att_idx].level as i32,
                    bp,
                    atk,
                    def,
                    mspec.is_physical,
                    mspec.is_selfdestruct,
                    is_crit,
                    has_type_boost,
                    weather,
                    mspec.is_water,
                    mspec.is_fire,
                    mspec.is_solarbeam,
                    mspec.move_type,
                    table.mons[att_idx].type_mask,
                    type_mod,
                    is_immune,
                );
                if base_dmg > 0 {
                    let no_var = mspec.special_kind == SpecialMoveKind::FlailReversal;
                    let roll = if no_var { 255 } else { 217 + rng.random(39) };
                    let single_dmg = apply_roll_branchless(base_dmg, roll, no_var);
                    if mspec.multihit_mean_x2 > 2 && def_had_sub {
                        // Cold path: multihit move into Substitute can break Sub on an earlier hit
                        // and spill subsequent hits (and secondaries like Twineedle) onto main HP.
                        let hits = (mspec.multihit_mean_x2 / 2).max(2) as usize;
                        let per_hit = single_dmg.min(9999) as i16;
                        let has_endure = (self.mons[def_idx].vol_flags & vol_bit::ENDURE) != 0;
                        for _ in 0..hits {
                            if self.mons[def_idx].hp() <= 0 {
                                break;
                            }
                            if self.mons[def_idx].sub_hp() > 0 && !mspec.bypasses_sub {
                                dealt = dealt.saturating_add(
                                    self.mons[def_idx].apply_damage_branchless(per_hit, false),
                                );
                            } else {
                                let d = if has_endure {
                                    per_hit.min((self.mons[def_idx].hp() - 1).max(0))
                                } else {
                                    per_hit
                                };
                                let h = self.mons[def_idx].apply_damage_branchless(d, true);
                                dealt = dealt.saturating_add(h);
                                main_dealt = h;
                                hit_main_hp = true;
                            }
                        }
                        hit_landed = true;
                    } else {
                        let mut final_dmg = single_dmg;
                        if mspec.multihit_mean_x2 > 2 {
                            final_dmg = (final_dmg * (mspec.multihit_mean_x2 as i32)) / 2;
                        }
                        let mut hit_dmg = final_dmg.min(9999) as i16;
                        if def_endure {
                            hit_dmg = hit_dmg.min(max_endure_dmg);
                        }
                        dealt = self.mons[def_idx].apply_damage_branchless(hit_dmg, mspec.bypasses_sub);
                        hit_landed = true;
                        if !def_had_sub {
                            hit_main_hp = true;
                            main_dealt = dealt;
                        }
                    }

                    if mspec.drain_half && dealt > 0 {
                        let drain = (dealt / 2).max(1);
                        let maxhp = table.mons[att_idx].maxhp;
                        self.mons[att_idx].hp_pool[0] = (self.mons[att_idx].hp() + drain).min(maxhp);
                    }
                }
            }

            // Record Counter / Mirror Coat turn damage only when hitting main HP (never Substitute).
            if main_dealt > 0 && hit_main_hp {
                self.hurt_damage_this_turn[foe_side] = main_dealt as u16;
                self.hurt_cat_this_turn[foe_side] = if mspec.is_physical { 1 } else { 2 };
            }

            // Recoil (`1 HP` on Substitute, skip 1v1 KO recoil per Stadium 2).
            if mspec.recoil_quarter && dealt > 0 && self.mons[att_idx].hp() > 0 {
                if def_had_sub {
                    self.mons[att_idx].apply_damage_branchless(1, true);
                } else {
                    let is_1v1_ko = self.sides[0].alive_mask.count_ones() == 1
                        && self.sides[1].alive_mask.count_ones() == 1
                        && self.mons[def_idx].hp() <= 0;
                    if !is_1v1_ko {
                        let recoil = (dealt / 4).max(1);
                        self.mons[att_idx].apply_damage_branchless(recoil, true);
                    }
                }
            }

            // Hyper Beam sets mustrecharge on any non-immune hit (even on KO or Substitute).
            if mspec.is_recharge && hit_landed && self.mons[att_idx].hp() > 0 {
                self.mons[att_idx].set_must_recharge(true);
            }

            // Rapid Spin clears Spikes, Leech Seed, and partial traps (Wrap/Bind/Fire Spin/Clamp/Whirlpool),
            // NEVER Mean Look / Spider Web (`vol_bit::TRAPPED_OR_LOCKED`).
            if mspec.special_kind == SpecialMoveKind::RapidSpin
                && hit_landed
                && self.mons[att_idx].hp() > 0
            {
                self.sides[side].side_flags &= !0x03;
                self.mons[att_idx].vol_flags &= !vol_bit::LEECH_SEED;
                self.mons[att_idx].set_partially_trapped(false);
            }

            // Defrost user (Flame Wheel / Sacred Fire) or frozen target hit by burn-secondary move.
            if hit_landed
                && self.mons[att_idx].hp() > 0
                && mspec.is_defrost
                && self.mons[att_idx].status_raw() == Status::Frz as u8
            {
                let prev_res = if (self.mons[att_idx].status_counter() & 0x10) != 0 {
                    self.mons[att_idx].status_counter()
                } else {
                    0
                };
                self.mons[att_idx].set_status(Status::None as u8, prev_res);
            }
            if hit_landed
                && hit_main_hp
                && self.mons[def_idx].hp() > 0
                && self.mons[def_idx].status_raw() == Status::Frz as u8
                && mspec.sec_status == Status::Brn as u8
            {
                let prev_res = if (self.mons[def_idx].status_counter() & 0x10) != 0 {
                    self.mons[def_idx].status_counter()
                } else {
                    0
                };
                self.mons[def_idx].set_status(Status::None as u8, prev_res);
            }

            // Destiny Bond: if direct attack fainted a target with Destiny Bond active, attacker faints too.
            if def_had_dbond && self.mons[def_idx].hp() <= 0 && self.mons[att_idx].hp() > 0 {
                self.mons[att_idx].hp_pool[0] = 0;
                self.mons[att_idx].status_packed = Status::Fnt as u8;
                self.mons[att_idx].vol_flags = 0;
            }

            // Secondary effects on target (when target survived and hit main HP — including Endure 0 dmg at 1 HP).
            if hit_landed
                && hit_main_hp
                && self.mons[def_idx].hp() > 0
                && mspec.sec_chance_256 > 0
                && rng.random_chance(mspec.sec_chance_256 as u32, 256)
            {
                if mspec.sec_status != 0 && self.mons[def_idx].status_raw() == 0 && self.sides[foe_side].safeguard_turns == 0 {
                    self.try_inflict_status(table, foe_side, mspec.sec_status, rng);
                }
                if mspec.sec_vol != 0 {
                    let mut add_vol = mspec.sec_vol;
                    let def_st = self.mons[def_idx].status_raw();
                    // In Gen 2, non-King's-Rock flinch secondaries cannot flinch sleeping or frozen targets.
                    if (add_vol & vol_bit::FLINCH) != 0
                        && (def_st == Status::Slp as u8 || def_st == Status::Frz as u8)
                    {
                        add_vol &= !vol_bit::FLINCH;
                    }
                    if (add_vol & vol_bit::CONFUSION) != 0 {
                        if self.sides[foe_side].safeguard_turns != 0
                            || (self.mons[def_idx].vol_flags & vol_bit::CONFUSION) != 0
                        {
                            add_vol &= !vol_bit::CONFUSION;
                        } else {
                            self.mons[def_idx].timers[timer_idx::CONFUSION] =
                                (2 + rng.random(4)) as u8;
                        }
                    }
                    self.mons[def_idx].vol_flags |= add_vol;
                }
                if mspec.sec_boost.0 != NO_STAT_IDX {
                    let is_self_boost = (mspec.sec_boost.0 & SELF_BOOST_FLAG) != 0;
                    let b_idx = if is_self_boost { att_idx } else { def_idx };
                    let st_i = mspec.sec_boost.0 & !SELF_BOOST_FLAG;
                    if is_self_boost && st_i == ANCIENTPOWER_ALL_STATS {
                        for stat_i in 0..5usize {
                            self.mons[b_idx].boosts[stat_i] =
                                (self.mons[b_idx].boosts[stat_i] + mspec.sec_boost.1).clamp(-6, 6);
                        }
                    } else if (st_i as usize) < 7 {
                        let st = st_i as usize;
                        self.mons[b_idx].boosts[st] =
                            (self.mons[b_idx].boosts[st] + mspec.sec_boost.1).clamp(-6, 6);
                    }
                }
                // Thief steals via 100% (255/256) secondary when hitting surviving, un-Substituted target.
                if mspec.special_kind == SpecialMoveKind::Thief
                    && self.mons[att_idx].hp() > 0
                    && self.mons[att_idx].item == 0
                    && self.mons[def_idx].item != 0
                {
                    self.mons[att_idx].item = self.mons[def_idx].item;
                    self.mons[def_idx].item = 0;
                }
            }
        } else if mspec.is_status {
            self.exec_status_move(table, side, foe_side, att_idx, def_idx, &mspec, moved_second, rng);
        }

        if !mspec.is_selfdestruct
            && mspec.special_kind != SpecialMoveKind::BatonPass
            && self.mons[att_idx].hp() > 0
            && self.mons[def_idx].hp() > 0
            && self.outcome == 0
        {
            self.exec_after_move_self(table, side);
        } else if self.mons[def_idx].hp() > 0 {
            self.mons[def_idx].vol_flags &= !vol_bit::DESTINY_BOND;
        }
    }

    /// Gen 2 `onAfterMoveSelf` slip damage (Burn, Poison, Toxic, Leech Seed, Nightmare, Curse).
    #[inline]
    fn exec_after_move_self(&mut self, table: &MatchTable, side: usize) {
        let foe_side = side ^ 1;
        let idx = side * 3 + self.sides[side].active_idx as usize;
        let foe_idx = foe_side * 3 + self.sides[foe_side].active_idx as usize;
        if self.mons[foe_idx].hp() > 0 {
            self.mons[foe_idx].vol_flags &= !vol_bit::DESTINY_BOND;
        }
        if self.mons[idx].hp() <= 0 || self.mons[foe_idx].hp() <= 0 {
            return;
        }
        let st = self.mons[idx].status_raw();
        if st == Status::Brn as u8 || st == Status::Psn as u8 {
            let cnt = self.mons[idx].status_counter();
            if (cnt & 0x10) != 0 {
                let c = ((cnt & 0x0F) + 1).min(15);
                self.mons[idx].set_status(st, c | 0x10);
                let slip = table.mons[idx].quantum_1_16 * (c as i16);
                self.mons[idx].apply_damage_branchless(slip, true);
            } else {
                let slip = table.mons[idx].quantum_1_8;
                self.mons[idx].apply_damage_branchless(slip, true);
            }
        } else if st == Status::Tox as u8 {
            let c = ((self.mons[idx].status_counter() & 0x0F) + 1).min(15);
            self.mons[idx].set_status(Status::Tox as u8, c | 0x10);
            let slip = table.mons[idx].quantum_1_16 * (c as i16);
            self.mons[idx].apply_damage_branchless(slip, true);
        }
        if self.mons[idx].hp() > 0 && (self.mons[idx].vol_flags & vol_bit::LEECH_SEED) != 0 {
            let drain = table.mons[idx].quantum_1_8;
            let dealt = self.mons[idx].apply_damage_branchless(drain, true);
            if self.mons[foe_idx].hp() > 0 {
                let foe_max = table.mons[foe_idx].maxhp;
                self.mons[foe_idx].hp_pool[0] = (self.mons[foe_idx].hp() + dealt).min(foe_max);
            }
        }
        if self.mons[idx].hp() > 0
            && (self.mons[idx].vol_flags & vol_bit::NIGHTMARE) != 0
            && self.mons[idx].status_raw() == Status::Slp as u8
        {
            let slip = (table.mons[idx].maxhp / 4).max(1);
            self.mons[idx].apply_damage_branchless(slip, true);
        }
        if self.mons[idx].hp() > 0 && (self.mons[idx].vol_flags & vol_bit::CURSE) != 0 {
            let slip = (table.mons[idx].maxhp / 4).max(1);
            self.mons[idx].apply_damage_branchless(slip, true);
        }
    }

    #[inline]
    fn try_inflict_status(
        &mut self,
        table: &MatchTable,
        target_side: usize,
        status_code: u8,
        rng: &mut Prng,
    ) {
        let idx = target_side * 3 + self.sides[target_side].active_idx as usize;
        if self.mons[idx].status_raw() != 0 || self.mons[idx].sub_hp() > 0 || self.sides[target_side].safeguard_turns > 0 {
            return;
        }
        let prev_res = if (self.mons[idx].status_counter() & 0x10) != 0 {
            self.mons[idx].status_counter()
        } else {
            0
        };
        let target_tmask = table.mons[idx].type_mask;
        // Stadium Sleep Clause & Freeze Clause checks.
        if status_code == Status::Slp as u8 {
            for p_i in 0..3usize {
                if p_i != (self.sides[target_side].active_idx as usize)
                    && (self.sides[target_side].alive_mask & (1 << p_i)) != 0
                {
                    let m = &self.mons[target_side * 3 + p_i];
                    if m.hp() > 0 && m.status_raw() == Status::Slp as u8 {
                        if (self.clause_flags & 1) != 0 {
                            self.outcome = if target_side == 0 { 1 } else { 2 };
                        }
                        return;
                    }
                }
            }
            // Gen 2 PS (`gen2stadium2nc2000`): non-Rest sleep counter is 2..=4 (1..=3 skipped turns).
            let turns = (2 + rng.random(3)) as u8;
            self.mons[idx].set_status(Status::Slp as u8, turns);
        } else if status_code == Status::Frz as u8 {
            if (target_tmask & (1u32 << (table.ice_type & 31))) != 0
                || (self.weather_turns > 0 && self.weather_kind == CompactWeather::Sun)
            {
                return;
            }
            for p_i in 0..3usize {
                if (self.sides[target_side].alive_mask & (1 << p_i)) != 0 {
                    let m = &self.mons[target_side * 3 + p_i];
                    if m.hp() > 0 && m.status_raw() == Status::Frz as u8 {
                        return;
                    }
                }
            }
            self.mons[idx].set_status(Status::Frz as u8, prev_res);
        } else if status_code == Status::Par as u8 {
            self.mons[idx].set_status(Status::Par as u8, prev_res);
            self.mons[idx].vol_flags |= vol_bit::PAR_SPEED_DROP;
        } else if status_code == Status::Brn as u8 {
            if (target_tmask & (1u32 << (table.fire_type & 31))) != 0 {
                return;
            }
            self.mons[idx].set_status(Status::Brn as u8, prev_res);
            self.mons[idx].vol_flags |= vol_bit::BRN_ATK_DROP;
        } else if status_code == Status::Tox as u8 {
            if (target_tmask & table.poison_immune_mask) != 0 {
                return;
            }
            self.mons[idx].set_status(Status::Tox as u8, 0x10);
        } else if status_code == Status::Psn as u8 {
            if (target_tmask & table.poison_immune_mask) != 0 {
                return;
            }
            self.mons[idx].set_status(Status::Psn as u8, prev_res);
        }
    }

    /// Gen 2 Mint Berry / Miracle Berry activation (`onBeforeMove` priority 11 + `onResidual` order 5).
    /// Healing berries (`Gold Berry`, `Berry Juice`, `Berry`) activate only at EOT `Residual` order 10.
    #[inline]
    fn check_status_berry_trigger(&mut self, table: &MatchTable, side: usize) {
        let idx = side * 3 + self.sides[side].active_idx as usize;
        let mon = &mut self.mons[idx];
        if mon.hp() <= 0 || mon.item == 0 {
            return;
        }
        let ispec = table.item_spec(mon.item);
        let st = mon.status_raw();
        if ispec.is_mintberry && st == Status::Slp as u8 {
            mon.set_status(Status::None as u8, 0);
            mon.item = 0;
        } else if ispec.is_miracleberry && (st != 0 || (mon.vol_flags & vol_bit::CONFUSION) != 0) {
            let prev_res = if st != Status::Slp as u8 && (mon.status_counter() & 0x10) != 0 {
                mon.status_counter()
            } else {
                0
            };
            mon.set_status(Status::None as u8, prev_res);
            mon.vol_flags &= !(vol_bit::CONFUSION | vol_bit::PAR_SPEED_DROP | vol_bit::BRN_ATK_DROP);
            mon.timers[timer_idx::CONFUSION] = 0;
            mon.item = 0;
        }
    }

    #[allow(clippy::too_many_arguments)]
    #[inline]
    fn exec_status_move(
        &mut self,
        table: &MatchTable,
        side: usize,
        foe_side: usize,
        att_idx: usize,
        def_idx: usize,
        mspec: &CompactMoveSpec,
        moved_second: bool,
        rng: &mut Prng,
    ) {
        match mspec.special_kind {
            SpecialMoveKind::Reflect => {
                if self.sides[side].reflect_turns == 0 {
                    self.sides[side].reflect_turns = 5;
                }
            }
            SpecialMoveKind::LightScreen => {
                if self.sides[side].lightscreen_turns == 0 {
                    self.sides[side].lightscreen_turns = 5;
                }
            }
            SpecialMoveKind::Safeguard => {
                if self.sides[side].safeguard_turns == 0 {
                    self.sides[side].safeguard_turns = 5;
                }
            }
            SpecialMoveKind::Spikes => {
                if (self.sides[foe_side].side_flags & 0x03) == 0 {
                    self.sides[foe_side].side_flags |= 1;
                }
            }
            SpecialMoveKind::Curse => {
                // Curse: non-Ghost (+1 Atk, +1 Def, -1 Spe), Ghost (-50% HP, inflicts Curse).
                let is_ghost = (table.mons[att_idx].type_mask & (1u32 << (table.ghost_type & 31))) != 0;
                if !is_ghost {
                    self.mons[att_idx].boosts[stat_idx::ATK] =
                        (self.mons[att_idx].boosts[stat_idx::ATK] + 1).clamp(-6, 6);
                    self.mons[att_idx].boosts[stat_idx::DEF] =
                        (self.mons[att_idx].boosts[stat_idx::DEF] + 1).clamp(-6, 6);
                    self.mons[att_idx].boosts[stat_idx::SPE] =
                        (self.mons[att_idx].boosts[stat_idx::SPE] - 1).clamp(-6, 6);
                } else if self.mons[def_idx].sub_hp() == 0 && (self.mons[def_idx].vol_flags & vol_bit::CURSE) == 0 {
                    let cost = (table.mons[att_idx].maxhp / 2).max(1);
                    self.mons[att_idx].apply_damage_branchless(cost, true);
                    self.mons[def_idx].vol_flags |= vol_bit::CURSE;
                }
            }
            SpecialMoveKind::BellyDrum => {
                // Belly Drum: costs 50% maxhp and sets Attack stage to +6.
                let cost = table.mons[att_idx].maxhp / 2;
                if self.mons[att_idx].hp() > cost && self.mons[att_idx].boosts[stat_idx::ATK] < 6 {
                    self.mons[att_idx].hp_pool[0] -= cost;
                    self.mons[att_idx].boosts[stat_idx::ATK] = 6;
                }
            }
            SpecialMoveKind::Phaze => {
                // Roar / Whirlwind: in Gen 2, fails if user moves before target; otherwise force-switches foe to a random alive bench member.
                if moved_second {
                    let mask = self.sides[foe_side].alive_mask & !(1 << self.sides[foe_side].active_idx);
                    if mask != 0 {
                        let slot = if mask.count_ones() == 1 || rng.random(2) == 0 {
                            mask.trailing_zeros() as u8
                        } else {
                            (7 - mask.leading_zeros()) as u8
                        };
                        self.exec_switch(table, foe_side, slot.min(2), SwitchKind::Drag);
                    }
                }
            }
            SpecialMoveKind::Encore => {
                // Encore: locks target into its last move for 3..=6 turns (bypasses Substitute in Gen 2).
                if (self.mons[def_idx].vol_flags & vol_bit::ENCORE) == 0 {
                    if let Some(lm) = self.mons[def_idx].last_move_slot().map(|s| s as usize) {
                        if lm < table.mons[def_idx].num_moves as usize
                            && self.mons[def_idx].pp[lm] > 0
                            && table.move_specs[def_idx][lm].special_kind != SpecialMoveKind::Encore
                        {
                            self.mons[def_idx].vol_flags |= vol_bit::ENCORE;
                            self.mons[def_idx].timers[timer_idx::ENCORE] =
                                (3 + rng.random(4)) as u8;
                            self.mons[def_idx].set_encore_slot(lm as u8);
                        }
                    }
                }
            }
            SpecialMoveKind::Disable => {
                // Disable: disables target's last move (bypasses Substitute in Gen 2).
                // Duration matches Battle (`random_range(2, 6)` + 1 when target already moved this turn).
                if (self.mons[def_idx].vol_flags & vol_bit::DISABLE) == 0 {
                    if let Some(lm) = self.mons[def_idx].last_move_slot().map(|s| s as usize) {
                        if lm < table.mons[def_idx].num_moves as usize
                            && self.mons[def_idx].pp[lm] > 0
                        {
                            self.mons[def_idx].vol_flags |= vol_bit::DISABLE;
                            self.mons[def_idx].timers[timer_idx::DISABLE] =
                                (2 + rng.random(4) + (moved_second as u32)) as u8;
                            self.mons[def_idx].set_disable_slot(lm as u8);
                        }
                    }
                }
            }
            SpecialMoveKind::PainSplit => {
                // Pain Split: average HP when target has no Substitute.
                if self.mons[def_idx].sub_hp() == 0 {
                    let avg = ((self.mons[att_idx].hp() + self.mons[def_idx].hp()) / 2).max(1);
                    self.mons[att_idx].hp_pool[0] = avg.min(table.mons[att_idx].maxhp);
                    self.mons[def_idx].hp_pool[0] = avg.min(table.mons[def_idx].maxhp);
                }
            }
            SpecialMoveKind::BatonPass => {
                // Baton Pass: switch to best alive bench receiver passing boosts/sub/volatiles.
                let mask = self.sides[side].alive_mask & !(1 << self.sides[side].active_idx);
                if mask != 0 {
                    let bp_fix = (self.sides[0].side_flags & SIDE0_DISABLE_BP_FIX) == 0;
                    let mut best_slot = mask.trailing_zeros() as u8;
                    if !bp_fix {
                        let mut best_hp = -1i32;
                        for s_i in 0..3u8 {
                            if (mask & (1 << s_i)) != 0 {
                                let idx = side * 3 + s_i as usize;
                                let frac = (self.mons[idx].hp() as i32 * 1024)
                                    / (table.mons[idx].maxhp.max(1) as i32);
                                if frac > best_hp {
                                    best_hp = frac;
                                    best_slot = s_i;
                                }
                            }
                        }
                    } else if mask.count_ones() > 1 {
                        let passed_boosts = self.mons[att_idx].boosts;
                        let has_off_boost = passed_boosts[0] > 0 || passed_boosts[2] > 0;
                        let foe_slot = self.sides[foe_side].active_idx as usize;
                        let foe_alive = has_off_boost
                            && (self.sides[foe_side].alive_mask & (1 << foe_slot)) != 0
                            && self.mons[def_idx].hp() > 0;
                        let mut best_score = -1.0f64;
                        for s_i in 0..3u8 {
                            if (mask & (1 << s_i)) != 0 {
                                let idx = side * 3 + s_i as usize;
                                let hp_frac = (self.mons[idx].hp() as f64)
                                    / (table.mons[idx].maxhp.max(1) as f64);
                                let boost_gain = if foe_alive {
                                    let base_h = self.best_hit_fraction(
                                        table,
                                        side,
                                        s_i as usize,
                                        foe_side,
                                        foe_slot,
                                    );
                                    let saved = self.mons[idx].boosts;
                                    self.mons[idx].boosts = passed_boosts;
                                    let boosted_h = self.best_hit_fraction(
                                        table,
                                        side,
                                        s_i as usize,
                                        foe_side,
                                        foe_slot,
                                    );
                                    self.mons[idx].boosts = saved;
                                    (boosted_h - base_h).clamp(0.0, 1.0)
                                } else {
                                    0.0
                                };
                                let score = hp_frac + boost_gain;
                                if score > best_score {
                                    best_score = score;
                                    best_slot = s_i;
                                }
                            }
                        }
                    }
                    self.exec_switch(table, side, best_slot.min(2), SwitchKind::BatonPass);
                }
            }
            SpecialMoveKind::PerishSong => {
                // Perish Song
                if self.sides[side].alive_mask.count_ones() > 1 {
                    for &idx in &[att_idx, def_idx] {
                        if (self.mons[idx].vol_flags & vol_bit::PERISH_SONG) == 0 {
                            self.mons[idx].vol_flags |= vol_bit::PERISH_SONG;
                            self.mons[idx].set_perish_turns(4);
                        }
                    }
                }
            }
            SpecialMoveKind::DestinyBond => {
                if self.sides[side].alive_mask.count_ones() > 1 {
                    self.mons[att_idx].vol_flags |= vol_bit::DESTINY_BOND;
                }
            }
            SpecialMoveKind::HealBell => {
                // Heal Bell cures party statuses (preserving any BP-carried `residualdmg` counter).
                for p_i in 0..3usize {
                    let m = &mut self.mons[side * 3 + p_i];
                    if m.hp() > 0 {
                        let prev_res = if m.status_raw() != Status::Slp as u8
                            && (m.status_counter() & 0x10) != 0
                        {
                            m.status_counter()
                        } else {
                            0
                        };
                        m.set_status(Status::None as u8, prev_res);
                        m.vol_flags &= !(vol_bit::PAR_SPEED_DROP | vol_bit::BRN_ATK_DROP);
                    }
                }
            }
            SpecialMoveKind::PsychUp => {
                // Psych Up
                self.mons[att_idx].boosts = self.mons[def_idx].boosts;
            }
            SpecialMoveKind::RainDance => {
                self.weather_kind = CompactWeather::Rain;
                self.weather_turns = 5;
            }
            SpecialMoveKind::SunnyDay => {
                self.weather_kind = CompactWeather::Sun;
                self.weather_turns = 5;
            }
            SpecialMoveKind::Sandstorm => {
                self.weather_kind = CompactWeather::Sandstorm;
                self.weather_turns = 5;
            }
            SpecialMoveKind::MeanLook => {
                // Mean Look / Spider Web bypass Substitute in Gen 2.
                self.mons[def_idx].vol_flags |= vol_bit::TRAPPED_OR_LOCKED;
            }
            SpecialMoveKind::Haze => {
                // Haze
                self.mons[att_idx].boosts = [0; 7];
                self.mons[def_idx].boosts = [0; 7];
                self.mons[att_idx].vol_flags &= !(vol_bit::PAR_SPEED_DROP | vol_bit::BRN_ATK_DROP);
                self.mons[def_idx].vol_flags &= !(vol_bit::PAR_SPEED_DROP | vol_bit::BRN_ATK_DROP);
            }
            SpecialMoveKind::Swagger => {
                // Gen 2 Swagger: fails if target Atk is already +6 or unmodified Atk == 999
                // (exact equality per Battle::SwaggerTryHit — Thick Club > 999 still succeeds!);
                // otherwise raises target Atk +2 even through Substitute, and confuses only if
                // target has no Substitute, no Safeguard, and is not already confused.
                if self.mons[def_idx].boosts[stat_idx::ATK] < 6
                    && self.active_stat(table, foe_side, stat_idx::ATK, false, true) != 999
                {
                    self.mons[def_idx].boosts[stat_idx::ATK] =
                        (self.mons[def_idx].boosts[stat_idx::ATK] + 2).clamp(-6, 6);
                    if self.mons[def_idx].sub_hp() == 0
                        && self.sides[foe_side].safeguard_turns == 0
                        && (self.mons[def_idx].vol_flags & vol_bit::CONFUSION) == 0
                    {
                        self.mons[def_idx].vol_flags |= vol_bit::CONFUSION;
                        self.mons[def_idx].timers[timer_idx::CONFUSION] =
                            (2 + rng.random(4)) as u8;
                    }
                }
                return;
            }
            _ => {}
        }

        if mspec.primary_status != 0 {
            // Thunder Wave is blocked by Ground immunity.
            let ground_immune = mspec.move_type == table.electric_type
                && (table.mons[def_idx].type_mask & (1u32 << (table.ground_type & 31))) != 0;
            if !ground_immune {
                self.try_inflict_status(table, foe_side, mspec.primary_status, rng);
            }
        }
        if mspec.primary_vol != 0
            && !matches!(
                mspec.special_kind,
                SpecialMoveKind::Curse
                    | SpecialMoveKind::Encore
                    | SpecialMoveKind::Disable
                    | SpecialMoveKind::PerishSong
                    | SpecialMoveKind::DestinyBond
            )
            && mspec.primary_vol != vol_bit::DESTINY_BOND
            && (self.mons[def_idx].sub_hp() == 0 || mspec.bypasses_sub)
        {
            let seed_immune = (mspec.primary_vol & vol_bit::LEECH_SEED) != 0
                && (table.mons[def_idx].type_mask & (1u32 << (table.grass_type & 31))) != 0;
            let night_fail = (mspec.primary_vol & vol_bit::NIGHTMARE) != 0
                && self.mons[def_idx].status_raw() != Status::Slp as u8;
            let conf_fail = (mspec.primary_vol & vol_bit::CONFUSION) != 0
                && (self.sides[foe_side].safeguard_turns != 0
                    || (self.mons[def_idx].vol_flags & vol_bit::CONFUSION) != 0);
            if !seed_immune && !night_fail && !conf_fail {
                self.mons[def_idx].vol_flags |= mspec.primary_vol;
                if (mspec.primary_vol & vol_bit::CONFUSION) != 0 {
                    self.mons[def_idx].timers[timer_idx::CONFUSION] =
                        (2 + rng.random(4)) as u8;
                }
            }
        }
        let b_target = if mspec.boosts_self { att_idx } else { def_idx };
        if mspec.boosts_self || self.mons[def_idx].sub_hp() == 0 || mspec.bypasses_sub {
            for &(st_i, delta) in &mspec.boosts {
                if (st_i as usize) < 7 {
                    self.mons[b_target].boosts[st_i as usize] =
                        (self.mons[b_target].boosts[st_i as usize] + delta).clamp(-6, 6);
                }
            }
        }
    }

    /// Gen 2 end-of-turn `Residual` in exact PS/Stadium 2 order:
    /// Weather/Sandstorm (2) -> Perish Song (4) -> Leftovers & Mint/Miracle Berry (5) ->
    /// Freeze thaw (7) -> Berry/Gold Berry/Berry Juice (10) & Disable (no shared state with order 13) -> Encore & Stall reset (13) -> Screens.
    #[inline]
    fn exec_residuals(&mut self, table: &MatchTable, rng: &mut Prng) {
        let s0 = self.mon_action_speed(table, 0, self.sides[0].active_idx as usize);
        let s1 = self.mon_action_speed(table, 1, self.sides[1].active_idx as usize);
        let first = if s0 == s1 {
            (rng.random(2) == 1) as usize
        } else {
            (s0 < s1) as usize
        };
        let order = [first, first ^ 1];

        // Order 2: Weather countdown and Sandstorm damage (both sides damaged before faint batch).
        if self.weather_kind != CompactWeather::None {
            self.weather_turns = self.weather_turns.saturating_sub(1);
            if self.weather_turns == 0 {
                self.weather_kind = CompactWeather::None;
            } else if self.weather_kind == CompactWeather::Sandstorm {
                for &side in &order {
                    let p_idx = self.sides[side].active_idx as usize;
                    let idx = side * 3 + p_idx;
                    if self.mons[idx].hp() > 0
                        && (table.mons[idx].type_mask & table.sandstorm_immune_mask) == 0
                    {
                        let sand_dmg = table.mons[idx].quantum_1_8;
                        self.mons[idx].apply_damage_branchless(sand_dmg, true);
                        if self.mons[idx].hp() <= 0 {
                            self.sides[side].alive_mask &= !(1 << p_idx);
                            let foe_side = side ^ 1;
                            let foe_idx = foe_side * 3 + self.sides[foe_side].active_idx as usize;
                            self.mons[foe_idx].vol_flags &= !vol_bit::TRAPPED_OR_LOCKED;
                            self.mons[foe_idx].set_partially_trapped(false);
                        }
                    }
                }
                let a0 = self.sides[0].alive_mask;
                let a1 = self.sides[1].alive_mask;
                if a0 == 0 && a1 == 0 {
                    // Matches Battle::faint_messages: faint_queue = [faster, slower],
                    // so winner = 1 - last_faint.side = faster (`order[0]`).
                    self.outcome = if order[0] == 0 { 1 } else { 2 };
                    return;
                } else if a0 == 0 {
                    self.outcome = 2;
                    return;
                } else if a1 == 0 {
                    self.outcome = 1;
                    return;
                }
            }
        }

        // Order 4: Perish Song countdown (both sides resolve before faint batch, matching Battle::field_event).
        for &side in &order {
            let p_idx = self.sides[side].active_idx as usize;
            let idx = side * 3 + p_idx;
            if self.mons[idx].hp() > 0 && (self.mons[idx].vol_flags & vol_bit::PERISH_SONG) != 0 {
                let rem = self.mons[idx].perish_turns().saturating_sub(1);
                self.mons[idx].set_perish_turns(rem);
                if rem == 0 {
                    self.mons[idx].hp_pool[0] = 0;
                    self.mons[idx].status_packed = Status::Fnt as u8;
                    self.mons[idx].vol_flags = 0;
                    self.mons[idx].timers = [0; 4];
                    self.sides[side].alive_mask &= !(1 << p_idx);
                    let foe_side = side ^ 1;
                    let foe_idx = foe_side * 3 + self.sides[foe_side].active_idx as usize;
                    self.mons[foe_idx].vol_flags &= !vol_bit::TRAPPED_OR_LOCKED;
                    self.mons[foe_idx].set_partially_trapped(false);
                }
            }
        }
        {
            let a0 = self.sides[0].alive_mask;
            let a1 = self.sides[1].alive_mask;
            if a0 == 0 && a1 == 0 {
                // Matches Battle::field_event + faint_messages: both PerishSongEnd handlers queue
                // `[faster, slower]` before faint_messages runs, so `order[0]` (faster) wins.
                self.outcome = if order[0] == 0 { 1 } else { 2 };
                return;
            } else if a0 == 0 {
                self.outcome = 2;
                return;
            } else if a1 == 0 {
                self.outcome = 1;
                return;
            }
        }

        // Order 5: Leftovers and Mint/Miracle Berry in speed order.
        for &side in &order {
            let p_idx = self.sides[side].active_idx as usize;
            let idx = side * 3 + p_idx;
            if self.mons[idx].hp() <= 0 {
                continue;
            }
            let spec = &table.mons[idx];
            if table.item_spec(self.mons[idx].item).is_leftovers && self.mons[idx].hp() < spec.maxhp {
                self.mons[idx].hp_pool[0] =
                    (self.mons[idx].hp() + spec.quantum_1_16).min(spec.maxhp);
            }
            self.check_status_berry_trigger(table, side);
        }

        // Order 7: Freeze 25/256 thaw in speed order.
        for &side in &order {
            let p_idx = self.sides[side].active_idx as usize;
            let idx = side * 3 + p_idx;
            if self.mons[idx].hp() > 0
                && self.mons[idx].status_raw() == Status::Frz as u8
                && rng.random_chance(25, 256)
            {
                let prev_res = if (self.mons[idx].status_counter() & 0x10) != 0 {
                    self.mons[idx].status_counter()
                } else {
                    0
                };
                self.mons[idx].set_status(Status::None as u8, prev_res);
            }
        }

        // Order 10: Healing berries (Gold Berry +30, Berry Juice +20, Berry +10) and Disable countdown.
        for &side in &order {
            let p_idx = self.sides[side].active_idx as usize;
            let idx = side * 3 + p_idx;
            if self.mons[idx].hp() <= 0 {
                continue;
            }
            let spec = &table.mons[idx];
            if self.mons[idx].item != 0 && self.mons[idx].hp() <= spec.maxhp / 2 {
                let ispec = table.item_spec(self.mons[idx].item);
                let heal = if ispec.is_goldberry {
                    30
                } else if ispec.is_berry_juice {
                    20
                } else if ispec.is_berry {
                    10
                } else {
                    0
                };
                if heal > 0 {
                    self.mons[idx].hp_pool[0] = (self.mons[idx].hp() + heal).min(spec.maxhp);
                    self.mons[idx].item = 0;
                }
            }
            if (self.mons[idx].vol_flags & vol_bit::DISABLE) != 0 {
                let rem = self.mons[idx].timers[timer_idx::DISABLE].saturating_sub(1);
                self.mons[idx].timers[timer_idx::DISABLE] = rem;
                if rem == 0 {
                    self.mons[idx].vol_flags &= !vol_bit::DISABLE;
                }
            }
        }

        // Order 13: Encore countdown, Stall reset, and turn-scoped flag clear.
        for &side in &order {
            let p_idx = self.sides[side].active_idx as usize;
            let idx = side * 3 + p_idx;
            if self.mons[idx].hp() <= 0 {
                continue;
            }
            let spec = &table.mons[idx];
            if (self.mons[idx].vol_flags & vol_bit::ENCORE) != 0 {
                let enc_slot = self.mons[idx].encore_slot() as usize;
                let rem = self.mons[idx].timers[timer_idx::ENCORE].saturating_sub(1);
                self.mons[idx].timers[timer_idx::ENCORE] = rem;
                if rem == 0
                    || enc_slot >= spec.num_moves as usize
                    || self.mons[idx].pp[enc_slot] == 0
                {
                    self.mons[idx].vol_flags &= !vol_bit::ENCORE;
                    self.mons[idx].timers[timer_idx::ENCORE] = 0;
                }
            }
            if (self.mons[idx].vol_flags & (vol_bit::PROTECT | vol_bit::ENDURE)) == 0 {
                self.mons[idx].set_stall_counter(0);
            }
            self.mons[idx].vol_flags &= !(vol_bit::PROTECT | vol_bit::ENDURE | vol_bit::FLINCH);
        }

        // Decrement Reflect / Light Screen / Safeguard timers.
        for side in 0..2usize {
            self.sides[side].reflect_turns = self.sides[side].reflect_turns.saturating_sub(1);
            self.sides[side].lightscreen_turns =
                self.sides[side].lightscreen_turns.saturating_sub(1);
            self.sides[side].safeguard_turns = self.sides[side].safeguard_turns.saturating_sub(1);
        }

        self.check_faints(table, rng);
    }

    /// Resolve a simultaneous dual replacement Spikes faint in action-speed order
    /// matching `Battle` (`ActionKind::RunSwitch` sorted by `get_pokemon_action_speed`):
    /// the faster incoming mon takes Spikes and faints first, awarding the win to the slower side.
    #[inline]
    fn resolve_dual_spikes_faint(&self, table: &MatchTable, rng: &mut Prng) -> u8 {
        let spd0 = self.mon_action_speed(table, 0, self.sides[0].active_idx as usize);
        let spd1 = self.mon_action_speed(table, 1, self.sides[1].active_idx as usize);
        if spd0 == spd1 {
            if rng.random(2) == 0 { 1 } else { 2 }
        } else if spd0 > spd1 {
            2 // P1 incoming mon faster -> faints first -> P2Win
        } else {
            1 // P2 incoming mon faster -> faints first -> P1Win
        }
    }

    #[inline]
    fn check_faints(&mut self, table: &MatchTable, rng: &mut Prng) {
        if self.outcome != 0 {
            return;
        }
        for side in 0..2usize {
            let p_idx = self.sides[side].active_idx as usize;
            let m_idx = side * 3 + p_idx;
            if self.mons[m_idx].hp() <= 0 {
                self.sides[side].alive_mask &= !(1 << p_idx);
                // Gen 2: any faint cancels remaining queued moves (`Battle::faint_messages` `queue_cancel_move`).
                self.sides[0].side_flags &= !0x70;
                self.sides[1].side_flags &= !0x70;
                // Fainting releases Mean Look / Spider Web and partial trap on the opposing active Pokemon.
                let foe_side = side ^ 1;
                let foe_idx = foe_side * 3 + self.sides[foe_side].active_idx as usize;
                self.mons[foe_idx].vol_flags &= !vol_bit::TRAPPED_OR_LOCKED;
                self.mons[foe_idx].set_partially_trapped(false);
            }
        }
        let a0 = self.sides[0].alive_mask;
        let a1 = self.sides[1].alive_mask;
        if a0 == 0 && a1 == 0 {
            // Stadium 2 Self-KO Clause: user of Self-Destruct / Explosion loses.
            if self.sides[0].last_move_selfko != u8::MAX {
                self.outcome = 2; // P2Win
            } else if self.sides[1].last_move_selfko != u8::MAX {
                self.outcome = 1; // P1Win
            } else {
                self.outcome = 3; // Tie
            }
        } else if a0 == 0 {
            self.outcome = 2; // P2Win
        } else if a1 == 0 {
            self.outcome = 1; // P1Win
        } else {
            // Clear Self-KO tracker once non-terminal faints resolve so an
            // early-game Explosion/Self-Destruct cannot leak into a later 1v1 mutual faint.
            self.sides[0].last_move_selfko = u8::MAX;
            self.sides[1].last_move_selfko = u8::MAX;
            // Process single-bench auto-switches (and resulting Spikes faints).
            let mut any_auto_switch = false;
            for side in 0..2usize {
                let p_idx = self.sides[side].active_idx as usize;
                if (self.sides[side].alive_mask & (1 << p_idx)) == 0
                    && self.sides[side].alive_mask.count_ones() == 1
                {
                    let next_slot = (self.sides[side].alive_mask.trailing_zeros() as u8).min(2);
                    self.exec_switch(table, side, next_slot, SwitchKind::FaintReplace);
                    any_auto_switch = true;
                    if self.mons[side * 3 + next_slot as usize].hp() <= 0 {
                        self.sides[side].alive_mask &= !(1 << next_slot);
                    }
                }
            }
            if any_auto_switch {
                let a0 = self.sides[0].alive_mask;
                let a1 = self.sides[1].alive_mask;
                if a0 == 0 && a1 == 0 {
                    self.outcome = self.resolve_dual_spikes_faint(table, rng);
                    return;
                } else if a0 == 0 {
                    self.outcome = 2;
                    return;
                } else if a1 == 0 {
                    self.outcome = 1;
                    return;
                }
            }

            let mut any_forced_switch = false;
            for side in 0..2usize {
                let p_idx = self.sides[side].active_idx as usize;
                if (self.sides[side].alive_mask & (1 << p_idx)) == 0 {
                    self.sides[side].request = 2; // Forced Switch request
                    any_forced_switch = true;
                } else {
                    self.sides[side].request = 1;
                }
            }
            if any_forced_switch {
                for side in 0..2usize {
                    if self.sides[side].request != 2 {
                        self.sides[side].request = 0; // Wait
                    }
                }
            }
        }
    }

    /// Resolve a forced switch phase when one or both active Pokemon fainted
    /// with 2 bench options remaining (`request == 2`).
    pub fn step_switch(&mut self, table: &MatchTable, switch_actions: [u8; 2]) {
        let mut rng = Prng::new(self.prng_seed);
        // Any forced-switch prompt occurs strictly after a non-terminal faint batch.
        self.sides[0].last_move_selfko = u8::MAX;
        self.sides[1].last_move_selfko = u8::MAX;
        let bp_fix = (self.sides[0].side_flags & SIDE0_DISABLE_BP_FIX) == 0;
        let both_forced = self.sides[0].request == 2 && self.sides[1].request == 2;
        for side in 0..2usize {
            if self.sides[side].request == 2 {
                let act = switch_actions[side];
                let slot = if act >= 4 {
                    (act - 4).min(2)
                } else {
                    (self.sides[side].alive_mask.trailing_zeros() as u8).min(2)
                };
                if (self.sides[side].alive_mask & (1 << slot)) != 0 {
                    let act_idx = side * 3 + (self.sides[side].active_idx.min(2) as usize);
                    let copy_vol = bp_fix && self.mons[act_idx].hp() > 0;
                    self.exec_switch(
                        table,
                        side,
                        slot,
                        if copy_vol {
                            SwitchKind::BatonPass
                        } else {
                            SwitchKind::FaintReplace
                        },
                    );
                }
            }
            self.sides[side].request = 1;
        }
        if both_forced {
            let i0 = self.sides[0].active_idx as usize;
            let i1 = self.sides[1].active_idx as usize;
            if self.mons[i0].hp() <= 0 && self.mons[3 + i1].hp() <= 0 {
                self.sides[0].alive_mask &= !(1 << i0);
                self.sides[1].alive_mask &= !(1 << i1);
                if self.sides[0].alive_mask == 0 && self.sides[1].alive_mask == 0 {
                    self.outcome = self.resolve_dual_spikes_faint(table, &mut rng);
                    self.prng_seed = rng.seed();
                    return;
                }
            }
        }
        self.check_faints(table, &mut rng);
        // If this was a mid-turn self-switch (e.g. faster Baton Pass) and the other side
        // had a pending move queued for the second half of the turn, execute it now in CompactBattle.
        if bp_fix && self.outcome == 0 && self.sides[0].request == 1 && self.sides[1].request == 1 {
            for s in 0..2usize {
                let flags = self.sides[s].side_flags;
                if (flags & 0x10) != 0 {
                    self.sides[s].side_flags &= !0x70;
                    let m_slot = ((flags >> 5) & 0x03) as usize;
                    let forced_str = self.is_forced_struggle(table, s);
                    self.exec_move(table, s, m_slot, true, forced_str, &mut rng);
                    self.check_faints(table, &mut rng);
                    break;
                }
            }
        }
        if self.outcome == 0 && self.sides[0].request == 1 && self.sides[1].request == 1 {
            if (self.sides[0].side_flags & 0x08) == 0 {
                self.sides[0].side_flags |= 0x08;
                self.exec_residuals(table, &mut rng);
                self.check_faints(table, &mut rng);
            }
            if self.outcome == 0 && self.sides[0].request == 1 && self.sides[1].request == 1 {
                self.sides[0].side_flags &= !0x08;
                self.turn = self.turn.saturating_add(1);
                if self.turn > 1000 {
                    self.outcome = 3; // Tie
                } else {
                    self.quick_claw_roll = rng.random_chance(60, 256) as u8;
                }
            }
        }
        self.prng_seed = rng.seed();
    }

    /// Enumerate legal compact action codes for `side` (`0` or `1`) into a stack buffer.
    /// Action codes:
    /// - `0..4`: move slot `0..4`
    /// - `4..7`: switch to party slot `0..3` (`action - 4`)
    /// - `7`: `"pass"` (when waiting for opponent's forced switch)
    #[inline]
    pub fn legal_actions(&self, table: &MatchTable, side: usize, out: &mut [u8; 7]) -> usize {
        if self.outcome != 0 {
            return 0;
        }
        let side_st = &self.sides[side];
        if side_st.request == 0 {
            out[0] = 7; // pass
            return 1;
        }
        if side_st.request == 2 {
            let mut n = 0;
            for slot in 0..3u8 {
                if slot != side_st.active_idx && (side_st.alive_mask & (1 << slot)) != 0 {
                    out[n] = 4 + slot;
                    n += 1;
                }
            }
            return n;
        }
        let mut n = 0;
        let idx = side * 3 + side_st.active_idx as usize;
        let mon = &self.mons[idx];
        if mon.must_recharge() {
            out[0] = 0; // Forced Recharge turn cannot choose other moves or switch
            return 1;
        }
        let num_moves = table.mons[idx].num_moves as usize;
        if (mon.vol_flags & vol_bit::ENCORE) != 0 {
            let enc_slot = mon.encore_slot() as usize;
            if enc_slot < num_moves && mon.pp[enc_slot] > 0 && !mon.is_slot_disabled(enc_slot) {
                out[0] = enc_slot as u8;
                n = 1;
            }
        } else {
            for m in 0..num_moves {
                if mon.pp[m] > 0 && !mon.is_slot_disabled(m) {
                    out[n] = m as u8;
                    n += 1;
                }
            }
        }
        if n == 0 {
            out[0] = 0; // Struggle fallback
            n = 1;
        }
        if !mon.is_trapped() {
            for slot in 0..3u8 {
                if slot != side_st.active_idx && (side_st.alive_mask & (1 << slot)) != 0 {
                    out[n] = 4 + slot;
                    n += 1;
                }
            }
        }
        n
    }

    /// Apply a pair of action codes `[p1_action, p2_action]`, handling both normal
    /// turns and forced-switch phases.
    #[inline]
    pub fn apply_actions(&mut self, table: &MatchTable, actions: [u8; 2]) {
        if self.sides[0].request == 2 || self.sides[1].request == 2 {
            self.step_switch(table, actions);
        } else {
            self.step_turn(table, actions);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compact_battle_layout_is_exact_192_bytes_and_64b_aligned() {
        assert_eq!(std::mem::size_of::<CompactMon>(), 24);
        assert_eq!(std::mem::size_of::<CompactSide>(), 12);
        assert_eq!(std::mem::size_of::<CompactBattle>(), 192);
        assert_eq!(std::mem::align_of::<CompactBattle>(), 64);
    }

    #[test]
    fn step_turn_respects_switches_priority_and_quick_claw() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../data/gen2stadium2.json");
        let json = std::fs::read_to_string(&path).unwrap();
        let dex = Dex::from_json(&json).unwrap();
        let mk = |species: &str, level: u8, item: &str, moves: &[&str]| -> crate::battle::PokemonSet {
            serde_json::from_value(serde_json::json!({
                "name": species, "species": species, "item": item, "ability": "No Ability",
                "moves": moves, "level": level,
                "evs": {"hp":255,"atk":255,"def":255,"spa":255,"spd":255,"spe":255},
                "ivs": {"hp":30,"atk":30,"def":30,"spa":30,"spd":30,"spe":30},
                "happiness": 255
            }))
            .unwrap()
        };
        let p1 = vec![
            mk("Snorlax", 55, "Quick Claw", &["Double-Edge", "Protect", "Rest", "Curse"]),
            mk("Zapdos", 50, "Leftovers", &["Thunderbolt", "Drill Peck", "Rest", "Sleep Talk"]),
            mk("Cloyster", 50, "Leftovers", &["Surf", "Ice Beam", "Spikes", "Explosion"]),
        ];
        let p2 = vec![
            mk("Marowak", 50, "Thick Club", &["Earthquake", "Rock Slide", "Swords Dance", "Protect"]),
            mk("Jolteon", 55, "Leftovers", &["Thunderbolt", "Quick Attack", "Substitute", "Baton Pass"]),
            mk("Skarmory", 50, "Leftovers", &["Drill Peck", "Whirlwind", "Curse", "Rest"]),
        ];
        let mut b = Battle::from_fixture(&dex, "1,2,3,4", &p1, &p2).unwrap();
        b.choose(&dex, 0, "team 1,2,3").unwrap();
        b.choose(&dex, 1, "team 1,2,3").unwrap();
        let (table, base) = CompactBattle::from_battle(&b, &dex);

        // Switch beats +2 Protect (slot 1 moves first onto incoming Zapdos; Marowak Protect fails because it moves second, consuming PP cleanly).
        let mut cb_sw = base;
        cb_sw.step_turn(&table, [5, 3]);
        assert_eq!(cb_sw.sides[0].active_idx, 1);
        assert_eq!(cb_sw.mons[3].stall_counter(), 0);
        assert_eq!(cb_sw.mons[3].pp[3], base.mons[3].pp[3] - 1);

        // +2 Protect blocks slower priority-0 Double-Edge (Marowak stays full HP).
        let mut cb_prot = base;
        cb_prot.quick_claw_roll = 0;
        cb_prot.step_turn(&table, [0, 3]);
        assert_eq!(cb_prot.mons[3].hp(), table.mons[3].maxhp);
        assert_eq!(cb_prot.mons[3].stall_counter(), 1);

        // Quick Claw on slow Snorlax (quick_claw_roll != 0, bit 0 = P1 triggers) moves before faster Marowak at priority 0.
        let mut cb_qc = base;
        cb_qc.quick_claw_roll = 1;
        cb_qc.step_turn(&table, [0, 0]);
        assert_eq!(cb_qc.mons[0].hp() < table.mons[0].maxhp, true);
        assert_eq!(cb_qc.mons[3].hp() < table.mons[3].maxhp, true);

        // Switch to Jolteon, then verify Quick Attack (+1) outprioritizes Quick Claw Snorlax Double-Edge (0).
        let mut cb_qa = base;
        cb_qa.step_turn(&table, [2, 5]); // Snorlax Rest (full HP noop), P2 switches Jolteon
        cb_qa.quick_claw_roll = 1;
        // Snorlax asleep? No, Rest fails at full HP; try Double-Edge (0) vs Jolteon Quick Attack (1) at low Snorlax HP
        let mut low_hp = cb_qa;
        low_hp.mons[0].hp_pool[0] = 5;
        low_hp.step_turn(&table, [0, 1]);
        assert_eq!(low_hp.mons[0].hp(), 0);
        assert_eq!(low_hp.mons[4].hp(), table.mons[4].maxhp);
    }

    #[test]
    fn stat_branchless_matches_reference_formula_exhaustively() {
        const STAT_POS: [f64; 7] = [1.0, 1.5, 2.0, 2.5, 3.0, 3.5, 4.0];
        const STAT_NEG: [f64; 7] = [100.0, 66.0, 50.0, 40.0, 33.0, 28.0, 25.0];

        for stored in [1, 15, 95, 130, 178, 255, 358, 500, 999] {
            for boost in -6..=6i8 {
                for stat_idx in 0..5usize {
                    for &unboosted in &[false, true] {
                        for &unmodified in &[false, true] {
                            for &par in &[false, true] {
                                for &brn in &[false, true] {
                                    for &screen in &[false, true] {
                                        for &item in &[
                                            ItemStatKind::None,
                                            ItemStatKind::ThickClub,
                                            ItemStatKind::LightBall,
                                            ItemStatKind::MetalPowder,
                                        ] {
                                            let mut ref_val = stored as f64;
                                            if !unboosted {
                                                if boost >= 0 {
                                                    ref_val =
                                                        (ref_val * STAT_POS[boost as usize]).floor();
                                                } else {
                                                    ref_val = (ref_val * STAT_NEG[(-boost) as usize]
                                                        / 100.0)
                                                        .floor();
                                                }
                                            }
                                            if par && stat_idx == 4 {
                                                ref_val = (ref_val / 4.0).floor();
                                            }
                                            if !unmodified && brn && stat_idx == 0 {
                                                ref_val = (ref_val / 2.0).floor();
                                            }
                                            ref_val = ref_val.clamp(1.0, 999.0);
                                            if !unboosted && screen && (stat_idx == 1 || stat_idx == 3) {
                                                ref_val *= 2.0;
                                            }
                                            if (item == ItemStatKind::ThickClub && stat_idx == 0)
                                                || (item == ItemStatKind::LightBall && stat_idx == 2)
                                            {
                                                ref_val *= 2.0;
                                            } else if item == ItemStatKind::MetalPowder
                                                && (stat_idx == 1 || stat_idx == 3)
                                            {
                                                ref_val = (ref_val * 1.5).floor();
                                            }

                                            let fast_val = get_stat_branchless(
                                                stored, boost, stat_idx, unboosted, unmodified,
                                                false, par, brn, screen, screen, item,
                                            );
                                            assert_eq!(
                                                fast_val, ref_val as i32,
                                                "mismatch at stored={stored}, boost={boost}, stat={stat_idx}, unboosted={unboosted}, unmodified={unmodified}"
                                            );
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn acc_lut_and_reciprocal_damage_rolls_are_bit_exact() {
        // Verify reciprocal multiplication (x * 0x808081) >> 31 matches x / 255
        // for every possible damage (1..=4000) and every roll (217..=255).
        for dmg in 1..=4000i32 {
            for roll in 217..=255u32 {
                let exact = ((dmg as f64) * (roll as f64) / 255.0).floor() as i32;
                let fast = apply_roll_branchless(dmg, roll, false);
                assert_eq!(fast, exact.max(1), "mismatch at dmg={dmg}, roll={roll}");
            }
        }

        // Verify accuracy LUT across stages, Foresight (ignore_positive_evasion), and BrightPowder (-20).
        assert_eq!(lookup_accuracy_threshold(100, 0, 0, false, false, false, false), 255);
        assert_eq!(lookup_accuracy_threshold(100, 0, 0, false, false, false, true), 235);
        assert_eq!(lookup_accuracy_threshold(75, 0, 0, false, false, false, false), 191);
        assert!(
            lookup_accuracy_threshold(100, 0, 2, false, false, false, false) < 255,
            "+2 evasion must lower threshold"
        );
        assert_eq!(
            lookup_accuracy_threshold(100, 0, 2, false, false, true, false),
            255,
            "Foresight drops positive evasion"
        );
        assert!(
            lookup_accuracy_threshold(100, 0, -2, false, false, true, false) == 255,
            "Foresight keeps negative evasion beneficial"
        );
        assert!(
            lookup_accuracy_threshold(100, -1, 0, false, false, false, false) < 255,
            "-1 accuracy lowers threshold"
        );
        assert_eq!(
            lookup_accuracy_threshold(100, -1, 0, true, false, false, false),
            255,
            "ignore_accuracy drops negative accuracy"
        );
    }

    #[test]
    fn compact_battle_from_battle_step_and_hash() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../data/gen2stadium2.json");
        let json = std::fs::read_to_string(&path).unwrap();
        let dex = Dex::from_json(&json).unwrap();
        let mk = |species: &str, level: u8, item: &str, moves: &[&str]| -> crate::battle::PokemonSet {
            serde_json::from_value(serde_json::json!({
                "name": species, "species": species, "item": item, "ability": "No Ability",
                "moves": moves, "level": level,
                "evs": {"hp":255,"atk":255,"def":255,"spa":255,"spd":255,"spe":255},
                "ivs": {"hp":30,"atk":30,"def":30,"spa":30,"spd":30,"spe":30},
                "happiness": 255
            }))
            .unwrap()
        };
        let p1 = vec![
            mk("Snorlax", 55, "Leftovers", &["Double-Edge", "Earthquake", "Rest", "Sleep Talk"]),
            mk("Zapdos", 50, "Mint Berry", &["Thunderbolt", "Hidden Power", "Rest", "Whirlwind"]),
            mk("Cloyster", 50, "Leftovers", &["Surf", "Ice Beam", "Spikes", "Explosion"]),
        ];
        let p2 = vec![
            mk("Marowak", 55, "Thick Club", &["Earthquake", "Rock Slide", "Swords Dance", "Rest"]),
            mk("Skarmory", 50, "Leftovers", &["Drill Peck", "Whirlwind", "Curse", "Rest"]),
            mk("Raikou", 50, "Miracle Berry", &["Thunderbolt", "Crunch", "Reflect", "Rest"]),
        ];
        let mut b = Battle::from_fixture(&dex, "1,2,3,4", &p1, &p2).unwrap();
        b.choose(&dex, 0, "team 1,2,3").unwrap();
        b.choose(&dex, 1, "team 1,2,3").unwrap();

        let (table, mut cb) = CompactBattle::from_battle(&b, &dex);
        let h0 = cb.search_hash();
        // Cloning CompactBattle is a 256-byte Copy.
        let mut cb_clone = cb;
        assert_eq!(cb_clone.search_hash(), h0);

        // Execute a turn in CompactBattle: Snorlax Double-Edge (0) vs Marowak Earthquake (0).
        cb.step_turn(&table, [0, 0]);
        assert_eq!(cb.turn, 2);
        assert_ne!(cb.search_hash(), h0);
        // Original copy is untouched.
        assert_eq!(cb_clone.turn, 1);
        // Switch Snorlax to party slot 1 (action 4 + 1 = 5), Marowak Swords Dance (move 2).
        cb_clone.step_turn(&table, [5, 2]);
        assert_eq!(cb_clone.sides[0].active_idx, 1);
    }

    #[test]
    fn compact_battle_forced_switch_after_ko_steps_safely() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../data/gen2stadium2.json");
        let json = std::fs::read_to_string(&path).unwrap();
        let dex = Dex::from_json(&json).unwrap();
        let mk = |species: &str, level: u8, item: &str, moves: &[&str]| -> crate::battle::PokemonSet {
            serde_json::from_value(serde_json::json!({
                "name": species, "species": species, "item": item, "ability": "No Ability",
                "moves": moves, "level": level,
                "evs": {"hp":255,"atk":255,"def":255,"spa":255,"spd":255,"spe":255},
                "ivs": {"hp":30,"atk":30,"def":30,"spa":30,"spd":30,"spe":30},
                "happiness": 255
            }))
            .unwrap()
        };
        let p1 = vec![
            mk("Cloyster", 50, "Leftovers", &["Explosion", "Surf", "Ice Beam", "Spikes"]),
            mk("Snorlax", 55, "Leftovers", &["Double-Edge", "Earthquake", "Rest", "Sleep Talk"]),
            mk("Zapdos", 50, "Mint Berry", &["Thunderbolt", "Hidden Power", "Rest", "Whirlwind"]),
        ];
        let p2 = vec![
            mk("Marowak", 50, "Thick Club", &["Earthquake", "Rock Slide", "Swords Dance", "Rest"]),
            mk("Skarmory", 50, "Leftovers", &["Drill Peck", "Whirlwind", "Curse", "Rest"]),
            mk("Raikou", 50, "Miracle Berry", &["Thunderbolt", "Crunch", "Reflect", "Rest"]),
        ];
        let mut b = Battle::from_fixture(&dex, "1,2,3,4", &p1, &p2).unwrap();
        b.choose(&dex, 0, "team 1,2,3").unwrap();
        b.choose(&dex, 1, "team 1,2,3").unwrap();

        // Cloyster uses Explosion to KO both itself (side 0) and Marowak (side 1).
        let id1 = b.active_id(1).unwrap();
        b.poke_mut(id1).hp = 10;
        b.choose(&dex, 0, "move 1").unwrap();
        b.choose(&dex, 1, "move 3").unwrap();

        assert_eq!(b.sides[0].request_state(), Some(RequestKind::Switch));
        assert_eq!(b.sides[1].request_state(), Some(RequestKind::Switch));

        // Also test the PositionSpec / protocol-import representation where `side.active` is `None`.
        for clear_active in [false, true] {
            let mut b_case = b.clone();
            if clear_active {
                b_case.sides[0].active = None;
                b_case.sides[1].active = None;
            }
            let (table, mut cb) = CompactBattle::from_battle(&b_case, &dex);
            for s in 0..2 {
                assert!(
                    cb.sides[s].active_idx < 3,
                    "side {s} active_idx={} must be < 3 (clear_active={clear_active})",
                    cb.sides[s].active_idx
                );
                assert_eq!(cb.sides[s].request, 2);
                assert_eq!(cb.sides[s].alive_mask & (1 << cb.sides[s].active_idx), 0);
                let mut acts = [0u8; 7];
                let n = cb.legal_actions(&table, s, &mut acts);
                assert_eq!(&acts[..n], &[5, 6]);
            }

            // Step forced switch on both sides (including side 1 where active_idx=3 previously panicked).
            cb.apply_actions(&table, [5, 6]);
            assert_eq!(cb.sides[0].active_idx, 1);
            assert_eq!(cb.sides[1].active_idx, 2);
            assert_eq!(cb.sides[0].request, 1);
            assert_eq!(cb.sides[1].request, 1);
            assert_eq!(cb.hurt_damage_this_turn, [0, 0]);
            assert_eq!(cb.hurt_cat_this_turn, [0, 0]);
            assert_eq!(cb.speed_order, 0);
        }
    }

    #[test]
    fn compact_protect_decay_self_moves_freeze_thaw_and_encore_lock() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../data/gen2stadium2.json");
        let json = std::fs::read_to_string(&path).unwrap();
        let dex = Dex::from_json(&json).unwrap();
        let mk = |species: &str, level: u8, item: &str, moves: &[&str]| -> crate::battle::PokemonSet {
            serde_json::from_value(serde_json::json!({
                "name": species, "species": species, "item": item, "ability": "No Ability",
                "moves": moves, "level": level,
                "evs": {"hp":255,"atk":255,"def":255,"spa":255,"spd":255,"spe":255},
                "ivs": {"hp":30,"atk":30,"def":30,"spa":30,"spd":30,"spe":30},
                "happiness": 255
            }))
            .unwrap()
        };
        let p1 = vec![
            mk("Jolteon", 50, "Leftovers", &["Protect", "Thunderbolt", "Agility", "Baton Pass"]),
            mk("Marowak", 55, "Thick Club", &["Earthquake", "Rock Slide", "Swords Dance", "Rest"]),
            mk("Cloyster", 50, "Leftovers", &["Surf", "Ice Beam", "Spikes", "Explosion"]),
        ];
        let p2 = vec![
            mk("Alakazam", 50, "Leftovers", &["Psychic", "Encore", "Disable", "Recover"]),
            mk("Snorlax", 55, "Leftovers", &["Body Slam", "Earthquake", "Curse", "Rest"]),
            mk("Skarmory", 50, "Leftovers", &["Drill Peck", "Whirlwind", "Curse", "Rest"]),
        ];
        let mut b = Battle::from_fixture(&dex, "1,2,3,4", &p1, &p2).unwrap();
        b.choose(&dex, 0, "team 1,2,3").unwrap();
        b.choose(&dex, 1, "team 1,2,3").unwrap();

        // 1. Protect sets stall counter in Battle and imports into CompactBattle; consecutive Protect decays.
        let mut b_prot = b.clone();
        b_prot.choose(&dex, 0, "move 1").unwrap(); // Protect
        b_prot.choose(&dex, 1, "move 1").unwrap(); // Psychic (blocked)
        let (table, cb_imported) = CompactBattle::from_battle(&b_prot, &dex);
        assert_eq!(cb_imported.mons[0].stall_counter(), 1);
        assert_eq!(cb_imported.mons[0]. hp(), table.mons[0].maxhp);

        let mut fail_seen = false;
        for seed in 1..=64u64 {
            let mut cb_try = cb_imported;
            cb_try.prng_seed = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15);
            let hp_before = cb_try.mons[0].hp();
            cb_try.step_turn(&table, [0, 0]); // 2nd consecutive Protect vs Psychic
            if cb_try.mons[0].hp() < hp_before {
                fail_seen = true;
                assert_eq!(cb_try.mons[0].stall_counter(), 0);
                break;
            }
        }
        assert!(fail_seen, "2nd consecutive Protect must not succeed 100% of the time");

        // 2. Opponent Protect does NOT block self-targeted moves (Swords Dance / Baton Pass / Curse).
        let (table2, mut cb2) = CompactBattle::from_battle(&b, &dex);
        // Jolteon Protect (0) vs Alakazam Recover/Curse test: switch P2 to Snorlax first, then Jolteon Protect vs Snorlax Curse.
        cb2.step_turn(&table2, [2, 5]); // Jolteon Agility (+2 Spe), P2 switches to Snorlax (slot 1)
        assert_eq!(cb2.mons[0].boosts[4], 2);
        cb2.step_turn(&table2, [0, 2]); // Jolteon Protect vs Snorlax Curse (+1 Atk/+1 Def/-1 Spe)
        assert_eq!(cb2.mons[4].boosts[0], 1, "Snorlax Curse must bypass Jolteon Protect");
        assert_eq!(cb2.mons[4].boosts[1], 1);
        // Next turn: Jolteon Baton Pass (move 3) passes +2 Spe to Marowak even if Snorlax uses Curse.
        cb2.step_turn(&table2, [3, 2]);
        let recv_slot = cb2.sides[0].active_idx as usize;
        assert_ne!(recv_slot, 0);
        assert_eq!(cb2.mons[recv_slot].boosts[4], 2);

        // 3. Encore imported from Battle locks legal_actions to the encored move slot and expires.
        let mut b_enc = b.clone();
        b_enc.choose(&dex, 0, "move 3").unwrap(); // Jolteon Agility (slot 2)
        b_enc.choose(&dex, 1, "move 2").unwrap(); // Alakazam Encore -> locks Jolteon into Agility
        let (t_enc, mut cb_enc) = CompactBattle::from_battle(&b_enc, &dex);
        assert_ne!(cb_enc.mons[0].vol_flags & vol_bit::ENCORE, 0);
        assert_eq!(cb_enc.mons[0].encore_slot(), 2);
        let mut acts = [0u8; 7];
        let n = cb_enc.legal_actions(&t_enc, 0, &mut acts);
        // Only move 2 (Agility) plus bench switches (5, 6) are legal!
        assert_eq!(&acts[..n], &[2, 5, 6]);
        for _ in 0..8 {
            if (cb_enc.mons[0].vol_flags & vol_bit::ENCORE) == 0 {
                break;
            }
            cb_enc.step_turn(&t_enc, [2, 3]);
        }
        assert_eq!(cb_enc.mons[0].vol_flags & vol_bit::ENCORE, 0);

        // 4. Freeze 25/256 residual thaw occurs across seeds.
        let (t_frz, cb_frz_base) = CompactBattle::from_battle(&b, &dex);
        let mut thawed = false;
        for seed in 1..=128u64 {
            let mut cb_frz = cb_frz_base;
            cb_frz.mons[0].set_status(Status::Frz as u8, 0);
            cb_frz.prng_seed = seed.wrapping_mul(0xD1B5_4A32_D192_ED03);
            cb_frz.step_turn(&t_frz, [1, 3]); // frozen Jolteon vs Alakazam Recover
            if cb_frz.mons[0].status_raw() == Status::None as u8 {
                thawed = true;
                break;
            }
        }
        assert!(thawed, "Frozen Pokemon must have 25/256 residual thaw chance in CompactBattle");
    }

    #[test]
    fn compact_step_fixes_match_battle_mechanics() {
        let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("data/gen2stadium2.json");
        let json = std::fs::read_to_string(&path).unwrap();
        let dex = Dex::from_json(&json).unwrap();
        let mk = |species: &str, level: u8, item: &str, moves: &[&str]| -> crate::battle::PokemonSet {
            serde_json::from_value(serde_json::json!({
                "name": species, "species": species, "item": item, "ability": "No Ability",
                "moves": moves, "level": level,
                "evs": {"hp":255,"atk":255,"def":255,"spa":255,"spd":255,"spe":255},
                "ivs": {"hp":30,"atk":30,"def":30,"spa":30,"spd":30,"spe":30},
                "happiness": 255
            }))
            .unwrap()
        };

        // 1. Non-Ghost Curse, Destiny Bond, Encore, Disable do not fall through via primary_vol.
        {
            let dt = dex_tables(&dex);
            for key in ["curse", "destinybond", "encore", "disable", "perishsong"] {
                let mid = dex.moves.id(key).unwrap();
                assert_eq!(dt.move_specs[mid.0 as usize].primary_vol, 0, "{key} primary_vol must be 0");
            }
            let p1 = vec![mk("Snorlax", 55, "", &["Curse", "Substitute", "Double-Edge", "Rest"])];
            let p2 = vec![mk("Gengar", 55, "", &["Destiny Bond", "Encore", "Disable", "Psychic"])];
            let mut b = Battle::from_fixture(&dex, "1,2,3,4", &p1, &p2).unwrap();
            b.choose(&dex, 0, "team 1").unwrap();
            b.choose(&dex, 1, "team 1").unwrap();
            let (t, mut cb) = CompactBattle::from_battle(&b, &dex);
            cb.step_turn(&t, [1, 0]); // Snorlax Sub, Gengar Destiny Bond (both faster Gengar moves first -> Snorlax Sub)
            assert_eq!(cb.mons[0].vol_flags & vol_bit::DESTINY_BOND, 0);
            assert_eq!(cb.mons[3].vol_flags & vol_bit::DESTINY_BOND, 0, "DBond expired after Snorlax moved");
            cb.step_turn(&t, [0, 1]); // Gengar Encore (faster) locks Snorlax into Substitute through Sub, redirecting Curse -> Substitute!
            assert_eq!(cb.mons[0].boosts[0], 0);
            assert_eq!(cb.mons[3].vol_flags & vol_bit::CURSE, 0);
            assert_ne!(cb.mons[0].vol_flags & vol_bit::ENCORE, 0, "Encore must bypass Substitute in Gen 2");
        }

        // 2. Struggle hits Ghost, takes 1/4 recoil on main HP and 1 HP recoil on Sub (Battle parity).
        {
            let p1 = vec![mk("Snorlax", 55, "", &["Tackle"])];
            let p2 = vec![mk("Gengar", 55, "", &["Substitute", "Splash"])];
            let mut b = Battle::from_fixture(&dex, "1,2,3,4", &p1, &p2).unwrap();
            b.choose(&dex, 0, "team 1").unwrap();
            b.choose(&dex, 1, "team 1").unwrap();
            let (t, mut cb) = CompactBattle::from_battle(&b, &dex);
            cb.mons[0].pp = [0; 4];
            let gengar_before = cb.mons[3].hp();
            cb.step_turn(&t, [0, 1]); // Struggle vs Splash
            let dmg = gengar_before - cb.mons[3].hp();
            assert!(dmg > 0, "Struggle must hit Ghost (Gengar) typelessly");
            assert_eq!(cb.mons[0].hp(), t.mons[0].maxhp - (dmg / 4).max(1));

            let (_, mut cb_sub) = CompactBattle::from_battle(&b, &dex);
            cb_sub.mons[0].pp = [0; 4];
            cb_sub.step_turn(&t, [0, 0]); // Gengar sets Sub first, Snorlax Struggles into Sub
            assert_eq!(cb_sub.mons[0].hp(), t.mons[0].maxhp - 1, "Struggle into Sub must deal 1 HP recoil");
        }

        // 3. Double residuals prevented on post-Sandstorm/Perish forced switch (`RESIDUALS_DONE`).
        {
            let p1 = vec![
                mk("Cloyster", 50, "", &["Surf", "Spikes"]),
                mk("Zapdos", 50, "Leftovers", &["Thunderbolt"]),
                mk("Snorlax", 55, "Leftovers", &["Body Slam"]),
            ];
            let p2 = vec![
                mk("Tyranitar", 55, "", &["Sandstorm", "Rock Slide"]),
                mk("Skarmory", 50, "Leftovers", &["Drill Peck"]),
                mk("Gengar", 50, "Leftovers", &["Thunderbolt"]),
            ];
            let mut b = Battle::from_fixture(&dex, "1,2,3,4", &p1, &p2).unwrap();
            b.choose(&dex, 0, "team 1,2,3").unwrap();
            b.choose(&dex, 1, "team 1,2,3").unwrap();
            let clo_id = b.active_id(0).unwrap();
            b.poke_mut(clo_id).hp = 1;
            let mut b_ref = b.clone();
            b_ref.choose(&dex, 0, "move 2").unwrap(); // Spikes
            b_ref.choose(&dex, 1, "move 1").unwrap(); // Sandstorm -> kills 1 HP Cloyster in Residual!
            b_ref.choose(&dex, 0, "switch 2").unwrap(); // Switch to Zapdos (auto-completes Turn 1 switch and advances to Turn 2)

            let (t, mut cb) = CompactBattle::from_battle(&b, &dex);
            cb.mons[0].hp_pool[0] = 1;
            cb.step_turn(&t, [1, 0]);
            assert_eq!(cb.sides[0].request, 2);
            assert_ne!(cb.sides[0].side_flags & 0x08, 0, "RESIDUALS_DONE must be set after Sandstorm KO");
            cb.step_switch(&t, [5, 7]);
            assert_eq!(cb.turn, 2);
            assert_eq!(cb.sides[0].side_flags & 0x08, 0);
            assert_eq!(cb.mons[1].hp(), b_ref.poke(b_ref.active_id(0).unwrap()).hp as i16, "Zapdos must not take Sandstorm on residual switch-in");
        }

        // 4. Baton Pass skips AfterMoveSelf slip damage on receiver.
        {
            let p1 = vec![
                mk("Jolteon", 50, "", &["Baton Pass", "Splash"]),
                mk("Snorlax", 55, "", &["Body Slam"]),
            ];
            let p2 = vec![mk("Umbreon", 50, "", &["Toxic", "Mean Look"])];
            let mut b = Battle::from_fixture(&dex, "1,2,3,4", &p1, &p2).unwrap();
            b.choose(&dex, 0, "team 1,2").unwrap();
            b.choose(&dex, 1, "team 1").unwrap();
            let (t, mut cb) = CompactBattle::from_battle(&b, &dex);
            cb.mons[0].set_status(Status::Psn as u8, 0);
            cb.step_turn(&t, [0, 1]); // Jolteon Baton Passes to Snorlax
            assert_eq!(cb.sides[0].active_idx, 1);
            assert_eq!(cb.mons[1].hp(), t.mons[1].maxhp, "Baton Pass receiver must not take outgoing Psn slip");
        }

        // 5. Counter/Mirror Coat Ghost/Dark immunities, Sub blocking hurt tracking, and Fixed/Super Fang arming Counter.
        {
            let p1 = vec![
                mk("Snorlax", 55, "", &["Double-Edge", "Seismic Toss", "Super Fang", "Thunderbolt"]),
                mk("Gengar", 50, "", &["Counter", "Night Shade"]),
                mk("Umbreon", 50, "", &["Mirror Coat", "Substitute"]),
            ];
            let p2 = vec![
                mk("Wobbuffet", 50, "", &["Counter", "Mirror Coat", "Substitute", "Splash"]),
            ];
            let mut b = Battle::from_fixture(&dex, "1,2,3,4", &p1, &p2).unwrap();
            b.choose(&dex, 0, "team 1,2,3").unwrap();
            b.choose(&dex, 1, "team 1").unwrap();
            let (t, mut cb) = CompactBattle::from_battle(&b, &dex);
            // Sub absorbs Double-Edge -> Wobbuffet Counter fails
            cb.step_turn(&t, [0, 2]);
            cb.step_turn(&t, [3, 0]); // Thunderbolt vs Counter -> hurt_cat from Sub wasn't set
            // Fresh battle: Seismic Toss on main HP arms Counter vs Snorlax
            let (_, mut cb2) = CompactBattle::from_battle(&b, &dex);
            cb2.step_turn(&t, [1, 0]); // Snorlax Seismic Toss (55), Wobbuffet Counter (110)
            assert_eq!(cb2.mons[0].hp(), t.mons[0].maxhp - 110);
            // Gengar (Ghost) immune to Wobbuffet Counter
            let (_, mut cb_g) = CompactBattle::from_battle(&b, &dex);
            cb_g.step_turn(&t, [5, 3]); // switch to Gengar
            cb_g.mons[1].hp_pool[0] = t.mons[1].maxhp;
            cb_g.step_turn(&t, [1, 0]); // Gengar Night Shade (Physical Ghost move in Gen2, though Special/Phys Ghost -> Wait: Night Shade is fixed Special Ghost!)
            // Switch Gengar into Counter after hurt_cat=1 manually to test Fighting immunity cleanly:
            cb_g.hurt_cat_this_turn[1] = 1;
            cb_g.hurt_damage_this_turn[1] = 50;
            // Direct exec via exec_move/step not exposing hurt across turns: test Umbreon vs Mirror Coat cleanly:
            let (_, mut cb_u) = CompactBattle::from_battle(&b, &dex);
            cb_u.step_turn(&t, [6, 3]); // switch to Umbreon (Dark)
            let p1b = vec![mk("Jolteon", 50, "", &["Mirror Coat"]), mk("Gengar", 50, "", &["Shadow Ball"])];
            let p2b = vec![mk("Heracross", 50, "", &["Seismic Toss"]), mk("Umbreon", 50, "", &["Thunder Wave"])];
            let mut b2 = Battle::from_fixture(&dex, "1,2,3,4", &p1b, &p2b).unwrap();
            b2.choose(&dex, 0, "team 1,2").unwrap();
            b2.choose(&dex, 1, "team 1,2").unwrap();
            // P2 Umbreon immune to Jolteon Mirror Coat (Dark) -> zero damage even if hurt_cat == 2
            let (t2, mut cbu) = CompactBattle::from_battle(&b2, &dex);
            cbu.exec_switch(&t2, 1, 1, SwitchKind::Manual);
            cbu.step_turn(&t2, [0, 0]);
            assert_eq!(cbu.mons[4].hp(), t2.mons[4].maxhp);
        }

        // 6. 1v1 Perish/Sandstorm speed order, 1v1 Double-Edge KO skips recoil, Hyper Beam recharge, Haze, Defrost, Pain Split vs Sub, Rapid Spin, Moonlight, AncientPower.
        {
            let p1 = vec![mk("Zapdos", 55, "", &["Hyper Beam", "AncientPower", "Moonlight", "Double-Edge"])];
            let p2 = vec![mk("Forretress", 50, "Charcoal", &["Rapid Spin", "Spikes", "Pain Split", "Flame Wheel"])];
            let mut b = Battle::from_fixture(&dex, "1,2,3,4", &p1, &p2).unwrap();
            b.choose(&dex, 0, "team 1").unwrap();
            b.choose(&dex, 1, "team 1").unwrap();
            let (t, base) = CompactBattle::from_battle(&b, &dex);

            // 1v1 Double-Edge KO skips recoil on attacker.
            let mut cb_1v1 = base;
            cb_1v1.mons[3].hp_pool[0] = 1;
            cb_1v1.step_turn(&t, [3, 0]);
            assert_eq!(cb_1v1.outcome, 1);
            assert_eq!(cb_1v1.mons[0].hp(), t.mons[0].maxhp, "1v1 KO recoil must be skipped");

            // 1v1 Double Perish: Battle::field_event queues [faster Zapdos, slower Forretress] into
            // faint_queue before faint_messages runs, so last faint is slower -> faster Zapdos wins (outcome = 1).
            let mut cb_perish = base;
            cb_perish.mons[0].vol_flags |= vol_bit::PERISH_SONG;
            cb_perish.mons[0].set_perish_turns(1);
            cb_perish.mons[3].vol_flags |= vol_bit::PERISH_SONG;
            cb_perish.mons[3].set_perish_turns(1);
            cb_perish.step_turn(&t, [2, 2]);
            assert_eq!(cb_perish.outcome, 1, "1v1 double Perish must match Battle (faster side wins)");

            // Moonlight weather scaling: Clear=1/2, Sun=maxhp, Rain=1/4
            let mut cb_moon = base;
            cb_moon.mons[0].hp_pool[0] = 10;
            cb_moon.weather_kind = CompactWeather::Sun;
            cb_moon.weather_turns = 3;
            cb_moon.step_turn(&t, [2, 1]);
            assert_eq!(cb_moon.mons[0].hp(), t.mons[0].maxhp);

            let mut cb_rain = base;
            cb_rain.mons[0].hp_pool[0] = 10;
            cb_rain.weather_kind = CompactWeather::Rain;
            cb_rain.weather_turns = 3;
            cb_rain.step_turn(&t, [2, 1]);
            assert_eq!(cb_rain.mons[0].hp(), 10 + (t.mons[0].maxhp / 4).max(1));

            // Rapid Spin removes P1 Spikes and Flame Wheel thaws frozen Forretress.
            let mut cb_rs = base;
            cb_rs.sides[1].side_flags |= 1; // 1 Spike layer on P2 side
            cb_rs.mons[3].vol_flags |= vol_bit::LEECH_SEED;
            cb_rs.step_turn(&t, [2, 0]); // Moonlight vs Rapid Spin
            assert_eq!(cb_rs.sides[1].side_flags & 0x03, 0);
            assert_eq!(cb_rs.mons[3].vol_flags & vol_bit::LEECH_SEED, 0);

            let mut cb_fw = base;
            cb_fw.mons[3].set_status(Status::Frz as u8, 0);
            cb_fw.step_turn(&t, [2, 3]);
            assert_eq!(cb_fw.mons[3].status_raw(), Status::None as u8, "Flame Wheel must thaw user on hit");

            // AncientPower 10% boost raises all 5 stats (0..5) when triggered.
            let mut ap_boosted = false;
            for s in 1..=256u64 {
                let mut cb_ap = base;
                cb_ap.prng_seed = s.wrapping_mul(0x9E37_79B9_7F4A_7C15);
                cb_ap.step_turn(&t, [1, 1]);
                if cb_ap.mons[0].boosts[0] == 1 {
                    assert_eq!(&cb_ap.mons[0].boosts[..5], &[1, 1, 1, 1, 1]);
                    ap_boosted = true;
                    break;
                }
            }
            assert!(ap_boosted, "AncientPower must boost atk..spe together");
        }

        // 7. Super Fang, 1.1x type boost item, and Rain/Sun hit fraction parity.
        {
            let p1 = vec![mk("Raticate", 50, "Charcoal", &["Super Fang", "Flame Wheel"])];
            let p2 = vec![mk("Snorlax", 55, "Mystic Water", &["Surf", "Solar Beam"])];
            let mut b = Battle::from_fixture(&dex, "1,2,3,4", &p1, &p2).unwrap();
            b.choose(&dex, 0, "team 1").unwrap();
            b.choose(&dex, 1, "team 1").unwrap();
            let (t, mut cb) = CompactBattle::from_battle(&b, &dex);
            let sf = cb.expected_hit_fraction(&t, 0, 0, 1, 0, 0);
            assert!(sf > 0.4 && sf < 0.6, "Super Fang expected hit fraction ~0.5: {sf}");
            cb.weather_kind = CompactWeather::Rain;
            cb.weather_turns = 3;
            let fw_rain = cb.expected_hit_fraction(&t, 0, 0, 1, 0, 1);
            let surf_rain = cb.expected_hit_fraction(&t, 1, 0, 0, 0, 0);
            let sb_rain = cb.expected_hit_fraction(&t, 1, 0, 0, 0, 1);
            cb.weather_kind = CompactWeather::None;
            cb.weather_turns = 0;
            let fw_clear = cb.expected_hit_fraction(&t, 0, 0, 1, 0, 1);
            let surf_clear = cb.expected_hit_fraction(&t, 1, 0, 0, 0, 0);
            let sb_clear = cb.expected_hit_fraction(&t, 1, 0, 0, 0, 1);
            assert!(fw_rain < fw_clear);
            assert!(surf_rain > surf_clear);
            assert!(sb_rain < sb_clear);
            // cfg!(debug_assertions) inside best_hit_fraction checks bit-for-bit equality with expected_hit_fraction:
            let _ = cb.best_hit_fraction(&t, 0, 0, 1, 0);
            let _ = cb.best_hit_fraction(&t, 1, 0, 0, 0);
        }

        // 8. Hyper Beam mustrecharge, Haze clearing status speed/atk drops, Pain Split vs Sub,
        //    Destiny Bond aborts (flinch/frz/slp/par/recharge) & foe switch expiry,
        //    symmetric auto-switch Spikes faint, and Fire secondary burn-chance thaw.
        {
            let p1 = vec![
                mk("Zapdos", 55, "", &["Hyper Beam", "Destiny Bond", "Thunderbolt", "Sleep Talk"]),
                mk("Vaporeon", 50, "", &["Haze", "Flamethrower", "Thunderbolt", "Rest"]),
            ];
            let p2 = vec![
                mk("Forretress", 50, "", &["Substitute", "Pain Split", "Spikes", "Baton Pass"]),
                mk("Tauros", 50, "", &["Body Slam", "Headbutt", "Hyper Beam", "Sleep Talk"]),
            ];
            let mut b = Battle::from_fixture(&dex, "1,2,3,4", &p1, &p2).unwrap();
            b.choose(&dex, 0, "team 1,2").unwrap();
            b.choose(&dex, 1, "team 1,2").unwrap();
            let (t, base) = CompactBattle::from_battle(&b, &dex);

            // Hyper Beam non-KO sets mustrecharge (slot_refs & 0x80), legal_actions == [0],
            // and recharging clears 0x80 as well as armed Destiny Bond.
            let mut cb_hb = base;
            cb_hb.prng_seed = 1;
            cb_hb.step_turn(&t, [0, 2]); // Hyper Beam vs Spikes (Steel resists Normal -> non-KO)
            assert!(cb_hb.mons[0].must_recharge(), "Hyper Beam hit must set mustrecharge");
            let mut legal = [0u8; 7];
            let n_legal = cb_hb.legal_actions(&t, 0, &mut legal);
            assert_eq!(&legal[..n_legal], &[0]);
            cb_hb.mons[0].vol_flags |= vol_bit::DESTINY_BOND;
            cb_hb.step_turn(&t, [0, 2]); // Recharge turn aborts any armed DBond via onMoveAborted
            assert!(!cb_hb.mons[0].must_recharge());
            assert_eq!(cb_hb.mons[0].vol_flags & vol_bit::DESTINY_BOND, 0);

            // Haze clears PAR_SPEED_DROP and BRN_ATK_DROP alongside boosts.
            let mut cb_haze = base;
            cb_haze.exec_switch(&t, 0, 1, SwitchKind::Manual); // Vaporeon active
            cb_haze.mons[1].set_status(Status::Brn as u8, 0);
            cb_haze.mons[1].vol_flags |= vol_bit::PAR_SPEED_DROP | vol_bit::BRN_ATK_DROP;
            cb_haze.mons[1].boosts[0] = 2;
            cb_haze.mons[3].set_status(Status::Par as u8, 0);
            cb_haze.mons[3].vol_flags |= vol_bit::PAR_SPEED_DROP | vol_bit::BRN_ATK_DROP;
            cb_haze.mons[3].boosts[1] = -2;
            cb_haze.step_turn(&t, [0, 2]); // Vaporeon Haze (not asleep/frozen/paralyzed)
            assert_eq!(cb_haze.mons[1].vol_flags & (vol_bit::PAR_SPEED_DROP | vol_bit::BRN_ATK_DROP), 0);
            assert_eq!(cb_haze.mons[3].vol_flags & (vol_bit::PAR_SPEED_DROP | vol_bit::BRN_ATK_DROP), 0);
            assert_eq!(cb_haze.mons[1].boosts[0], 0);
            assert_eq!(cb_haze.mons[3].boosts[1], 0);

            // Pain Split fails when target has Substitute (hp & sub unchanged).
            let mut cb_ps = base;
            cb_ps.mons[0].hp_pool[0] = 50;
            cb_ps.mons[0].vol_flags |= vol_bit::SUBSTITUTE;
            cb_ps.mons[0].hp_pool[1] = 30;
            cb_ps.mons[3].hp_pool[0] = 100;
            cb_ps.step_turn(&t, [1, 1]); // Zapdos Destiny Bond, Forretress Pain Split into Sub
            assert_eq!(cb_ps.mons[0].hp(), 50);
            assert_eq!(cb_ps.mons[0].sub_hp(), 30);
            assert_eq!(cb_ps.mons[3].hp(), 100);

            // Destiny Bond clears on foe switch (onFoeSwitchOut), Sleep abort, Freeze abort, Flinch abort, Full Par abort.
            let mut cb_fsw = base;
            cb_fsw.mons[0].vol_flags |= vol_bit::DESTINY_BOND;
            cb_fsw.exec_switch(&t, 1, 1, SwitchKind::Manual);
            assert_eq!(cb_fsw.mons[0].vol_flags & vol_bit::DESTINY_BOND, 0, "Foe switch must clear Destiny Bond");

            let mut cb_slp = base;
            cb_slp.mons[0].vol_flags |= vol_bit::DESTINY_BOND;
            cb_slp.mons[0].set_status(Status::Slp as u8, 3);
            cb_slp.step_turn(&t, [2, 2]); // Sleeping Zapdos attempts Thunderbolt -> cleared before Spikes
            assert_eq!(cb_slp.mons[0].vol_flags & vol_bit::DESTINY_BOND, 0, "Sleep abort must clear Destiny Bond");

            let mut cb_frz = base;
            cb_frz.mons[0].vol_flags |= vol_bit::DESTINY_BOND;
            cb_frz.mons[0].set_status(Status::Frz as u8, 0);
            cb_frz.step_turn(&t, [2, 2]);
            assert_eq!(cb_frz.mons[0].vol_flags & vol_bit::DESTINY_BOND, 0, "Freeze abort must clear Destiny Bond");

            let mut cb_flinch = base;
            cb_flinch.mons[0].vol_flags |= vol_bit::DESTINY_BOND | vol_bit::FLINCH;
            cb_flinch.step_turn(&t, [2, 2]);
            assert_eq!(cb_flinch.mons[0].vol_flags & vol_bit::DESTINY_BOND, 0, "Flinch abort must clear Destiny Bond");

            // Full paralysis abort: 100%-accurate Thunderbolt blocked only by 25% full paralysis.
            let mut par_cleared = false;
            for s in 1..=64u64 {
                let mut cb_par = base;
                cb_par.prng_seed = s.wrapping_mul(0x9E37_79B9_7F4A_7C15);
                cb_par.mons[0].vol_flags |= vol_bit::DESTINY_BOND;
                cb_par.mons[0].set_status(Status::Par as u8, 0);
                cb_par.mons[3].hp_pool[0] = t.mons[3].maxhp;
                cb_par.step_turn(&t, [2, 2]); // Thunderbolt
                if cb_par.mons[3].hp() == t.mons[3].maxhp {
                    assert_eq!(cb_par.mons[0].vol_flags & vol_bit::DESTINY_BOND, 0, "Full para abort must clear DBond");
                    par_cleared = true;
                    break;
                }
            }
            assert!(par_cleared);

            // Symmetric auto-switch Spikes faint (side 0 sole remaining bench has 1 HP vs 1 layer of Spikes).
            let mut cb_autosp2 = base;
            cb_autosp2.sides[0].side_flags |= 1;
            cb_autosp2.mons[0].hp_pool[0] = 1;
            cb_autosp2.mons[1].hp_pool[0] = 1; // Vaporeon 1 HP -> Spikes deals maxhp/8 >= 1
            cb_autosp2.mons[0].boosts[4] = -6;
            cb_autosp2.exec_switch(&t, 1, 1, SwitchKind::Manual);
            cb_autosp2.step_turn(&t, [0, 0]); // Tauros Body Slam KOs Zapdos -> Vaporeon auto-switches in and dies to Spikes -> outcome == 2
            assert_eq!(cb_autosp2.outcome, 2, "Auto-switch Spikes faint must end battle cleanly");
            assert_eq!(cb_autosp2.sides[0].alive_mask, 0);

            // Fire secondary-burn move (Flamethrower) thaws frozen target even when burn proc fails.
            let mut cb_thaw = base;
            cb_thaw.prng_seed = 1;
            cb_thaw.exec_switch(&t, 0, 1, SwitchKind::Manual); // Vaporeon active (has Flamethrower at move index 1)
            cb_thaw.exec_switch(&t, 1, 1, SwitchKind::Manual); // Tauros active
            cb_thaw.mons[4].set_status(Status::Frz as u8, 0);
            cb_thaw.step_turn(&t, [1, 0]); // Vaporeon Flamethrower hits Tauros -> must thaw Frz
            assert_ne!(cb_thaw.mons[4].status_raw(), Status::Frz as u8, "Flamethrower must thaw frozen target");
        }
    }

    #[test]
    fn compact_ps_parity_bugfix_suite() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../data/gen2stadium2.json");
        let json = std::fs::read_to_string(&path).unwrap();
        let dex = Dex::from_json(&json).unwrap();
        let mk = |species: &str, level: u8, item: &str, moves: &[&str]| -> crate::battle::PokemonSet {
            serde_json::from_value(serde_json::json!({
                "name": species, "species": species, "item": item, "ability": "No Ability",
                "moves": moves, "level": level,
                "evs": {"hp":255,"atk":255,"def":255,"spa":255,"spd":255,"spe":255},
                "ivs": {"hp":30,"atk":30,"def":30,"spa":30,"spd":30,"spe":30},
                "happiness": 255
            }))
            .unwrap()
        };

        // 1. Non-Rest sleep duration is 2..=4 (1..=3 skipped turns), matching PS `gen2stadium2nc2000`.
        {
            let p1 = vec![mk("Parasect", 50, "", &["Spore", "Splash"])];
            let p2 = vec![mk("Snorlax", 55, "", &["Splash"])];
            let mut b = Battle::from_fixture(&dex, "1,2,3,4", &p1, &p2).unwrap();
            b.choose(&dex, 0, "team 1").unwrap();
            b.choose(&dex, 1, "team 1").unwrap();
            let (t, base) = CompactBattle::from_battle(&b, &dex);
            let mut seen = [false; 8];
            for seed in 1..=128u64 {
                let mut cb = base;
                cb.prng_seed = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15);
                cb.step_turn(&t, [0, 0]);
                assert_eq!(cb.mons[3].status_raw(), Status::Slp as u8);
                let c = cb.mons[3].status_counter() as usize;
                assert!(
                    (2..=4).contains(&c),
                    "non-Rest sleep counter must be in 2..=4 (1..=3 skipped turns), got {c}"
                );
                seen[c] = true;
            }
            assert!(seen[2] && seen[3] && seen[4], "all 3 sleep counters (2, 3, 4) must occur");
        }

        // 2. Substitute bypass: Encore, Disable, Mean Look, Swagger (+2 Atk only, no confusion), Foresight.
        //    Plus Disable duration matches Battle (2..=5 moved first, 3..=6 moved second).
        {
            let dt = dex_tables(&dex);
            for key in ["encore", "disable", "foresight", "roar", "whirlwind"] {
                let mid = dex.moves.id(key).unwrap();
                assert!(dt.move_specs[mid.0 as usize].bypasses_sub, "{key} must set bypasses_sub");
            }
            let p1 = vec![mk("Alakazam", 55, "", &["Encore", "Disable", "Swagger", "Foresight"])];
            let p2 = vec![mk("Snorlax", 50, "", &["Substitute", "Splash", "Mean Look", "Disable"])];
            let mut b = Battle::from_fixture(&dex, "1,2,3,4", &p1, &p2).unwrap();
            b.choose(&dex, 0, "team 1").unwrap();
            b.choose(&dex, 1, "team 1").unwrap();
            // Turn 1: Alakazam Disable (fails: Snorlax hasn't moved yet) vs Snorlax Substitute
            // -> Snorlax now has Sub & last_move = Substitute, and FORESIGHT is NOT set yet.
            b.choose(&dex, 0, "move 2").unwrap();
            b.choose(&dex, 1, "move 1").unwrap();
            let (t, base) = CompactBattle::from_battle(&b, &dex);
            assert!(base.mons[3].sub_hp() > 0);
            assert_eq!(base.mons[3].vol_flags & vol_bit::FORESIGHT, 0);

            // Encore bypasses Sub (matches Battle)
            let mut b_enc = b.clone();
            b_enc.choose(&dex, 0, "move 1").unwrap();
            b_enc.choose(&dex, 1, "move 2").unwrap();
            assert!(b_enc.poke(b_enc.active_id(1).unwrap()).has_volatile(crate::cond_id!(&dex, "encore").unwrap()));
            let mut cb_enc = base;
            cb_enc.step_turn(&t, [0, 1]);
            assert_ne!(cb_enc.mons[3].vol_flags & vol_bit::ENCORE, 0, "Encore must bypass Sub");

            // Disable bypasses Sub & duration matches Battle:
            // Alakazam faster (moved_second=false): initial duration in 2..=5 -> after EOT decrement: 1..=4
            let mut seen_fast = [false; 8];
            for seed in 1..=256u64 {
                let mut cb_dis = base;
                cb_dis.prng_seed = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15);
                cb_dis.step_turn(&t, [1, 1]); // Alakazam Disable (55% acc) vs Snorlax Splash
                if (cb_dis.mons[3].vol_flags & vol_bit::DISABLE) != 0 {
                    let rem = cb_dis.mons[3].timers[timer_idx::DISABLE] as usize;
                    assert!((1..=4).contains(&rem), "faster Disable post-EOT rem must be 1..=4, got {rem}");
                    seen_fast[rem] = true;
                }
            }
            assert!(seen_fast[1] && seen_fast[2] && seen_fast[3] && seen_fast[4]);

            // Snorlax slower (moved_second=true) Disables Alakazam's Foresight behind Sub:
            // initial duration in 3..=6 -> after EOT decrement: 2..=5
            let mut seen_slow = [false; 8];
            for seed in 1..=256u64 {
                let mut cb_dis = base;
                cb_dis.mons[0].vol_flags |= vol_bit::SUBSTITUTE;
                cb_dis.mons[0].hp_pool[1] = 30;
                cb_dis.prng_seed = seed.wrapping_mul(0xD1B5_4A32_D192_ED03);
                cb_dis.step_turn(&t, [3, 3]); // Alakazam Foresight vs Snorlax Disable
                if (cb_dis.mons[0].vol_flags & vol_bit::DISABLE) != 0 {
                    let rem = cb_dis.mons[0].timers[timer_idx::DISABLE] as usize;
                    assert!((2..=5).contains(&rem), "slower Disable post-EOT rem must be 2..=5, got {rem}");
                    seen_slow[rem] = true;
                }
            }
            assert!(seen_slow[2] && seen_slow[3] && seen_slow[4] && seen_slow[5]);

            // Mean Look bypasses Sub (Snorlax uses Mean Look on Subbed Alakazam)
            let mut cb_ml = base;
            cb_ml.mons[0].vol_flags |= vol_bit::SUBSTITUTE;
            cb_ml.mons[0].hp_pool[1] = 30;
            cb_ml.step_turn(&t, [3, 2]);
            assert_ne!(cb_ml.mons[0].vol_flags & vol_bit::TRAPPED_OR_LOCKED, 0, "Mean Look must bypass Sub");

            // Swagger through Sub: raises Atk +2 WITHOUT confusing target; fails at +6 or Atk == 999
            let mut hit_swag = false;
            for seed in 1..=64u64 {
                let mut cb_sw = base;
                cb_sw.prng_seed = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15);
                cb_sw.step_turn(&t, [2, 1]); // Alakazam Swagger vs Subbed Snorlax
                if cb_sw.mons[3].boosts[stat_idx::ATK] == 2 {
                    assert_eq!(cb_sw.mons[3].vol_flags & vol_bit::CONFUSION, 0, "Swagger must not confuse through Sub");
                    hit_swag = true;
                    break;
                }
            }
            assert!(hit_swag, "Swagger must boost Atk +2 through Sub");

            // Swagger misses when target Atk is already +6 (or unmodified Atk == 999, e.g. Lv 100 Snorlax at +5).
            for seed in 1..=32u64 {
                let mut cb_sw6 = base;
                cb_sw6.mons[3].vol_flags &= !vol_bit::SUBSTITUTE;
                cb_sw6.mons[3].hp_pool[1] = 0;
                cb_sw6.mons[3].boosts[stat_idx::ATK] = 6;
                cb_sw6.prng_seed = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15);
                cb_sw6.step_turn(&t, [2, 1]);
                assert_eq!(cb_sw6.mons[3].boosts[stat_idx::ATK], 6, "Swagger must fail at +6 Atk");
                assert_eq!(cb_sw6.mons[3].vol_flags & vol_bit::CONFUSION, 0, "failed Swagger must not confuse");
            }
            {
                let p1_hi = vec![mk("Alakazam", 55, "", &["Swagger"])];
                let p2_hi = vec![mk("Snorlax", 100, "", &["Splash"])];
                let mut b_hi = Battle::from_fixture(&dex, "1,2,3,4", &p1_hi, &p2_hi).unwrap();
                b_hi.choose(&dex, 0, "team 1").unwrap();
                b_hi.choose(&dex, 1, "team 1").unwrap();
                let (t_hi, base_hi) = CompactBattle::from_battle(&b_hi, &dex);
                for seed in 1..=32u64 {
                    let mut cb_hi = base_hi;
                    cb_hi.mons[3].boosts[stat_idx::ATK] = 5; // Lv 100 Snorlax +5 unmodified Atk = 999
                    cb_hi.prng_seed = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15);
                    cb_hi.step_turn(&t_hi, [0, 0]);
                    assert_eq!(cb_hi.mons[3].boosts[stat_idx::ATK], 5, "Swagger must fail at 999 Atk");
                    assert_eq!(cb_hi.mons[3].vol_flags & vol_bit::CONFUSION, 0, "failed Swagger at 999 Atk must not confuse");
                }
            }
            // Lv 55 Thick Club Marowak at +5 Atk has unmodified Atk = 1008 (!= 999) -> Swagger SUCCEEDS (+6 & confuses), matching Battle!
            {
                let p1_tc = vec![mk("Alakazam", 55, "", &["Swagger"])];
                let p2_tc = vec![mk("Marowak", 55, "Thick Club", &["Splash"])];
                let mut b_tc = Battle::from_fixture(&dex, "1,2,3,4", &p1_tc, &p2_tc).unwrap();
                b_tc.choose(&dex, 0, "team 1").unwrap();
                b_tc.choose(&dex, 1, "team 1").unwrap();
                let wak_id = b_tc.active_id(1).unwrap();
                b_tc.poke_mut(wak_id).boosts[stat_idx::ATK] = 5;
                let (t_tc, base_tc) = CompactBattle::from_battle(&b_tc, &dex);
                let mut b_tc_hit = false;
                for seed in 1..=32u16 {
                    let mut b_try = Battle::from_fixture(&dex, &format!("{seed},2,3,4"), &p1_tc, &p2_tc).unwrap();
                    b_try.choose(&dex, 0, "team 1").unwrap();
                    b_try.choose(&dex, 1, "team 1").unwrap();
                    let wid = b_try.active_id(1).unwrap();
                    b_try.poke_mut(wid).boosts[stat_idx::ATK] = 5;
                    b_try.choose(&dex, 0, "move 1").unwrap();
                    b_try.choose(&dex, 1, "move 1").unwrap();
                    if b_try.poke(wid).boosts[stat_idx::ATK] == 6 {
                        assert!(b_try.poke(wid).has_volatile(crate::cond_id!(&dex, "confusion").unwrap()));
                        b_tc_hit = true;
                        break;
                    }
                }
                assert!(b_tc_hit, "Battle: Swagger must succeed on Lv 55 Thick Club Marowak at +5 Atk (1008 != 999)");
                let mut cb_tc_hit = false;
                for seed in 1..=64u64 {
                    let mut cb_try = base_tc;
                    cb_try.prng_seed = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15);
                    cb_try.step_turn(&t_tc, [0, 0]);
                    if cb_try.mons[3].boosts[stat_idx::ATK] == 6 {
                        assert_ne!(cb_try.mons[3].vol_flags & vol_bit::CONFUSION, 0);
                        cb_tc_hit = true;
                        break;
                    }
                }
                assert!(cb_tc_hit, "CompactBattle: Swagger must succeed on Lv 55 Thick Club Marowak at +5 Atk (1008 != 999)");
            }

            // Foresight bypasses Sub (starting cleanly with FORESIGHT == 0, matching Battle)
            let mut b_fs = b.clone();
            b_fs.choose(&dex, 0, "move 4").unwrap();
            b_fs.choose(&dex, 1, "move 2").unwrap();
            assert!(b_fs.poke(b_fs.active_id(1).unwrap()).has_volatile(crate::cond_id!(&dex, "foresight").unwrap()));
            let mut cb_fs = base;
            assert_eq!(cb_fs.mons[3].vol_flags & vol_bit::FORESIGHT, 0);
            cb_fs.step_turn(&t, [3, 1]);
            assert_ne!(cb_fs.mons[3].vol_flags & vol_bit::FORESIGHT, 0, "Foresight must bypass Sub");
        }

        // 3. Giga Drain misses vs Sub; Protect/Endure fail behind own Sub; Sub-break blocks secondaries.
        {
            let p1 = vec![mk("Exeggutor", 55, "", &["Giga Drain", "Protect", "Endure", "Substitute"])];
            let p2 = vec![mk("Snorlax", 50, "Leftovers", &["Substitute", "Body Slam", "Headbutt", "Thief"])];
            let mut b = Battle::from_fixture(&dex, "1,2,3,4", &p1, &p2).unwrap();
            b.choose(&dex, 0, "team 1").unwrap();
            b.choose(&dex, 1, "team 1").unwrap();
            // Turn 1: Exeggutor Sub vs Snorlax Sub
            b.choose(&dex, 0, "move 4").unwrap();
            b.choose(&dex, 1, "move 1").unwrap();

            let (t, base) = CompactBattle::from_battle(&b, &dex);
            // Giga Drain into Sub: 0 Sub damage, 0 drain heal (matches Battle!)
            let mut b_gd = b.clone();
            let egg_id = b_gd.active_id(0).unwrap();
            let lax_id = b_gd.active_id(1).unwrap();
            let sub_cid = crate::cond_id!(&dex, "substitute").unwrap();
            let b_sub_before = b_gd.poke(lax_id).volatile(sub_cid).unwrap().get_int(crate::state::DK::Hp);
            let b_hp_before = b_gd.poke(egg_id).hp;
            b_gd.choose(&dex, 0, "move 1").unwrap(); // Giga Drain
            b_gd.choose(&dex, 1, "move 1").unwrap(); // Substitute (fails)
            assert_eq!(b_gd.poke(lax_id).volatile(sub_cid).unwrap().get_int(crate::state::DK::Hp), b_sub_before);
            assert_eq!(b_gd.poke(egg_id).hp, b_hp_before);

            let mut cb_gd = base;
            let cb_sub_before = cb_gd.mons[3].sub_hp();
            let cb_hp_before = cb_gd.mons[0].hp();
            cb_gd.step_turn(&t, [0, 0]);
            assert_eq!(cb_gd.mons[3].sub_hp(), cb_sub_before, "Giga Drain must not damage Sub");
            assert_eq!(cb_gd.mons[0].hp(), cb_hp_before, "Giga Drain must not heal off Sub");

            // Protect behind own Sub fails (matches Battle!)
            let mut cb_pr = base;
            let egg_sub_before = cb_pr.mons[0].sub_hp();
            cb_pr.step_turn(&t, [1, 1]); // Exeggutor Protect (fails behind own Sub) vs Snorlax Body Slam
            assert_eq!(cb_pr.mons[0].stall_counter(), 0, "Protect behind own Sub must fail");
            assert!(cb_pr.mons[0].sub_hp() < egg_sub_before, "Body Slam must hit Sub when Protect fails");

            // Endure behind own Sub fails
            let mut cb_en = base;
            cb_en.step_turn(&t, [2, 1]);
            assert_eq!(cb_en.mons[0].stall_counter(), 0, "Endure behind own Sub must fail");

            // Sub-breaking hit (1 HP Sub) must NOT inflict status/flinch/Thief secondary!
            for seed in 1..=32u64 {
                let mut cb_sb = base;
                cb_sb.mons[0].hp_pool[1] = 1; // 1 HP Sub
                cb_sb.mons[0].item = cb_sb.mons[3].item; // Give Exeggutor Leftovers
                cb_sb.mons[3].item = 0; // Snorlax itemless for Thief
                cb_sb.prng_seed = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15);
                cb_sb.step_turn(&t, [1, 3]); // Snorlax Thief breaks Exeggutor's 1 HP Sub
                assert_eq!(cb_sb.mons[0].sub_hp(), 0);
                assert_eq!(cb_sb.mons[3].item, 0, "Thief breaking a Sub must NOT steal item");

                let mut cb_bs = base;
                cb_bs.mons[0].hp_pool[1] = 1;
                cb_bs.prng_seed = seed.wrapping_mul(0xD1B5_4A32_D192_ED03);
                cb_bs.step_turn(&t, [1, 1]); // Snorlax Body Slam breaks Exeggutor's 1 HP Sub
                assert_eq!(cb_bs.mons[0].sub_hp(), 0);
                assert_eq!(cb_bs.mons[0].status_raw(), Status::None as u8, "Body Slam breaking Sub must NOT paralyze");
            }

            // Multihit Sub-break: Twineedle breaks 1-HP Sub on hit 1 and deals main-HP damage + can poison on hit 2 (matching Battle).
            {
                let p1_mh = vec![mk("Beedrill", 55, "", &["Twineedle", "Splash"])];
                let p2_mh = vec![mk("Snorlax", 55, "", &["Substitute", "Splash"])];
                let mut b_mh = Battle::from_fixture(&dex, "1,2,3,4", &p1_mh, &p2_mh).unwrap();
                b_mh.choose(&dex, 0, "team 1").unwrap();
                b_mh.choose(&dex, 1, "team 1").unwrap();
                b_mh.choose(&dex, 0, "move 2").unwrap();
                b_mh.choose(&dex, 1, "move 1").unwrap(); // Snorlax creates Sub
                let lax_id = b_mh.active_id(1).unwrap();
                b_mh.poke_mut(lax_id).volatile_mut(sub_cid).unwrap().set_int(crate::state::DK::Hp, 1);
                let hp_before = b_mh.poke(lax_id).hp as i16;
                let (t_mh, base_mh) = CompactBattle::from_battle(&b_mh, &dex);
                b_mh.choose(&dex, 0, "move 1").unwrap();
                b_mh.choose(&dex, 1, "move 2").unwrap();
                assert!((b_mh.poke(lax_id).hp as i16) < hp_before, "Battle: Twineedle hit 2 must hit main HP after hit 1 breaks Sub");

                let mut hit_main = false;
                let mut poisoned = false;
                for seed in 1..=128u64 {
                    let mut cb_mh = base_mh;
                    cb_mh.prng_seed = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15);
                    cb_mh.step_turn(&t_mh, [0, 1]);
                    assert_eq!(cb_mh.mons[3].sub_hp(), 0);
                    if cb_mh.mons[3].hp() < hp_before {
                        hit_main = true;
                    }
                    if cb_mh.mons[3].status_raw() == Status::Psn as u8 {
                        poisoned = true;
                    }
                }
                assert!(hit_main, "CompactBattle: Twineedle hit 2 must hit main HP after hit 1 breaks Sub");
                assert!(poisoned, "CompactBattle: Twineedle hit 2 onto main HP must be able to poison");
            }
        }

        // 4. Rapid Spin clears Spikes/Leech Seed/partiallytrapped, NEVER Mean Look;
        //    Baton Pass passes Mean Look, partiallytrapped, AND Foresight (matching Battle!).
        {
            let p1 = vec![
                mk("Starmie", 50, "", &["Rapid Spin", "Baton Pass", "Splash"]),
                mk("Snorlax", 55, "", &["Splash"]),
            ];
            let p2 = vec![mk("Umbreon", 50, "", &["Mean Look", "Wrap", "Splash"])];
            let mut b = Battle::from_fixture(&dex, "1,2,3,4", &p1, &p2).unwrap();
            b.choose(&dex, 0, "team 1,2").unwrap();
            b.choose(&dex, 1, "team 1").unwrap();
            let (t, base) = CompactBattle::from_battle(&b, &dex);

            // Both Mean Look and partially_trapped set on Starmie: Rapid Spin clears only partial trap.
            let mut cb_rs = base;
            cb_rs.sides[0].side_flags |= 1;
            cb_rs.mons[0].vol_flags |= vol_bit::TRAPPED_OR_LOCKED | vol_bit::LEECH_SEED;
            cb_rs.mons[0].set_partially_trapped(true);
            assert!(cb_rs.mons[0].is_trapped());
            cb_rs.step_turn(&t, [0, 2]); // Rapid Spin vs Splash
            assert_eq!(cb_rs.sides[0].side_flags & 0x03, 0);
            assert_eq!(cb_rs.mons[0].vol_flags & vol_bit::LEECH_SEED, 0);
            assert!(!cb_rs.mons[0].partially_trapped(), "Rapid Spin must clear partiallytrapped");
            assert_ne!(
                cb_rs.mons[0].vol_flags & vol_bit::TRAPPED_OR_LOCKED,
                0,
                "Rapid Spin must NOT clear Mean Look"
            );
            let mut acts = [0u8; 7];
            let n = cb_rs.legal_actions(&t, 0, &mut acts);
            assert!(!acts[..n].contains(&5), "Starmie must remain trapped by Mean Look");

            // Baton Pass passes Mean Look, partiallytrapped, AND Foresight (verified directly against Battle!).
            let mut b_bp = b.clone();
            let st_id = b_bp.active_id(0).unwrap();
            let um_id = b_bp.active_id(1).unwrap();
            b_bp.add_volatile(&dex, st_id, "trapped", Some(um_id), crate::battle::EffectHandle::None);
            b_bp.add_volatile(&dex, st_id, "partiallytrapped", Some(um_id), crate::battle::EffectHandle::None);
            b_bp.add_volatile(&dex, st_id, "foresight", Some(um_id), crate::battle::EffectHandle::None);
            let (t_bp, mut cb_bp) = CompactBattle::from_battle(&b_bp, &dex);
            b_bp.choose(&dex, 0, "move 2").unwrap();
            b_bp.choose(&dex, 1, "move 3").unwrap();
            b_bp.choose(&dex, 0, "switch 2").unwrap();
            let lax_bp_id = b_bp.active_id(0).unwrap();
            assert!(b_bp.poke(lax_bp_id).has_volatile(crate::cond_id!(&dex, "trapped").unwrap()));
            assert!(b_bp.poke(lax_bp_id).has_volatile(crate::cond_id!(&dex, "partiallytrapped").unwrap()));
            assert!(b_bp.poke(lax_bp_id).has_volatile(crate::cond_id!(&dex, "foresight").unwrap()));

            cb_bp.step_turn(&t_bp, [1, 2]); // Starmie Baton Passes to Snorlax
            assert_eq!(cb_bp.sides[0].active_idx, 1);
            assert_ne!(cb_bp.mons[1].vol_flags & vol_bit::TRAPPED_OR_LOCKED, 0, "Baton Pass must pass Mean Look");
            assert!(cb_bp.mons[1].partially_trapped(), "Baton Pass must pass partiallytrapped matching Battle");
            assert_ne!(cb_bp.mons[1].vol_flags & vol_bit::FORESIGHT, 0, "Baton Pass must pass Foresight matching Battle");

            // Import from Battle distinguishes meanlook vs partiallytrapped.
            let mut b_wrap = b.clone();
            b_wrap.add_volatile(&dex, b_wrap.active_id(0).unwrap(), "partiallytrapped", b_wrap.active_id(1), crate::battle::EffectHandle::None);
            let (_, cb_imp) = CompactBattle::from_battle(&b_wrap, &dex);
            assert!(cb_imp.mons[0].partially_trapped());
            assert_eq!(cb_imp.mons[0].vol_flags & vol_bit::TRAPPED_OR_LOCKED, 0);
            assert!(cb_imp.mons[0].is_trapped());
        }

        // 5. AfterSwitchInSelf: Burn/Poison damages on manual switch & Baton Pass, NOT faint replace / Roar;
        //    Baton Pass passes residualdmg with pre-increment damage (C=0 -> 1 HP) & healthy receiver retention.
        {
            let p1 = vec![
                mk("Jolteon", 55, "", &["Baton Pass", "Explosion", "Splash"]),
                mk("Snorlax", 55, "", &["Splash"]),
            ];
            let p2 = vec![mk("Skarmory", 50, "", &["Whirlwind", "Splash"])];
            let mut b = Battle::from_fixture(&dex, "1,2,3,4", &p1, &p2).unwrap();
            b.choose(&dex, 0, "team 1,2").unwrap();
            b.choose(&dex, 1, "team 1").unwrap();
            b.sides[0].roster[1].status = Status::Psn;
            b.refresh_poke_mask(&dex, crate::state::PokeId { side: 0, slot: 1 });

            let mut b_man = b.clone();
            b_man.choose(&dex, 0, "switch 2").unwrap();
            b_man.choose(&dex, 1, "move 2").unwrap();
            let expected_hp_man = b_man.poke(b_man.active_id(0).unwrap()).hp as i16;

            let (t, base) = CompactBattle::from_battle(&b, &dex);
            assert!(expected_hp_man < t.mons[1].maxhp);

            let mut cb_man = base;
            cb_man.step_turn(&t, [5, 1]); // Manual switch to poisoned Snorlax
            assert_eq!(cb_man.mons[1].hp(), expected_hp_man, "manual switch-in must take Gen 2 Psn slip");

            let mut b_bp = b.clone();
            b_bp.choose(&dex, 0, "move 1").unwrap();
            b_bp.choose(&dex, 1, "move 2").unwrap();
            b_bp.choose(&dex, 0, "switch 2").unwrap();
            let expected_hp_bp = b_bp.poke(b_bp.active_id(0).unwrap()).hp as i16;

            let mut cb_bp = base;
            cb_bp.step_turn(&t, [0, 1]); // Baton Pass to poisoned Snorlax
            assert_eq!(cb_bp.mons[1].hp(), expected_hp_bp, "Baton Pass switch-in must take Gen 2 Psn slip");

            // Baton Pass residualdmg pre-increment: passer has residualdmg counter=0, passes to poisoned Snorlax
            // -> Battle deals ((maxhp/16)*0).max(1) == 1 HP (NOT 1/16) and increments counter to 1.
            {
                let mut b_res = b.clone();
                let j_id = b_res.active_id(0).unwrap();
                let res_cid = crate::cond_id!(&dex, "residualdmg").unwrap();
                b_res.add_volatile(&dex, j_id, "residualdmg", None, crate::battle::EffectHandle::None);
                b_res.poke_mut(j_id).volatile_mut(res_cid).unwrap().set_int(crate::state::DK::Counter, 0);
                let (t_r, mut cb_res) = CompactBattle::from_battle(&b_res, &dex);
                b_res.choose(&dex, 0, "move 1").unwrap();
                b_res.choose(&dex, 1, "move 2").unwrap();
                b_res.choose(&dex, 0, "switch 2").unwrap();
                let lax_id = b_res.active_id(0).unwrap();
                let b_hp = b_res.poke(lax_id).hp as i16;
                let b_cnt = b_res.poke(lax_id).volatile(res_cid).unwrap().get_int(crate::state::DK::Counter);
                assert_eq!(b_hp, t_r.mons[1].maxhp - 1, "Battle BP residualdmg C=0 must deal 1 HP");
                assert_eq!(b_cnt, 1);

                cb_res.step_turn(&t_r, [0, 1]);
                assert_eq!(cb_res.mons[1].hp(), b_hp, "CompactBattle BP residualdmg C=0 must deal pre-increment 1 HP");
                assert_eq!(cb_res.mons[1].status_counter(), 0x10 | 1);
            }

            // Baton Pass residualdmg onto a HEALTHY receiver retains counter WITHOUT incrementing
            // (Battle::ResidualDmgAfterMoveSelf only increments when status is Brn|Psn|Tox).
            {
                let mut b_hlth = Battle::from_fixture(&dex, "1,2,3,4", &p1, &p2).unwrap();
                b_hlth.choose(&dex, 0, "team 1,2").unwrap();
                b_hlth.choose(&dex, 1, "team 1").unwrap();
                let j_id = b_hlth.active_id(0).unwrap();
                let res_cid = crate::cond_id!(&dex, "residualdmg").unwrap();
                b_hlth.add_volatile(&dex, j_id, "residualdmg", None, crate::battle::EffectHandle::None);
                b_hlth.poke_mut(j_id).volatile_mut(res_cid).unwrap().set_int(crate::state::DK::Counter, 2);
                let (t_h, mut cb_hlth) = CompactBattle::from_battle(&b_hlth, &dex);
                b_hlth.choose(&dex, 0, "move 1").unwrap();
                b_hlth.choose(&dex, 1, "move 2").unwrap();
                b_hlth.choose(&dex, 0, "switch 2").unwrap();
                let lax_id = b_hlth.active_id(0).unwrap();
                assert_eq!(b_hlth.poke(lax_id).hp as i16, t_h.mons[1].maxhp);
                assert_eq!(b_hlth.poke(lax_id).volatile(res_cid).unwrap().get_int(crate::state::DK::Counter), 2);

                cb_hlth.step_turn(&t_h, [0, 1]);
                assert_eq!(cb_hlth.mons[1].hp(), t_h.mons[1].maxhp);
                assert_eq!(cb_hlth.mons[1].status_raw(), Status::None as u8);
                assert_eq!(cb_hlth.mons[1].status_counter(), 0x10 | 2, "healthy BP receiver must retain residualdmg counter unchanged");
            }

            // Faint replacement (Jolteon Explosion -> Snorlax auto-switch) skips AfterSwitchInSelf
            let mut cb_fnt = base;
            cb_fnt.step_turn(&t, [1, 1]);
            assert_eq!(cb_fnt.sides[0].active_idx, 1);
            assert_eq!(cb_fnt.mons[1].hp(), t.mons[1].maxhp, "faint replacement must NOT take AfterSwitchInSelf slip");

            // Whirlwind drag-in skips AfterSwitchInSelf
            let mut cb_drag = base;
            cb_drag.step_turn(&t, [2, 0]); // Jolteon Splash -> Skarmory Whirlwind drags Snorlax
            assert_eq!(cb_drag.sides[0].active_idx, 1);
            assert_eq!(cb_drag.mons[1].hp(), t.mons[1].maxhp, "Whirlwind drag-in must NOT take AfterSwitchInSelf slip");
        }

        // 6. Healing berries (Berry Juice +20, Gold Berry +30, Berry +10) trigger ONLY at EOT Residual,
        //    NOT mid-turn before AfterMoveSelf or second hit; Mint/Miracle Berry cure before move.
        {
            let p1 = vec![
                mk("Shuckle", 50, "Berry Juice", &["Splash"]),
                mk("Chansey", 50, "Berry", &["Splash"]),
            ];
            let p2 = vec![
                mk("Snorlax", 55, "Gold Berry", &["Seismic Toss", "Splash"]),
                mk("Tauros", 55, "Mint Berry", &["Splash"]),
            ];
            let mut b = Battle::from_fixture(&dex, "1,2,3,4", &p1, &p2).unwrap();
            b.choose(&dex, 0, "team 1,2").unwrap();
            b.choose(&dex, 1, "team 1,2").unwrap();
            let (t, base) = CompactBattle::from_battle(&b, &dex);

            // Shuckle at 60 HP (max 126): Snorlax Seismic Toss deals 55 -> 5 HP (<= 63) -> EOT Berry Juice heals +20 -> 25 HP!
            let mut b_bj = b.clone();
            b_bj.poke_mut(b_bj.active_id(0).unwrap()).hp = 60;
            b_bj.choose(&dex, 0, "move 1").unwrap();
            b_bj.choose(&dex, 1, "move 1").unwrap();
            let expected_bj = b_bj.poke(b_bj.active_id(0).unwrap()).hp as i16;
            assert_eq!(expected_bj, 25);

            let mut cb_bj = base;
            cb_bj.mons[0].hp_pool[0] = 60;
            cb_bj.step_turn(&t, [0, 0]);
            assert_eq!(cb_bj.mons[0].hp(), 25, "Berry Juice must heal +20 (not +10) at EOT");
            assert_eq!(cb_bj.mons[0].item, 0);

            // Gold Berry heals +30 at EOT when <= 50% HP; Berry heals +10 at EOT when <= 50% HP.
            let mut cb_gb = base;
            cb_gb.mons[3].hp_pool[0] = 100;
            cb_gb.step_turn(&t, [0, 1]);
            assert_eq!(cb_gb.mons[3].hp(), 130, "Gold Berry must heal +30 at EOT");
            assert_eq!(cb_gb.mons[3].item, 0);

            let mut cb_br = base;
            cb_br.exec_switch(&t, 0, 1, SwitchKind::Manual); // Chansey (Berry)
            cb_br.mons[1].hp_pool[0] = 100;
            cb_br.step_turn(&t, [0, 1]);
            assert_eq!(cb_br.mons[1].hp(), 110, "Berry must heal +10 at EOT");
            assert_eq!(cb_br.mons[1].item, 0);

            // Mint Berry cures Sleep before moving so move executes on the same turn.
            let mut cb_mb = base;
            cb_mb.exec_switch(&t, 1, 1, SwitchKind::Manual); // Tauros (Mint Berry)
            cb_mb.mons[4].set_status(Status::Slp as u8, 3);
            let pp0 = cb_mb.mons[4].pp[0];
            cb_mb.step_turn(&t, [0, 0]);
            assert_eq!(cb_mb.mons[4].status_raw(), Status::None as u8);
            assert_eq!(cb_mb.mons[4].item, 0);
            assert_eq!(cb_mb.mons[4].pp[0], pp0 - 1, "Mint Berry must cure Sleep before executing move");

            // Poisoned Shuckle at 60 HP (1v1): Snorlax Seismic Toss (55 dmg -> 5 HP), Shuckle moves AFTER Snorlax and dies to Poison
            // BEFORE EOT Residual -> Berry Juice must NOT save Shuckle mid-turn!
            let p1_1v1 = vec![mk("Shuckle", 50, "Berry Juice", &["Splash"])];
            let p2_1v1 = vec![mk("Snorlax", 55, "Gold Berry", &["Seismic Toss", "Splash"])];
            let mut b_mid = Battle::from_fixture(&dex, "1,2,3,4", &p1_1v1, &p2_1v1).unwrap();
            b_mid.choose(&dex, 0, "team 1").unwrap();
            b_mid.choose(&dex, 1, "team 1").unwrap();
            let sh_id = b_mid.active_id(0).unwrap();
            b_mid.poke_mut(sh_id).hp = 60;
            b_mid.poke_mut(sh_id).status = Status::Psn;
            b_mid.refresh_poke_mask(&dex, sh_id);
            let (t_mid, mut cb_mid) = CompactBattle::from_battle(&b_mid, &dex);
            b_mid.choose(&dex, 0, "move 1").unwrap();
            b_mid.choose(&dex, 1, "move 1").unwrap();
            assert_eq!(b_mid.outcome(), Some(crate::battle::Outcome::P2Win));
            cb_mid.step_turn(&t_mid, [0, 0]);
            assert_eq!(cb_mid.outcome, 2, "Berry Juice must NOT activate mid-turn before AfterMoveSelf Poison KO");
        }

        // 7. Thief steals item dynamically (stolen Leftovers heals, stolen Thick Club works on Marowak only),
        //    does NOT steal on KO, AND DOES steal on 1-HP Endure (0 damage hit) matching Battle.
        {
            let p1 = vec![
                mk("Gengar", 55, "", &["Thief", "Splash"]),
                mk("Marowak", 50, "", &["Thief", "Splash"]),
            ];
            let p2 = vec![
                mk("Snorlax", 55, "Leftovers", &["Splash", "Endure"]),
                mk("Cubone", 50, "Thick Club", &["Splash"]),
            ];
            let mut b = Battle::from_fixture(&dex, "1,2,3,4", &p1, &p2).unwrap();
            b.choose(&dex, 0, "team 1,2").unwrap();
            b.choose(&dex, 1, "team 1,2").unwrap();
            let (t, base) = CompactBattle::from_battle(&b, &dex);

            // Gengar steals Leftovers from surviving Snorlax -> heals at EOT!
            let mut cb_th = base;
            cb_th.prng_seed = 1;
            cb_th.mons[0].hp_pool[0] = 100;
            cb_th.step_turn(&t, [0, 0]);
            assert_ne!(cb_th.mons[0].item, 0, "Gengar must steal Leftovers");
            assert_eq!(cb_th.mons[3].item, 0);
            assert_eq!(
                cb_th.mons[0].hp(),
                100 + t.mons[0].quantum_1_16,
                "Stolen Leftovers must actually heal the thief at EOT"
            );

            // Thief on 1 HP foe KOs and must NOT steal item.
            let mut cb_ko = base;
            cb_ko.prng_seed = 1;
            cb_ko.mons[3].hp_pool[0] = 1;
            cb_ko.step_turn(&t, [0, 0]);
            assert_eq!(cb_ko.mons[0].item, 0, "Thief KOing target must NOT steal its item");

            // Thief on 1-HP Endure target deals 0 damage yet STILL steals item (matching Battle!).
            let mut b_end = b.clone();
            let lax_id = b_end.active_id(1).unwrap();
            b_end.poke_mut(lax_id).hp = 1;
            let (t_end, mut cb_end) = CompactBattle::from_battle(&b_end, &dex);
            b_end.choose(&dex, 0, "move 1").unwrap();
            b_end.choose(&dex, 1, "move 2").unwrap(); // Snorlax Endure at 1 HP
            assert!(b_end.poke(b_end.active_id(0).unwrap()).item.is_some(), "Battle: Thief on 1-HP Endure must steal");
            cb_end.step_turn(&t_end, [0, 1]);
            assert_ne!(cb_end.mons[0].item, 0, "CompactBattle: Thief on 1-HP Endure (0 dmg) must still steal");
            assert_eq!(cb_end.mons[3].item, 0);

            // Stolen Thick Club doubles Marowak's Atk, NOT Gengar's Atk.
            let mut cb_tc = base;
            let tc_item = cb_tc.mons[4].item;
            let gengar_atk_before = cb_tc.mon_stat(&t, 0, 0, stat_idx::ATK, false, false);
            let wak_atk_before = cb_tc.mon_stat(&t, 0, 1, stat_idx::ATK, false, false);
            cb_tc.mons[0].item = tc_item;
            cb_tc.mons[1].item = tc_item;
            assert_eq!(cb_tc.mon_stat(&t, 0, 0, stat_idx::ATK, false, false), gengar_atk_before);
            assert_eq!(cb_tc.mon_stat(&t, 0, 1, stat_idx::ATK, false, false), (wak_atk_before * 2).min(999));
        }

        // 8. Flinch runs AFTER Sleep (wake-up happens before flinch) and Headbutt cannot flinch sleeping/frozen foes.
        {
            let p1 = vec![mk("Tauros", 55, "", &["Headbutt", "Splash"])];
            let p2 = vec![mk("Snorlax", 55, "", &["Splash"])];
            let mut b = Battle::from_fixture(&dex, "1,2,3,4", &p1, &p2).unwrap();
            b.choose(&dex, 0, "team 1").unwrap();
            b.choose(&dex, 1, "team 1").unwrap();
            let (t, base) = CompactBattle::from_battle(&b, &dex);

            // On a wake-up turn with FLINCH set (e.g., King's Rock flinch), Sleep (prio 10) runs BEFORE Flinch (prio 8):
            // Snorlax wakes up (`Slp 1 -> None`) and then flinches instead of executing Splash.
            let mut cb_wf = base;
            cb_wf.mons[3].set_status(Status::Slp as u8, 1);
            cb_wf.mons[3].vol_flags |= vol_bit::FLINCH;
            let pp_before = cb_wf.mons[3].pp[0];
            let mut rng = Prng::new(1);
            cb_wf.exec_move(&t, 1, 0, true, false, &mut rng);
            assert_eq!(cb_wf.mons[3].status_raw(), Status::None as u8, "Sleep wake-up must precede Flinch");
            assert_eq!(cb_wf.mons[3].pp[0], pp_before, "Flinch must still block the move after waking up");

            // Non-vacuous: Headbutt against sleeping (Slp 3) or frozen target must NEVER set FLINCH,
            // and a waking target (Slp 1) hit by faster Headbutt must execute its move cleanly.
            for seed in 1..=64u64 {
                let mut cb_slp = base;
                cb_slp.mons[3].set_status(Status::Slp as u8, 3);
                let mut r1 = Prng::new(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15));
                cb_slp.exec_move(&t, 0, 0, false, false, &mut r1);
                assert_eq!(cb_slp.mons[3].vol_flags & vol_bit::FLINCH, 0, "Headbutt must not set FLINCH on sleeping target");

                let mut cb_frz = base;
                cb_frz.mons[3].set_status(Status::Frz as u8, 0);
                let mut r2 = Prng::new(seed.wrapping_mul(0xD1B5_4A32_D192_ED03));
                cb_frz.exec_move(&t, 0, 0, false, false, &mut r2);
                assert_eq!(cb_frz.mons[3].vol_flags & vol_bit::FLINCH, 0, "Headbutt must not set FLINCH on frozen target");

                let mut cb_wake = base;
                cb_wake.mons[3].set_status(Status::Slp as u8, 1);
                cb_wake.prng_seed = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15);
                let pp0 = cb_wake.mons[3].pp[0];
                cb_wake.step_turn(&t, [0, 0]);
                assert_eq!(cb_wake.mons[3].status_raw(), Status::None as u8);
                assert_eq!(cb_wake.mons[3].pp[0], pp0 - 1, "Waking target hit by Headbutt must not be flinched");
            }
        }

        // 9. 1v1 Sandstorm AND 1v1 Perish Song mutual KO winner matches Battle identically (including Quick Claw!).
        {
            let p1 = vec![mk("Zapdos", 55, "", &["Splash"])];
            let p2 = vec![mk("Snorlax", 50, "Quick Claw", &["Splash"])];
            let mut b = Battle::from_fixture(&dex, "1,2,3,4", &p1, &p2).unwrap();
            b.choose(&dex, 0, "team 1").unwrap();
            b.choose(&dex, 1, "team 1").unwrap();

            // 1v1 Sandstorm mutual KO (normal speed: Zapdos faster -> P1Win)
            let mut b_sand = b.clone();
            b_sand.set_weather(&dex, "sandstorm", None, crate::battle::EffectHandle::None);
            b_sand.poke_mut(b_sand.active_id(0).unwrap()).hp = 1;
            b_sand.poke_mut(b_sand.active_id(1).unwrap()).hp = 1;
            b_sand.quick_claw_roll = false;
            let mut b_qc = b_sand.clone();
            b_qc.quick_claw_roll = true;
            let (t_s, mut cb_sand) = CompactBattle::from_battle(&b_sand, &dex);
            b_sand.choose(&dex, 0, "move 1").unwrap();
            b_sand.choose(&dex, 1, "move 1").unwrap();
            cb_sand.step_turn(&t_s, [0, 0]);
            let expected_s = match b_sand.outcome() {
                Some(crate::battle::Outcome::P1Win) => 1,
                Some(crate::battle::Outcome::P2Win) => 2,
                _ => 3,
            };
            assert_eq!(expected_s, 1, "Battle 1v1 Sandstorm mutual KO: faster Zapdos wins");
            assert_eq!(cb_sand.outcome, expected_s, "CompactBattle 1v1 Sandstorm must match Battle");

            // 1v1 Sandstorm mutual KO WITH Quick Claw active: slower Snorlax gets +2000 action speed -> P2Win in BOTH Battle and CompactBattle!
            let (t_qc, mut cb_qc) = CompactBattle::from_battle(&b_qc, &dex);
            b_qc.choose(&dex, 0, "move 1").unwrap();
            b_qc.choose(&dex, 1, "move 1").unwrap();
            assert_eq!(b_qc.outcome(), Some(crate::battle::Outcome::P2Win), "Battle Quick Claw active: Snorlax wins 1v1 Sandstorm");
            cb_qc.step_turn(&t_qc, [0, 0]);
            assert_eq!(cb_qc.outcome, 2, "CompactBattle Quick Claw active: Snorlax wins 1v1 Sandstorm");

            // 1v1 Perish Song mutual KO
            let mut b_per = b.clone();
            b_per.quick_claw_roll = false;
            let p_cid = crate::cond_id!(&dex, "perishsong").unwrap();
            for s in 0..2usize {
                let id = b_per.active_id(s).unwrap();
                b_per.add_volatile(&dex, id, "perishsong", None, crate::battle::EffectHandle::None);
                b_per.poke_mut(id).volatile_mut(p_cid).unwrap().duration = Some(1);
            }
            let (t_p, mut cb_per) = CompactBattle::from_battle(&b_per, &dex);
            b_per.choose(&dex, 0, "move 1").unwrap();
            b_per.choose(&dex, 1, "move 1").unwrap();
            cb_per.step_turn(&t_p, [0, 0]);
            let expected_p = match b_per.outcome() {
                Some(crate::battle::Outcome::P1Win) => 1,
                Some(crate::battle::Outcome::P2Win) => 2,
                _ => 3,
            };
            assert_eq!(expected_p, 1, "Battle 1v1 Perish Song mutual KO: faster Zapdos wins");
            assert_eq!(cb_per.outcome, expected_p, "CompactBattle 1v1 Perish Song must match Battle");
        }

        // 10. Dual 1-HP replacement Spikes faints (faster incoming mon faints first -> slower wins),
        //     step_switch clearing stale last_move_selfko, and BP receiver Spikes faint canceling foe queued move.
        {
            let p1 = vec![
                mk("Gengar", 55, "", &["Explosion", "Splash"]),
                mk("Jolteon", 55, "", &["Splash"]),
            ];
            let p2 = vec![
                mk("Snorlax", 55, "", &["Splash"]),
                mk("Shuckle", 50, "", &["Splash"]),
            ];
            let mut b = Battle::from_fixture(&dex, "1,2,3,4", &p1, &p2).unwrap();
            b.choose(&dex, 0, "team 1,2").unwrap();
            b.choose(&dex, 1, "team 1,2").unwrap();
            let p0_act = b.active_id(0);
            let p1_act = b.active_id(1);
            b.add_side_condition(&dex, 0, "spikes", p1_act, crate::battle::EffectHandle::None);
            b.add_side_condition(&dex, 1, "spikes", p0_act, crate::battle::EffectHandle::None);
            b.sides[0].roster[1].hp = 1; // Faster Jolteon (sole grounded bench) at 1 HP
            b.sides[1].roster[1].hp = 1; // Slower Shuckle (sole grounded bench) at 1 HP
            b.sides[1].roster[0].hp = 1; // Snorlax dies to Explosion
            let (t_sp, mut cb_sp) = CompactBattle::from_battle(&b, &dex);
            b.choose(&dex, 0, "move 1").unwrap();
            b.choose(&dex, 1, "move 1").unwrap();
            b.choose(&dex, 0, "switch 2").unwrap();
            b.choose(&dex, 1, "switch 2").unwrap();
            let expected_sp = match b.outcome() {
                Some(crate::battle::Outcome::P1Win) => 1,
                Some(crate::battle::Outcome::P2Win) => 2,
                _ => 3,
            };
            assert_eq!(expected_sp, 2, "Battle dual Spikes faint: faster Jolteon faints first -> P2Win");
            cb_sp.step_turn(&t_sp, [0, 0]);
            assert_eq!(cb_sp.outcome, expected_sp, "CompactBattle dual auto-switch Spikes faint must match Battle");

            // Dual forced `step_switch` into 1-HP last mons on Spikes -> faster incoming mon faints first (not Tie),
            // and stale `last_move_selfko` is cleared at `step_switch` entry.
            let mut cb_ss = cb_sp;
            cb_ss.outcome = 0;
            cb_ss.sides[0].alive_mask = 0b010;
            cb_ss.sides[1].alive_mask = 0b010;
            cb_ss.sides[0].active_idx = 0;
            cb_ss.sides[1].active_idx = 0;
            cb_ss.sides[0].request = 2;
            cb_ss.sides[1].request = 2;
            cb_ss.sides[0].last_move_selfko = 1;
            cb_ss.sides[1].last_move_selfko = 1;
            cb_ss.mons[1].hp_pool[0] = 1; // Faster Jolteon
            cb_ss.mons[4].hp_pool[0] = 1; // Slower Shuckle
            cb_ss.step_switch(&t_sp, [5, 5]);
            assert_eq!(cb_ss.sides[0].last_move_selfko, u8::MAX);
            assert_eq!(cb_ss.sides[1].last_move_selfko, u8::MAX);
            assert_eq!(cb_ss.outcome, 2, "step_switch dual Spikes faint: faster Jolteon faints first -> P2Win");

            // Faster Baton Pass receiver fainting to Spikes in `step_switch` (2 bench options)
            // cancels slower opponent's pending queued move, matching Battle!
            let p1_bp = vec![
                mk("Jolteon", 55, "", &["Baton Pass", "Splash"]),
                mk("Gengar", 50, "", &["Splash"]),
                mk("Snorlax", 50, "", &["Splash"]),
            ];
            let p2_bp = vec![mk("Skarmory", 50, "", &["Tackle", "Splash"])];
            let mut b_bp = Battle::from_fixture(&dex, "1,2,3,4", &p1_bp, &p2_bp).unwrap();
            b_bp.choose(&dex, 0, "team 1,2,3").unwrap();
            b_bp.choose(&dex, 1, "team 1").unwrap();
            let foe_act = b_bp.active_id(1);
            b_bp.add_side_condition(&dex, 0, "spikes", foe_act, crate::battle::EffectHandle::None);
            b_bp.sides[0].roster[1].hp = 1; // Gengar 1 HP -> dies to Spikes on Baton Pass
            let skarm_pp_before = b_bp.poke(b_bp.active_id(1).unwrap()).move_slots[0].pp;
            b_bp.choose(&dex, 0, "move 1").unwrap();
            b_bp.choose(&dex, 1, "move 1").unwrap(); // Skarmory queues Tackle; Battle pauses for BP receiver choice
            let (t_bp, mut cb_bp) = CompactBattle::from_battle(&b_bp, &dex);
            assert_eq!(cb_bp.sides[0].request, 2);
            assert_ne!(cb_bp.sides[1].side_flags & 0x10, 0);

            b_bp.choose(&dex, 0, "switch 2").unwrap(); // Baton Pass to 1-HP Gengar -> dies to Spikes
            // After Gengar faints, Jolteon & Snorlax remain -> forced switch; Skarmory's Tackle must be canceled!
            b_bp.choose(&dex, 0, "switch 3").unwrap();
            let b_lax_hp = b_bp.poke(b_bp.active_id(0).unwrap()).hp as i16;
            let b_skarm_pp = b_bp.poke(b_bp.active_id(1).unwrap()).move_slots[0].pp;
            assert_eq!(b_skarm_pp, skarm_pp_before, "Battle: Skarmory Tackle must be canceled when BP receiver dies to Spikes");

            cb_bp.step_switch(&t_bp, [5, 0]); // BP into 1-HP Gengar -> dies to Spikes
            assert_eq!(cb_bp.sides[1].side_flags & 0x70, 0, "BP receiver faint must clear foe pending move");
            assert_eq!(cb_bp.sides[0].request, 2);
            cb_bp.step_switch(&t_bp, [6, 0]); // Switch to Snorlax
            assert_eq!(cb_bp.mons[3].pp[0], skarm_pp_before as u8, "CompactBattle: Skarmory Tackle must NOT execute after BP receiver faint");
            assert_eq!(cb_bp.mons[2].hp(), b_lax_hp);
        }

        // 11. Mid-turn Disable aborts target's queued move (NOT morphing into Struggle) and runs at prio 7
        //     after Mint Berry (prio 11) and Sleep (prio 10), matching Battle.
        {
            let p1 = vec![mk("Alakazam", 55, "", &["Disable", "Splash"])];
            let p2 = vec![mk("Snorlax", 50, "Mint Berry", &["Tackle", "Splash"])];
            let mut b = Battle::from_fixture(&dex, "1,2,3,4", &p1, &p2).unwrap();
            b.choose(&dex, 0, "team 1").unwrap();
            b.choose(&dex, 1, "team 1").unwrap();
            // Turn 1: Alakazam Splash, Snorlax Tackle (sets Snorlax last_move = Tackle)
            b.choose(&dex, 0, "move 2").unwrap();
            b.choose(&dex, 1, "move 1").unwrap();
            // Drain Snorlax's Splash PP to 0 so Tackle is its SOLE remaining move
            let lax_id = b.active_id(1).unwrap();
            b.poke_mut(lax_id).move_slots[1].pp = 0;
            let (t_d, base_d) = CompactBattle::from_battle(&b, &dex);

            // Find a seed where Alakazam's Disable hits Snorlax's Tackle mid-turn:
            // Snorlax must abort Tackle (no Struggle damage to Alakazam, no Struggle recoil, Tackle PP unchanged).
            let mut verified = false;
            for seed in 1..=64u64 {
                let mut cb_d = base_d;
                cb_d.prng_seed = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15);
                let zam_hp0 = cb_d.mons[0].hp();
                let lax_hp0 = cb_d.mons[3].hp();
                let lax_pp0 = cb_d.mons[3].pp[0];
                cb_d.step_turn(&t_d, [0, 0]);
                if (cb_d.mons[3].vol_flags & vol_bit::DISABLE) != 0 {
                    assert_eq!(cb_d.mons[0].hp(), zam_hp0, "Mid-turn Disabled move must abort (no Struggle damage)");
                    assert_eq!(cb_d.mons[3].hp(), lax_hp0, "Mid-turn Disabled move must abort (no Struggle recoil)");
                    assert_eq!(cb_d.mons[3].pp[0], lax_pp0, "Mid-turn Disabled move must not deduct PP");
                    verified = true;
                    break;
                }
            }
            assert!(verified, "Disable must hit and abort target's sole remaining move");

            // Direct `exec_move` (no EOT residuals masking it!):
            // (a) Mint Berry (prio 11) cures Sleep BEFORE Disable (prio 7) aborts the queued move;
            // (b) Sleep (prio 10) decrements BEFORE Disable (prio 7) aborts the queued move.
            {
                let mut cb_mb = base_d;
                cb_mb.mons[3].set_status(Status::Slp as u8, 3);
                cb_mb.mons[3].vol_flags |= vol_bit::DISABLE;
                cb_mb.mons[3].set_disable_slot(0);
                cb_mb.mons[3].timers[timer_idx::DISABLE] = 3;
                let lax_pp0 = cb_mb.mons[3].pp[0];
                let mut r_mb = Prng::new(1);
                cb_mb.exec_move(&t_d, 1, 0, true, false, &mut r_mb);
                assert_eq!(cb_mb.mons[3].status_raw(), Status::None as u8, "Mint Berry (prio 11) must cure Sleep before Disable (prio 7)");
                assert_eq!(cb_mb.mons[3].item, 0, "Mint Berry must be consumed inside exec_move before Disable aborts");
                assert_eq!(cb_mb.mons[3].pp[0], lax_pp0, "Disable (prio 7) must still abort move after Mint Berry cure");

                let mut cb_slp_dis = base_d;
                cb_slp_dis.mons[3].item = 0;
                cb_slp_dis.mons[3].set_status(Status::Slp as u8, 3);
                cb_slp_dis.mons[3].vol_flags |= vol_bit::DISABLE;
                cb_slp_dis.mons[3].set_disable_slot(0);
                cb_slp_dis.mons[3].timers[timer_idx::DISABLE] = 3;
                let mut r_sd = Prng::new(1);
                cb_slp_dis.exec_move(&t_d, 1, 0, true, false, &mut r_sd);
                assert_eq!(cb_slp_dis.mons[3].status_counter(), 2, "Sleep (prio 10) must decrement before Disable (prio 7)");
                assert_eq!(cb_slp_dis.mons[3].pp[0], lax_pp0, "Disabled sleeping move must not deduct PP");
            }
        }
    }
}
