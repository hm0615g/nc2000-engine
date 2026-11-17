//! Linear-algebra transition and Bellman-Markov operators on [`CompactBattle`]:
//!
//! 1. **1D Banded Toeplitz Damage & Residual Kernel ([`DmgKernel1D`], [`survival_curve_4`])**:
//!    Exact Gen 2 damage roll + critical hit + accuracy/evasion + BrightPowder +
//!    `Sleep Talk` uniform slot mixture + end-of-turn residual shift operator $T$.
//!    Evaluates $k$-turn survival probabilities $S(k) = (\mathbf{1}^\top T^k \mathbf{e}_{h_0})$
//!    with zero heap allocations.
//! 2. **2D Coupled Schur-Complement Bellman Solver & Rank-3 SVD ([`solve_schur_bellman_2d`], [`SolvedBellmanSvd`])**:
//!    For matchups with active recovery (`Rest` or 50% heals), solves the coupled
//!    Bellman-Shapley fixed-point equation $(I - \gamma P_{\pi^*}) V^* = r_{\pi^*}$ over
//!    the canonical $16 \times 16 \times 3 \times 3$ `(hp0_bin, hp1_bin, sleep0, sleep1)` state space
//!    (where $1/16$ is the exact Gen 2 Leftovers/residual quantum).
//!    Sleeping states ($s_0 + s_1 > 0$) are eliminated in a single bottom-up Schur-complement pass per
//!    sweep, and the awake stage game is solved in closed form ($2 \times 2$ minimax/Nash).
//!    The resulting awake value surface is decomposed into its top-3 singular vectors
//!    $V(b_0, b_1) - 0.5 \approx \sum_{r=1}^3 \sigma_r u_r(b_0) v_r(b_1)$ and cached in a
//!    fixed-size thread-local table.
//! 3. **Exact Stopping-Time Bilinear Race ([`toeplitz_coupled_race`])**:
//!    For non-healing matchups, evaluates the exact discrete-time stopping-time race probability
//!    $\mathbb{P}(\tau_0 > \tau_1) - \mathbb{P}(\tau_1 > \tau_0) = \sum_k (f_1(k) S_0(k) - f_0(k) S_1(k) + f_0(k) f_1(k) \text{spd\_sign})$
//!    from the 1D Toeplitz survival curves $T_0^k \mathbf{1}$ and $T_1^k \mathbf{1}$.
//! 4. **Perish Song / Mean Look Countdown Survival ([`compact_perish_margin`])**:
//!    Evaluates active Perish Song trap survival directly from the Toeplitz survival curve
//!    $S_{\text{trapper}}(c - 1 - \mathbf{1}_{\text{protect}}) \cdot (1 - \mathbf{1}_{\text{phaze}} S_{\text{victim}}(1))$.

use std::cell::RefCell;

use nc2000_engine::compact::{
    lookup_accuracy_threshold, role_bit, vol_bit, CompactBattle, CompactMoveSpec, CompactWeather,
    MatchTable,
};
use nc2000_engine::state::Status;

/// Canonical Gen 2 HP grid resolution ($1/16 = 6.25\%$, matching Leftovers $1/16$,
/// Burn/Poison/Leech Seed $2/16$, Substitute $4/16$, and Recover/Soft-Boiled $8/16$).
pub const B16: usize = 16;

/// Fine 1D grid resolution for pure stopping-time convolutions without recovery.
const B32: usize = 32;

/// Discount factor $\gamma$ pulling infinite stall cycles toward the tie value $0.5$.
const GAMMA: f32 = 0.985;

/// Resolvent discount factor $\gamma_1$ for stationary occupancy measure $\bar{\mu}$ extraction.
const GAMMA_RESOLVENT: f32 = 0.995;

/// Anti-procrastination discount factor $\gamma_{\text{slow}}$ on slow-manifold phase horizons.
const GAMMA_SLOW: f64 = 0.997;
const LAMBDA_SLOW: f64 = 0.0030045090164307026; // -ln(0.997)

#[inline]
fn instant_heal_amount(cb: &CompactBattle, ms: &CompactMoveSpec, maxhp: i16) -> Option<i16> {
    if ms.is_half_heal {
        Some(maxhp / 2)
    } else if ms.special_kind == 29 {
        let amt = if cb.weather_turns > 0 {
            match cb.weather_kind {
                2 => maxhp,
                1 | 3 => (maxhp / 4).max(1),
                _ => (maxhp / 2).max(1),
            }
        } else {
            (maxhp / 2).max(1)
        };
        Some(amt)
    } else {
        None
    }
}

#[inline]
fn is_spite_move(table: &MatchTable, ms: &CompactMoveSpec) -> bool {
    ms.is_status
        && ms.move_type == table.ghost_type
        && ms.special_kind == 0
        && ms.primary_vol == 0
        && ms.primary_status == 0
        && ms.boosts[0].0 == 0xFF
}

/// Strictly symmetric standard normal CDF $\Phi(z) \in [0, 1]$ satisfying $\Phi(-z) = 1 - \Phi(z)$ to 0 ULP
/// via `0.5 + sign(z) * half_erf(|z| / sqrt(2))`.
#[inline]
fn norm_cdf_sym(z: f64) -> f64 {
    if !z.is_finite() {
        return if z.is_sign_positive() { 1.0 } else { 0.0 };
    }
    let ax = z.abs() * std::f64::consts::FRAC_1_SQRT_2;
    if ax < 1e-12 {
        return 0.5;
    }
    if ax >= 6.0 {
        return if z >= 0.0 { 1.0 } else { 0.0 };
    }
    // Abramowitz & Stegun 7.1.26 (max error < 1.5e-7)
    let t = 1.0 / (1.0 + 0.3275911 * ax);
    let poly = t
        * (0.254829592
            + t * (-0.284496736 + t * (1.421413741 + t * (-1.453152027 + t * 1.061405429))));
    let half_erf = 0.5 * (1.0 - poly * (-ax * ax).exp()).clamp(0.0, 1.0);
    if z >= 0.0 {
        0.5 + half_erf
    } else {
        0.5 - half_erf
    }
}

#[inline]
fn phi1_decay(kappa: f64, t: f64) -> f64 {
    phi1_and_exp(kappa, t).0
}

#[inline]
fn phi1_and_exp(kappa: f64, t: f64) -> (f64, f64) {
    if t <= 0.0 {
        return (0.0, 1.0);
    }
    let kt = kappa * t;
    if kt < 1e-5 {
        (t * (1.0 - 0.5 * kt), 1.0 - kt * (1.0 - 0.5 * kt))
    } else {
        let e = (-kt).exp();
        ((1.0 - e) / kappa, e)
    }
}

/// Stack-allocated 1D damage distribution kernel (up to 16 `(dmg, prob)` support points).
#[derive(Clone, Copy, Debug)]
pub struct DmgKernel1D {
    pub pts: [(i16, f32); 16],
    pub len: usize,
    pub max_dmg: i16,
    pub mean_dmg: f32,
}

impl DmgKernel1D {
    #[inline]
    pub const fn zero() -> Self {
        Self {
            pts: [(0, 1.0); 16],
            len: 1,
            max_dmg: 0,
            mean_dmg: 0.0,
        }
    }

    #[inline]
    fn push_mass(&mut self, dmg: i16, prob: f32) {
        if prob <= 0.0 {
            return;
        }
        if dmg > self.max_dmg {
            self.max_dmg = dmg;
        }
        self.mean_dmg += (dmg as f32) * prob;
        for i in 0..self.len {
            if self.pts[i].0 == dmg {
                self.pts[i].1 += prob;
                return;
            }
        }
        if self.len < 16 {
            self.pts[self.len] = (dmg, prob);
            self.len += 1;
        } else {
            self.pts[self.len - 1].1 += prob;
        }
    }

    /// Probability that a single attack from this kernel KOs a defender at `def_hp`.
    #[inline]
    pub fn ko_prob(&self, def_hp: i16) -> f32 {
        if def_hp <= 0 {
            return 1.0;
        }
        if self.max_dmg < def_hp {
            return 0.0;
        }
        let mut p = 0.0f32;
        for i in 0..self.len {
            if self.pts[i].0 >= def_hp {
                p += self.pts[i].1;
            }
        }
        p.clamp(0.0, 1.0)
    }

    /// Build the 1D damage kernel for the best damaging move of `(att_side, att_slot)` against `(def_side, def_slot)`.
    #[inline]
    pub fn best_attack(
        cb: &CompactBattle,
        table: &MatchTable,
        att_side: usize,
        att_slot: usize,
        def_side: usize,
        def_slot: usize,
    ) -> Self {
        let att_idx = att_side * 3 + att_slot;
        let def_idx = def_side * 3 + def_slot;
        let att_mon = &cb.mons[att_idx];
        let def_hp = cb.mons[def_idx].hp().max(1);
        let num_moves = table.mons[att_idx].num_moves as usize;
        let is_disabled = (att_mon.vol_flags & vol_bit::DISABLE) != 0;
        let dis_slot = ((att_mon.slot_refs >> 4) & 0x03) as usize;
        let last_mon = cb.sides[att_side].alive_mask.count_ones() <= 1;

        let mut best_kern = Self::zero();
        let mut best_score = 0.0f32;
        for m_i in 0..num_moves {
            if att_mon.pp[m_i] == 0 || (is_disabled && m_i == dis_slot) {
                continue;
            }
            let mspec = &table.move_specs[att_idx][m_i];
            if last_mon && mspec.is_selfdestruct {
                continue;
            }
            let kern = Self::for_move(cb, table, att_side, att_slot, def_side, def_slot, m_i);
            if kern.max_dmg <= 0 {
                continue;
            }
            let p_ko = kern.ko_prob(def_hp);
            let frac = (kern.mean_dmg / (def_hp as f32)).min(1.0);
            let raw_frac = kern.mean_dmg / (table.mons[def_idx].maxhp.max(1) as f32);
            let score = p_ko + frac + 1e-3 * raw_frac;
            if score > best_score {
                best_score = score;
                best_kern = kern;
            }
        }
        best_kern
    }

    /// Build the 1D damage kernel for the best damaging move on the full-HP grid (independent of instantaneous HP).
    #[inline]
    pub fn best_attack_grid(
        cb: &CompactBattle,
        table: &MatchTable,
        att_side: usize,
        att_slot: usize,
        def_side: usize,
        def_slot: usize,
    ) -> Self {
        let att_idx = att_side * 3 + att_slot;
        let def_idx = def_side * 3 + def_slot;
        let att_mon = &cb.mons[att_idx];
        let def_maxhp = table.mons[def_idx].maxhp.max(1);
        let num_moves = table.mons[att_idx].num_moves as usize;
        let is_disabled = (att_mon.vol_flags & vol_bit::DISABLE) != 0;
        let dis_slot = ((att_mon.slot_refs >> 4) & 0x03) as usize;
        let last_mon = cb.sides[att_side].alive_mask.count_ones() <= 1;

        let mut best_kern = Self::zero();
        let mut best_score = 0.0f32;
        for m_i in 0..num_moves {
            if att_mon.pp[m_i] == 0 || (is_disabled && m_i == dis_slot) {
                continue;
            }
            let mspec = &table.move_specs[att_idx][m_i];
            if last_mon && mspec.is_selfdestruct {
                continue;
            }
            let kern = Self::for_move_impl(
                cb, table, att_side, att_slot, def_side, def_slot, m_i, true,
            );
            if kern.max_dmg <= 0 {
                continue;
            }
            let score = kern.mean_dmg / (def_maxhp as f32);
            if score > best_score {
                best_score = score;
                best_kern = kern;
            }
        }
        best_kern
    }

    /// Build the exact `Sleep Talk` mixture kernel: in Gen 2, `Sleep Talk` uniformly selects
    /// one of the 3 other move slots (`m != st_slot`) with weight $1 / (N_{\text{moves}} - 1)$.
    #[inline]
    pub fn sleep_talk(
        cb: &CompactBattle,
        table: &MatchTable,
        att_side: usize,
        att_slot: usize,
        def_side: usize,
        def_slot: usize,
    ) -> Self {
        Self::sleep_talk_impl(cb, table, att_side, att_slot, def_side, def_slot, false)
    }

    #[inline]
    pub fn sleep_talk_grid(
        cb: &CompactBattle,
        table: &MatchTable,
        att_side: usize,
        att_slot: usize,
        def_side: usize,
        def_slot: usize,
    ) -> Self {
        Self::sleep_talk_impl(cb, table, att_side, att_slot, def_side, def_slot, true)
    }

    #[inline]
    fn sleep_talk_impl(
        cb: &CompactBattle,
        table: &MatchTable,
        att_side: usize,
        att_slot: usize,
        def_side: usize,
        def_slot: usize,
        grid_mode: bool,
    ) -> Self {
        let att_idx = att_side * 3 + att_slot;
        let att_mon = &cb.mons[att_idx];
        let num_moves = table.mons[att_idx].num_moves as usize;
        let is_disabled = (att_mon.vol_flags & vol_bit::DISABLE) != 0;
        let dis_slot = ((att_mon.slot_refs >> 4) & 0x03) as usize;

        let mut st_slot = None;
        for m_i in 0..num_moves {
            if att_mon.pp[m_i] > 0
                && !(is_disabled && m_i == dis_slot)
                && table.move_specs[att_idx][m_i].special_kind == 10
            {
                st_slot = Some(m_i);
                break;
            }
        }
        let Some(st_i) = st_slot else {
            return Self::zero();
        };
        if num_moves <= 1 {
            return Self::zero();
        }
        let weight = 1.0 / ((num_moves - 1) as f32);
        let mut mix = Self {
            pts: [(0, 0.0); 16],
            len: 0,
            max_dmg: 0,
            mean_dmg: 0.0,
        };
        for m_i in 0..num_moves {
            if m_i == st_i {
                continue;
            }
            let mspec = &table.move_specs[att_idx][m_i];
            if mspec.is_rest || mspec.is_status || mspec.is_selfdestruct {
                mix.push_mass(0, weight);
                continue;
            }
            let k = Self::for_move_impl(
                cb, table, att_side, att_slot, def_side, def_slot, m_i, grid_mode,
            );
            for j in 0..k.len {
                mix.push_mass(k.pts[j].0, k.pts[j].1 * weight);
            }
        }
        if mix.len == 0 {
            Self::zero()
        } else {
            mix
        }
    }

    /// Build the 1D damage kernel for move slot `m_slot` of `(att_side, att_slot)` against `(def_side, def_slot)`.
    #[inline]
    pub fn for_move(
        cb: &CompactBattle,
        table: &MatchTable,
        att_side: usize,
        att_slot: usize,
        def_side: usize,
        def_slot: usize,
        m_slot: usize,
    ) -> Self {
        Self::for_move_impl(
            cb, table, att_side, att_slot, def_side, def_slot, m_slot, false,
        )
    }

    #[inline]
    #[allow(clippy::too_many_arguments)]
    fn for_move_impl(
        cb: &CompactBattle,
        table: &MatchTable,
        att_side: usize,
        att_slot: usize,
        def_side: usize,
        def_slot: usize,
        m_slot: usize,
        grid_mode: bool,
    ) -> Self {
        let att_idx = att_side * 3 + att_slot;
        let def_idx = def_side * 3 + def_slot;
        let mspec = &table.move_specs[att_idx][m_slot];
        let att_mon = &cb.mons[att_idx];
        let def_mon = &cb.mons[def_idx];
        let att_spec = &table.mons[att_idx];
        let def_spec = &table.mons[def_idx];

        let base_power = if mspec.var_bp_kind == 4 {
            let att_hp = if grid_mode {
                att_spec.maxhp / 2
            } else {
                att_mon.hp()
            };
            let ratio =
                ((att_hp as f64 * 48.0 / att_spec.maxhp.max(1) as f64).floor() as i32).max(1);
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

        let is_super_fang = mspec.special_kind == 15;
        if mspec.fixed_damage == 0 && !is_super_fang && (mspec.is_status || base_power <= 0) {
            return Self::zero();
        }
        let def_foresight = (def_mon.vol_flags & vol_bit::FORESIGHT) != 0;
        let (type_mod, is_immune) =
            table.effectiveness_ext(mspec.move_type, def_spec, def_foresight);
        if is_immune {
            return Self::zero();
        }

        let mut p_hit =
            cb.hit_probability(table, att_side, att_slot, def_side, def_slot, m_slot) as f32;
        if def_mon.item != 0 && def_spec.is_brightpowder && !mspec.always_hit {
            p_hit *= 235.0 / 256.0;
        }
        if p_hit <= 0.0 {
            return Self::zero();
        }

        if mspec.fixed_damage != 0 || is_super_fang {
            let fdmg = if is_super_fang {
                let d_hp = if grid_mode {
                    def_spec.maxhp / 2
                } else {
                    def_mon.hp()
                };
                (d_hp / 2).max(1)
            } else if mspec.fixed_damage < 0 {
                att_spec.level as i16
            } else {
                mspec.fixed_damage
            };
            let mut k = Self {
                pts: [(0, 0.0); 16],
                len: 0,
                max_dmg: 0,
                mean_dmg: 0.0,
            };
            if p_hit < 1.0 {
                k.push_mass(0, 1.0 - p_hit);
            }
            k.push_mass(fdmg, p_hit);
            return k;
        }

        let hits_mult = (mspec.multihit_mean_x2 as f32 * 0.5).max(1.0);
        const CRIT_DEN: [u32; 6] = [0, 16, 8, 4, 3, 2];
        let scope_bonus = ((att_mon.item != 0) & att_spec.is_scope_lens) as usize;
        let c_stage = (mspec.crit_ratio as usize + scope_bonus).min(5);
        let p_crit = if mspec.special_kind != 13 && c_stage > 0 {
            1.0 / (CRIT_DEN[c_stage] as f32)
        } else {
            0.0
        };

        let compute_base = |is_crit: bool| -> i32 {
            let atk_stat = if mspec.is_physical { 0 } else { 2 };
            let def_stat = if mspec.is_physical { 1 } else { 3 };
            let ignore_boosts = is_crit & (att_mon.boosts[atk_stat] <= def_mon.boosts[def_stat]);
            let mut atk =
                cb.mon_stat(table, att_side, att_slot, atk_stat, ignore_boosts, ignore_boosts) as i32;
            let mut def =
                cb.mon_stat(table, def_side, def_slot, def_stat, ignore_boosts, false) as i32;
            if (atk | def) >= 256 {
                atk = (atk >> 2).max(1);
                def = (def >> 2).max(1);
            }
            if mspec.is_selfdestruct && def_stat == 1 {
                def = (def / 2).max(1);
            }
            let level = att_spec.level as i32;
            let mut d = ((level * 2 / 5 + 2) * base_power * atk / def.max(1)) / 50;
            if is_crit {
                d *= 2;
            }
            if att_mon.item != 0 && att_spec.item_boost_type == mspec.move_type {
                d = d * 110 / 100;
            }
            d = d.min(997) + 2;
            if cb.weather_turns > 0 {
                if cb.weather_kind == CompactWeather::Rain as u8 {
                    if mspec.is_water {
                        d = d * 3 / 2;
                    } else if mspec.is_fire || mspec.special_kind == 23 {
                        d /= 2;
                    }
                } else if cb.weather_kind == CompactWeather::Sun as u8 {
                    if mspec.is_fire {
                        d = d * 3 / 2;
                    } else if mspec.is_water {
                        d /= 2;
                    }
                }
            }
            if (att_spec.type_mask & (1u32 << (mspec.move_type & 31))) != 0 {
                d += d / 2;
            }
            if type_mod > 0 {
                d <<= type_mod as u32;
            } else if type_mod < 0 {
                d >>= (-type_mod) as u32;
            }
            d.max(1)
        };

        let base_norm = compute_base(false);
        let base_crit = if p_crit > 0.0 {
            compute_base(true)
        } else {
            base_norm
        };
        const ROLL_NUMS: [i32; 4] = [221, 231, 242, 253];
        let mut k = Self {
            pts: [(0, 0.0); 16],
            len: 0,
            max_dmg: 0,
            mean_dmg: 0.0,
        };
        if p_hit < 1.0 {
            k.push_mass(0, 1.0 - p_hit);
        }
        let p_norm_roll = p_hit * (1.0 - p_crit) * 0.25;
        for &r in &ROLL_NUMS {
            let dmg = (((base_norm * r / 255).max(1) as f32) * hits_mult).round() as i16;
            k.push_mass(dmg, p_norm_roll);
        }
        if p_crit > 0.0 {
            let p_crit_roll = p_hit * p_crit * 0.25;
            for &r in &ROLL_NUMS {
                let dmg = (((base_crit * r / 255).max(1) as f32) * hits_mult).round() as i16;
                k.push_mass(dmg, p_crit_roll);
            }
        }
        k
    }

    /// Build the Gen 2 `Struggle` (`わるあがき`) 1D damage kernel (`BP = 50`, typeless physical,
    /// no STAB, no type effectiveness, `1/4` dealt-damage recoil min `1 HP`) and return
    /// `(dmg_kernel, mean_recoil_hp)`.
    #[inline]
    pub fn struggle(
        cb: &CompactBattle,
        table: &MatchTable,
        att_side: usize,
        att_slot: usize,
        def_side: usize,
        def_slot: usize,
    ) -> (Self, f32) {
        let att_idx = att_side * 3 + att_slot;
        let def_idx = def_side * 3 + def_slot;
        let att_mon = &cb.mons[att_idx];
        let def_mon = &cb.mons[def_idx];
        let att_spec = &table.mons[att_idx];
        let def_spec = &table.mons[def_idx];

        let def_foresight = (def_mon.vol_flags & vol_bit::FORESIGHT) != 0;
        let brightpowder = (def_mon.item != 0) & def_spec.is_brightpowder;
        let thresh = lookup_accuracy_threshold(
            100,
            att_mon.boosts[5],
            def_mon.boosts[6],
            false,
            false,
            def_foresight,
            brightpowder,
        );
        let p_hit = if thresh == 255 {
            1.0f32
        } else {
            thresh as f32 / 256.0
        };
        if p_hit <= 0.0 {
            return (Self::zero(), 0.0);
        }

        const CRIT_DEN: [u32; 6] = [0, 16, 8, 4, 3, 2];
        let scope_bonus = ((att_mon.item != 0) & att_spec.is_scope_lens) as usize;
        let c_stage = (1 + scope_bonus).min(5);
        let p_crit = 1.0 / (CRIT_DEN[c_stage] as f32);

        let compute_base = |is_crit: bool| -> i32 {
            let ignore_boosts = is_crit & (att_mon.boosts[0] <= def_mon.boosts[1]);
            let mut atk =
                cb.mon_stat(table, att_side, att_slot, 0, ignore_boosts, ignore_boosts) as i32;
            let mut def = cb.mon_stat(table, def_side, def_slot, 1, ignore_boosts, false) as i32;
            if (atk | def) >= 256 {
                atk = (atk >> 2).max(1);
                def = (def >> 2).max(1);
            }
            let level = att_spec.level as i32;
            let mut d = ((level * 2 / 5 + 2) * 50 * atk / def.max(1)) / 50;
            if is_crit {
                d *= 2;
            }
            d.clamp(1, 997) + 2
        };

        let base_norm = compute_base(false);
        let base_crit = compute_base(true);
        const ROLL_NUMS: [i32; 4] = [221, 231, 242, 253];
        let mut k = Self {
            pts: [(0, 0.0); 16],
            len: 0,
            max_dmg: 0,
            mean_dmg: 0.0,
        };
        if p_hit < 1.0 {
            k.push_mass(0, 1.0 - p_hit);
        }
        let mut mean_recoil = 0.0f32;
        let p_norm_roll = p_hit * (1.0 - p_crit) * 0.25;
        for &r in &ROLL_NUMS {
            let dmg = (base_norm * r / 255).max(1) as i16;
            k.push_mass(dmg, p_norm_roll);
            mean_recoil += p_norm_roll * ((dmg / 4).max(1) as f32);
        }
        let p_crit_roll = p_hit * p_crit * 0.25;
        for &r in &ROLL_NUMS {
            let dmg = (base_crit * r / 255).max(1) as i16;
            k.push_mass(dmg, p_crit_roll);
            mean_recoil += p_crit_roll * ((dmg / 4).max(1) as f32);
        }
        (k, mean_recoil)
    }
}

#[derive(Clone, Copy)]
struct StrugglePairStats {
    delta: [f64; 2],
    inc_str: [f64; 2],
    self_rec: [f64; 2],
    net_res_drain: [f64; 2],
    beta_spe: [f64; 2],
}

impl StrugglePairStats {
    #[inline]
    fn build(
        cb: &CompactBattle,
        table: &MatchTable,
        s0: usize,
        slot0: usize,
        s1: usize,
        slot1: usize,
    ) -> Self {
        let idx = [s0 * 3 + slot0, s1 * 3 + slot1];
        let (k01, rec0) = DmgKernel1D::struggle(cb, table, s0, slot0, s1, slot1);
        let (k10, rec1) = DmgKernel1D::struggle(cb, table, s1, slot1, s0, slot0);
        let maxhp = [
            table.mons[idx[0]].maxhp.max(1) as f64,
            table.mons[idx[1]].maxhp.max(1) as f64,
        ];
        let mut p_act = [1.0f64; 2];
        let mut net_res_drain = [0.0f64; 2];
        for i in 0..2 {
            let m = &cb.mons[idx[i]];
            let fm = &cb.mons[idx[1 - i]];
            let sp = &table.mons[idx[i]];
            let fsp = &table.mons[idx[1 - i]];
            let miracle = m.item != 0 && sp.is_miracleberry;
            let st_raw = m.status_raw();
            let is_frz = st_raw == Status::Frz as u8 && !miracle;
            let is_par = st_raw == Status::Par as u8 && !miracle;
            p_act[i] = if is_frz {
                0.0
            } else if is_par {
                0.75
            } else {
                1.0
            };
            let lefties = if m.item != 0 && sp.is_leftovers {
                (sp.maxhp.max(1) / 16).max(1) as f64 / maxhp[i]
            } else {
                0.0
            };
            let leech_drain = if (m.vol_flags & vol_bit::LEECH_SEED) != 0 {
                (sp.maxhp.max(1) / 8).max(1) as f64 / maxhp[i]
            } else {
                0.0
            };
            let leech_gain = if (fm.vol_flags & vol_bit::LEECH_SEED) != 0 {
                (fsp.maxhp.max(1) / 8).max(1) as f64 / maxhp[i]
            } else {
                0.0
            };
            let cnt = m.status_counter();
            let is_tox = st_raw == Status::Tox as u8
                || ((cnt & 0x10) != 0 && (st_raw == Status::Psn as u8 || st_raw == Status::Brn as u8));
            let stat_dot = if is_tox {
                4.0 * ((sp.maxhp.max(1) / 16).max(1) as f64) / maxhp[i]
            } else if st_raw == Status::Brn as u8 || st_raw == Status::Psn as u8 {
                (sp.maxhp.max(1) / 8).max(1) as f64 / maxhp[i]
            } else {
                0.0
            };
            let curse_dot = if (m.vol_flags & vol_bit::CURSE) != 0 {
                (sp.maxhp.max(1) / 4).max(1) as f64 / maxhp[i]
            } else {
                0.0
            };
            net_res_drain[i] = leech_drain + stat_dot + curse_dot - lefties - leech_gain;
        }
        let inc_str = [
            p_act[1] * (k10.mean_dmg as f64) / maxhp[0],
            p_act[0] * (k01.mean_dmg as f64) / maxhp[1],
        ];
        let self_rec = [
            p_act[0] * (rec0 as f64) / maxhp[0],
            p_act[1] * (rec1 as f64) / maxhp[1],
        ];
        let delta = [
            inc_str[0] + self_rec[0] + net_res_drain[0],
            inc_str[1] + self_rec[1] + net_res_drain[1],
        ];
        let spd0 = cb.mon_action_speed(table, s0, slot0);
        let spd1 = cb.mon_action_speed(table, s1, slot1);
        let b0 = if spd0 > spd1 {
            0.25
        } else if spd0 < spd1 {
            -0.25
        } else {
            0.0
        };
        Self {
            delta,
            inc_str,
            self_rec,
            net_res_drain,
            beta_spe: [b0, -b0],
        }
    }

    #[inline]
    fn eval(&self, h0: f64, h1: f64) -> f64 {
        match (h0 <= 1e-7, h1 <= 1e-7) {
            (true, true) => return 0.0,
            (true, false) => return -0.5,
            (false, true) => return 0.5,
            (false, false) => {}
        }
        let d0 = self.delta[0];
        let d1 = self.delta[1];
        if d0 <= 1e-7 && d1 <= 1e-7 {
            0.0
        } else if d0 <= 1e-7 && d1 > 1e-7 {
            0.5
        } else if d1 <= 1e-7 && d0 > 1e-7 {
            -0.5
        } else {
            let tau0 = h0 / d0 + self.beta_spe[0];
            let tau1 = h1 / d1 + self.beta_spe[1];
            let diff = tau0 - tau1;
            0.5 * diff / (diff * diff + 2.25).sqrt()
        }
    }
}

/// Evaluate the centered Gen 2 `Struggle` + `Leftovers` 4-quadrant terminal value
/// $u_{\text{str}} \in [-0.5, +0.5]$ ($V = 0.5 + u_{\text{str}}$).
#[inline]
pub fn struggle_terminal_1v1(
    cb: &CompactBattle,
    table: &MatchTable,
    s0: usize,
    slot0: usize,
    h0_frac: f64,
    s1: usize,
    slot1: usize,
    h1_frac: f64,
) -> f64 {
    StrugglePairStats::build(cb, table, s0, slot0, s1, slot1).eval(h0_frac, h1_frac)
}

/// Evaluate the diffused `Struggle` arrival clock payoff $u_{\text{III}} \in [-0.5, +0.5]$
/// from remaining PP horizons $(\mu_{T, 0}, \mu_{T, 1})$ and pre-Struggle HP fractions $(\bar{h}_0, \bar{h}_1)$.
#[inline]
fn struggle_arrival_payoff(
    str_stats: &StrugglePairStats,
    mu0: f64,
    mu1: f64,
    h0_base: f64,
    h1_base: f64,
    extra_atk_rate_0_to_1: f64,
    extra_atk_rate_1_to_0: f64,
    side0_has_heal_buffer: bool,
    side1_has_heal_buffer: bool,
) -> f64 {
    let drain_1_when_1_str =
        (extra_atk_rate_0_to_1 + str_stats.self_rec[1] + str_stats.net_res_drain[1]).max(0.0);
    let drain_0_when_0_str =
        (extra_atk_rate_1_to_0 + str_stats.self_rec[0] + str_stats.net_res_drain[0]).max(0.0);
    if str_stats.delta[0] <= 1e-7
        && str_stats.delta[1] <= 1e-7
        && drain_1_when_1_str <= 1e-7
        && drain_0_when_0_str <= 1e-7
    {
        return 0.0;
    }

    let var0 = 2.25 + 0.25 * mu0.max(0.0);
    let var1 = 2.25 + 0.25 * mu1.max(0.0);
    let z = (mu0 - mu1) / (var0 + var1).sqrt();
    let q0 = norm_cdf_sym(z);

    let eval_asym = |dt: f64,
                     h_adv: f64,
                     h_str: f64,
                     drain_adv_raw: f64,
                     drain_str: f64,
                     adv_heals: bool|
     -> (f64, f64) {
        if dt <= 1e-6 {
            return (h_adv, h_str);
        }
        let drain_adv = if adv_heals {
            0.0
        } else {
            drain_adv_raw.max(0.0)
        };
        let t_str = if drain_str > 1e-7 {
            h_str / drain_str
        } else {
            f64::INFINITY
        };
        let t_adv = if drain_adv > 1e-7 {
            h_adv / drain_adv
        } else {
            f64::INFINITY
        };
        let step = dt.min(t_str).min(t_adv);
        let next_adv = (h_adv - step * drain_adv).clamp(0.0, 1.0);
        let next_str = (h_str - step * drain_str).clamp(0.0, 1.0);
        if t_str < dt && t_str < t_adv {
            (next_adv.max(1e-3), 0.0)
        } else if t_adv < dt && t_adv < t_str {
            (0.0, next_str.max(1e-3))
        } else {
            (next_adv, next_str)
        }
    };

    let dt0 = (mu0 - mu1).max(0.0);
    let drain_0_pre = str_stats.inc_str[0] + str_stats.net_res_drain[0];
    let (h0_b0, h1_b0) = eval_asym(
        dt0,
        h0_base,
        h1_base,
        drain_0_pre,
        drain_1_when_1_str,
        side0_has_heal_buffer,
    );
    let u_b0 = str_stats.eval(h0_b0, h1_b0);

    let dt1 = (mu1 - mu0).max(0.0);
    let drain_1_pre = str_stats.inc_str[1] + str_stats.net_res_drain[1];
    let (h1_b1, h0_b1) = eval_asym(
        dt1,
        h1_base,
        h0_base,
        drain_1_pre,
        drain_0_when_0_str,
        side1_has_heal_buffer,
    );
    let u_b1 = str_stats.eval(h0_b1, h1_b1);

    q0 * u_b0 + (1.0 - q0) * u_b1
}

#[derive(Clone, Copy)]
struct SideRaceSchedule {
    /// Ordered awake move phases `(dmg_kernel, awake_turn_budget)` before `Struggle`.
    phases: [(DmgKernel1D, u16); 4],
    num_phases: usize,
    /// Total awake/SleepTalk moves before `Struggle` (`u16::MAX` when frozen).
    turns_before_struggle: f64,
    slp_kern: DmgKernel1D,
    str_kern: DmgKernel1D,
    str_recoil_hp: i16,
}

impl SideRaceSchedule {
    #[inline]
    fn build(
        cb: &CompactBattle,
        table: &MatchTable,
        att_side: usize,
        att_slot: usize,
        def_side: usize,
        def_slot: usize,
    ) -> Self {
        let att_idx = att_side * 3 + att_slot;
        let def_idx = def_side * 3 + def_slot;
        let att_mon = &cb.mons[att_idx];
        let att_spec = &table.mons[att_idx];
        let def_hp = cb.mons[def_idx].hp().max(1);
        let def_maxhp = table.mons[def_idx].maxhp.max(1);
        let num_moves = att_spec.num_moves as usize;
        let is_disabled = (att_mon.vol_flags & vol_bit::DISABLE) != 0;
        let dis_slot = ((att_mon.slot_refs >> 4) & 0x03) as usize;
        let last_mon = cb.sides[att_side].alive_mask.count_ones() <= 1;

        let miracle = att_mon.item != 0 && att_spec.is_miracleberry;
        let berry = att_mon.item != 0 && (att_spec.is_mintberry || att_spec.is_miracleberry);
        let st_raw = att_mon.status_raw();
        let is_frz = st_raw == Status::Frz as u8 && !miracle;
        let is_par = st_raw == Status::Par as u8 && !miracle;
        let p_deduct = if is_frz {
            0.0f64
        } else if is_par {
            0.75f64
        } else {
            1.0f64
        };
        let slp_rem = if st_raw == Status::Slp as u8 && !berry {
            att_mon.status_counter().saturating_sub(1) as usize
        } else {
            0
        };
        let mut has_st = false;
        let mut dmg_slots = [(0usize, 0.0f32, DmgKernel1D::zero(), 0u8); 4];
        let mut n_dmg = 0usize;
        let mut aux_pp: u16 = 0;

        for m_i in 0..num_moves {
            let pp = att_mon.pp[m_i];
            if pp == 0 || (is_disabled && m_i == dis_slot) {
                continue;
            }
            let mspec = &table.move_specs[att_idx][m_i];
            if last_mon && mspec.is_selfdestruct {
                continue;
            }
            if mspec.special_kind == 10 {
                has_st = true;
            }
            let kern = DmgKernel1D::for_move(cb, table, att_side, att_slot, def_side, def_slot, m_i);
            if kern.max_dmg > 0 {
                let p_ko = kern.ko_prob(def_hp);
                let frac = (kern.mean_dmg / (def_hp as f32)).min(1.0);
                let raw_frac = kern.mean_dmg / (def_maxhp as f32);
                let score = p_ko + frac + 1e-3 * raw_frac;
                dmg_slots[n_dmg] = (m_i, score, kern, pp);
                n_dmg += 1;
            } else {
                aux_pp += pp as u16;
            }
        }
        // Insertion sort descending by score (4 elements max)
        for i in 1..n_dmg {
            let cur = dmg_slots[i];
            let mut j = i;
            while j > 0 && cur.1 > dmg_slots[j - 1].1 {
                dmg_slots[j] = dmg_slots[j - 1];
                j -= 1;
            }
            dmg_slots[j] = cur;
        }

        let mut phases = [(DmgKernel1D::zero(), 0u16); 4];
        let mut num_phases = 0usize;
        let mut total_awake_turns = 0.0f64;
        if p_deduct <= 0.0 {
            if n_dmg > 0 {
                phases[0] = (dmg_slots[0].2, 1000);
                num_phases = 1;
            } else if aux_pp > 0 {
                phases[0] = (DmgKernel1D::zero(), 1000);
                num_phases = 1;
            }
            total_awake_turns = if n_dmg > 0 || aux_pp > 0 { 1e6 } else { 0.0 };
        } else {
            for &(_, _, kern, pp) in &dmg_slots[..n_dmg] {
                let t_f = (pp as f64) / p_deduct;
                total_awake_turns += t_f;
                let n_turns = t_f.ceil().clamp(1.0, 1000.0) as u16;
                if num_phases < 4 {
                    phases[num_phases] = (kern, n_turns);
                    num_phases += 1;
                } else {
                    phases[3].1 = phases[3].1.saturating_add(n_turns);
                }
            }
            if aux_pp > 0 {
                let st_spent = if slp_rem > 0 && has_st {
                    (slp_rem as u16).min(aux_pp)
                } else {
                    0
                };
                let rem_aux = aux_pp - st_spent;
                if rem_aux > 0 {
                    let t_f = (rem_aux as f64) / p_deduct;
                    total_awake_turns += t_f;
                    let n_turns = t_f.ceil().clamp(1.0, 1000.0) as u16;
                    if num_phases < 4 {
                        phases[num_phases] = (DmgKernel1D::zero(), n_turns);
                        num_phases += 1;
                    } else {
                        phases[3].1 = phases[3].1.saturating_add(n_turns);
                    }
                }
            }
        }

        let slp_kern = if slp_rem > 0 && has_st {
            DmgKernel1D::sleep_talk(cb, table, att_side, att_slot, def_side, def_slot)
        } else {
            DmgKernel1D::zero()
        };
        let turns_before_struggle = (slp_rem as f64) + total_awake_turns;
        let (str_kern, str_recoil_hp) = if turns_before_struggle < 32.0 {
            let (sk, mean_rec_f32) =
                DmgKernel1D::struggle(cb, table, att_side, att_slot, def_side, def_slot);
            let rec_hp = if sk.max_dmg > 0 {
                (mean_rec_f32.round() as i16).max(1)
            } else {
                0
            };
            (sk, rec_hp)
        } else {
            (DmgKernel1D::zero(), 0)
        };

        Self {
            phases,
            num_phases,
            turns_before_struggle,
            slp_kern,
            str_kern,
            str_recoil_hp,
        }
    }

    #[inline]
    fn kernel_for_awake_step(&self, awake_idx_0based: u16) -> (&DmgKernel1D, bool) {
        let mut rem = awake_idx_0based;
        for i in 0..self.num_phases {
            let dur = self.phases[i].1;
            if rem < dur {
                return (&self.phases[i].0, false);
            }
            rem -= dur;
        }
        (&self.str_kern, true)
    }
}

/// Propagate `def_side`'s 1D survival curve `S(0..=32)` and conditional mean HP fraction at `T = 32`
/// under `foe_sched` (incoming damage) and `own_sched` (own Struggle recoil after own PP depletes).
#[allow(clippy::too_many_arguments)]
#[inline]
fn survival_curve_32_piecewise(
    cb: &CompactBattle,
    table: &MatchTable,
    att_side: usize,
    att_slot: usize,
    def_side: usize,
    def_slot: usize,
    foe_sched: &SideRaceSchedule,
    own_sched: &SideRaceSchedule,
) -> ([f64; 33], usize, f64) {
    let att_idx = att_side * 3 + att_slot;
    let def_idx = def_side * 3 + def_slot;
    let start_hp = cb.mons[def_idx].hp();
    if start_hp <= 0 {
        return ([0.0; 33], 0, 0.0);
    }
    let maxhp = table.mons[def_idx].maxhp.max(1);
    let def_mon = &cb.mons[def_idx];
    let att_mon = &cb.mons[att_idx];
    let def_spec = &table.mons[def_idx];
    let att_spec = &table.mons[att_idx];

    let att_berry = att_mon.item != 0 && (att_spec.is_mintberry || att_spec.is_miracleberry);
    let att_miracle = att_mon.item != 0 && att_spec.is_miracleberry;
    let mut att_slp = if att_mon.status_raw() == Status::Slp as u8 && !att_berry {
        att_mon.status_counter().saturating_sub(1)
    } else {
        0
    };
    let def_berry = def_mon.item != 0 && (def_spec.is_mintberry || def_spec.is_miracleberry);
    let def_miracle = def_mon.item != 0 && def_spec.is_miracleberry;
    let mut def_slp = if def_mon.status_raw() == Status::Slp as u8 && !def_berry {
        def_mon.status_counter().saturating_sub(1)
    } else {
        0
    };
    let def_frz = def_mon.status_raw() == Status::Frz as u8 && !def_miracle;
    let def_par = def_mon.status_raw() == Status::Par as u8 && !def_miracle;

    let to_b = |hp: i16| -> usize {
        if hp <= 0 {
            0
        } else {
            (((hp as i32 * B32 as i32 + maxhp as i32 - 1) / maxhp as i32) as usize).clamp(1, B32)
        }
    };
    let b0 = to_b(start_hp.min(maxhp));
    let b_to_hp = |b: usize, step: usize| -> i16 {
        if b == 0 {
            0
        } else if step == 1 && b == b0 {
            start_hp.min(maxhp)
        } else {
            (((b as i32 * maxhp as i32) + (B32 as i32 / 2)) / B32 as i32)
                .clamp(1, maxhp as i32) as i16
        }
    };

    let mut dist = [0.0f32; B32 + 1];
    dist[b0] = 1.0;
    let mut min_b = b0;
    let mut max_b = b0;

    let lefties: i16 = if def_mon.item != 0 && def_spec.is_leftovers {
        (maxhp / 16).max(1)
    } else {
        0
    };
    let leech_drain: i16 = if (def_mon.vol_flags & vol_bit::LEECH_SEED) != 0 {
        (maxhp / 8).max(1)
    } else {
        0
    };
    let leech_gain: i16 = if (att_mon.vol_flags & vol_bit::LEECH_SEED) != 0 {
        (att_spec.maxhp.max(1) / 8).max(1)
    } else {
        0
    };
    let def_st = def_mon.status_raw();
    let cnt_packed = def_mon.status_counter();
    let is_tox = def_st == Status::Tox as u8
        || ((cnt_packed & 0x10) != 0 && (def_st == Status::Psn as u8 || def_st == Status::Brn as u8));
    let mut tox_counter = (cnt_packed & 0x0F) as i16;
    let psn_brn_dmg: i16 = if def_st == Status::Brn as u8 || def_st == Status::Psn as u8 {
        (maxhp / 8).max(1)
    } else {
        0
    };
    let curse_dmg: i16 = if (def_mon.vol_flags & vol_bit::CURSE) != 0 {
        (maxhp / 4).max(1)
    } else {
        0
    };
    let mut weather_rem = cb.weather_turns;
    let sand_dmg: i16 =
        if cb.weather_kind == 3 && (def_spec.type_mask & table.sandstorm_immune_mask) == 0 {
            (maxhp / 8).max(1)
        } else {
            0
        };

    let mut def_conf = if (def_mon.vol_flags & vol_bit::CONFUSION) != 0 && !def_miracle {
        def_mon.timers[0].saturating_sub(1)
    } else {
        0
    };
    let def_conf_dmg = if def_conf > 0 {
        let def_atk = cb.mon_stat(table, def_side, def_slot, 0, false, false) as i32;
        let def_def = cb.mon_stat(table, def_side, def_slot, 1, false, false) as i32;
        (((def_spec.level as i32 * 2 / 5 + 2) * 40 * def_atk / def_def.max(1)) / 50 + 2).max(1)
            as i16
    } else {
        0
    };

    let mut p_still_frz = if att_mon.status_raw() == Status::Frz as u8 && !att_miracle {
        1.0f32
    } else {
        0.0f32
    };
    let att_par = att_mon.status_raw() == Status::Par as u8 && !att_miracle;
    let mut att_conf = if (att_mon.vol_flags & vol_bit::CONFUSION) != 0 && !att_miracle {
        att_mon.timers[0].saturating_sub(1)
    } else {
        0
    };
    let mut att_rech = (att_mon.slot_refs & 0x80) != 0;
    let mut def_rech = (def_mon.slot_refs & 0x80) != 0;
    let mut p_sub_up = if (def_mon.vol_flags & vol_bit::SUBSTITUTE) != 0 && def_mon.hp_pool[1] > 0 {
        1.0f32
    } else {
        0.0f32
    };
    let mut rem_sub_hp = if p_sub_up > 0.0 {
        def_mon.sub_hp().max(1) as f32
    } else {
        0.0f32
    };

    let mut att_awake_step = 0u16;
    let mut def_awake_step = 0u16;
    let mut out = [1.0f64; 33];
    let mut steps_done = 0usize;

    for step in 1..=32usize {
        if min_b > max_b {
            for st in step..=32 {
                out[st] = 0.0;
            }
            break;
        }
        steps_done = step;
        let mut next = [0.0f32; B32 + 1];
        next[0] = dist[0];
        let mut next_min = B32 + 1;
        let mut next_max = 0usize;

        let status_dot = if is_tox {
            tox_counter = (tox_counter + 1).min(15);
            (maxhp / 16).max(1) * tox_counter
        } else {
            psn_brn_dmg
        };
        let sand_this_turn = if weather_rem > 0 {
            weather_rem -= 1;
            sand_dmg
        } else {
            0
        };
        let res_this_turn =
            lefties + leech_gain - leech_drain - status_dot - curse_dmg - sand_this_turn;

        let p_def_selfhit = if def_conf > 0 {
            def_conf -= 1;
            0.5f32
        } else {
            0.0f32
        };

        // Defender's own Struggle recoil once defender's PP schedule is exhausted
        let mut own_str_recoil = 0i16;
        let mut p_own_str = 0.0f32;
        if def_slp > 0 {
            def_slp -= 1;
        } else if def_rech {
            def_rech = false;
        } else if !def_frz {
            let (_, is_own_str) = own_sched.kernel_for_awake_step(def_awake_step);
            def_awake_step = def_awake_step.saturating_add(1);
            if is_own_str && own_sched.str_recoil_hp > 0 {
                own_str_recoil = own_sched.str_recoil_hp;
                p_own_str = if def_par { 0.75 } else { 1.0 };
            }
        }

        let mut p_att = 1.0f32;
        if att_rech {
            att_rech = false;
            p_att = 0.0;
        } else if p_still_frz > 0.0 {
            p_still_frz *= 231.0 / 256.0;
            p_att *= 1.0 - p_still_frz;
        } else {
            if att_conf > 0 {
                att_conf -= 1;
                p_att *= 0.5;
            }
            if att_par {
                p_att *= 0.75;
            }
        }

        let kern = if att_slp > 0 {
            att_slp -= 1;
            &foe_sched.slp_kern
        } else {
            let (k_ref, _) = foe_sched.kernel_for_awake_step(att_awake_step);
            if p_att > 0.0 {
                att_awake_step = att_awake_step.saturating_add(1);
            }
            k_ref
        };

        let p_sub_now = p_sub_up;
        if p_sub_now > 0.0 && p_att > 0.0 && kern.max_dmg > 0 {
            let mut p_break = 0.0f32;
            let mut exp_chip = 0.0f32;
            for i in 0..kern.len {
                let (dmg, prob) = kern.pts[i];
                if dmg as f32 >= rem_sub_hp {
                    p_break += prob;
                } else if dmg > 0 {
                    exp_chip += prob * (dmg as f32);
                }
            }
            rem_sub_hp = (rem_sub_hp - p_att * exp_chip).max(1.0);
            p_sub_up = (p_sub_up * (1.0 - p_att * p_break)).clamp(0.0, 1.0);
        }

        for b in min_b..=max_b {
            let p_state = dist[b];
            if p_state <= 1e-7 {
                continue;
            }
            let raw_hp = b_to_hp(b, step);

            let mut branches = [(0i16, 0.0f32); 3];
            let mut n_br = 0usize;
            if p_def_selfhit > 0.0 {
                branches[n_br] = ((raw_hp - def_conf_dmg).max(0), p_def_selfhit);
                n_br += 1;
            }
            let w_rem = 1.0 - p_def_selfhit;
            if p_own_str > 0.0 {
                branches[n_br] = ((raw_hp - own_str_recoil).max(0), w_rem * p_own_str);
                n_br += 1;
                if p_own_str < 1.0 {
                    branches[n_br] = (raw_hp, w_rem * (1.0 - p_own_str));
                    n_br += 1;
                }
            } else {
                branches[n_br] = (raw_hp, w_rem);
                n_br += 1;
            }

            for &(hp_after_self, w_self) in &branches[..n_br] {
                if w_self <= 0.0 {
                    continue;
                }
                let pm = p_state * w_self;
                if hp_after_self <= 0 {
                    next[0] += pm;
                    continue;
                }
                if p_att < 1.0 {
                    let p_no_act = pm * (1.0 - p_att);
                    let final_hp = (hp_after_self + res_this_turn).clamp(0, maxhp);
                    let tb = to_b(final_hp);
                    next[tb] += p_no_act;
                    if tb > 0 && p_no_act > 0.0 {
                        if tb < next_min {
                            next_min = tb;
                        }
                        if tb > next_max {
                            next_max = tb;
                        }
                    }
                }
                if p_att > 0.0 {
                    let pa = pm * p_att;
                    for i in 0..kern.len {
                        let (dmg, prob) = kern.pts[i];
                        if dmg > 0 && p_sub_now > 0.0 {
                            let final_sub_hp = (hp_after_self + res_this_turn).clamp(0, maxhp);
                            let tb_sub = to_b(final_sub_hp);
                            let add_sub = pa * prob * p_sub_now;
                            next[tb_sub] += add_sub;
                            if tb_sub > 0 && add_sub > 0.0 {
                                if tb_sub < next_min {
                                    next_min = tb_sub;
                                }
                                if tb_sub > next_max {
                                    next_max = tb_sub;
                                }
                            }
                            let p_unsub = 1.0 - p_sub_now;
                            if p_unsub > 1e-6 {
                                let post_hit = (hp_after_self - dmg).max(0);
                                let final_hp = if post_hit <= 0 {
                                    0
                                } else {
                                    (post_hit + res_this_turn).clamp(0, maxhp)
                                };
                                let tb = to_b(final_hp);
                                let add = pa * prob * p_unsub;
                                next[tb] += add;
                                if tb > 0 && add > 0.0 {
                                    if tb < next_min {
                                        next_min = tb;
                                    }
                                    if tb > next_max {
                                        next_max = tb;
                                    }
                                }
                            }
                        } else {
                            let post_hit = (hp_after_self - dmg).max(0);
                            let final_hp = if post_hit <= 0 {
                                0
                            } else {
                                (post_hit + res_this_turn).clamp(0, maxhp)
                            };
                            let tb = to_b(final_hp);
                            let add = pa * prob;
                            next[tb] += add;
                            if tb > 0 && add > 0.0 {
                                if tb < next_min {
                                    next_min = tb;
                                }
                                if tb > next_max {
                                    next_max = tb;
                                }
                            }
                        }
                    }
                }
            }
        }
        dist = next;
        min_b = next_min;
        max_b = next_max;
        out[step] = if min_b > max_b {
            0.0
        } else {
            (1.0 - (dist[0] as f64)).clamp(0.0, 1.0)
        };
    }

    if min_b > max_b {
        return (out, steps_done, 0.0);
    }
    let mut alive_mass = 0.0f32;
    let mut weighted_b = 0.0f32;
    for b in min_b..=max_b {
        alive_mass += dist[b];
        weighted_b += dist[b] * (b as f32 / B32 as f32);
    }
    let mean_hp_frac = if alive_mass > 1e-6 {
        (weighted_b / alive_mass) as f64
    } else {
        0.0
    };
    (out, steps_done, mean_hp_frac)
}

/// Compute `[S(0), S(1), S(2), S(3)]`, the probability that `def_side`'s active mon
/// stays strictly above `0` HP after $k = 0, 1, 2, 3$ turns of `att_side`'s attack + residuals:
/// $S(k) = (\mathbf{1}^\top T^k \mathbf{e}_{h_0})$.
#[inline]
pub fn survival_curve_4(
    cb: &CompactBattle,
    table: &MatchTable,
    att_side: usize,
    def_side: usize,
    start_hp: i16,
) -> [f64; 4] {
    let (s, _) = survival_curve_k_with_mean_hp(cb, table, att_side, def_side, start_hp, 3, false);
    [s[0], s[1], s[2], s[3]]
}

/// Compute `([S(0)..=S(6)], mean_hp_frac_at_6)` via exact 1D Toeplitz convolution $T^k \mathbf{e}_{h_0}$.
#[inline]
fn survival_curve_6_with_mean_hp(
    cb: &CompactBattle,
    table: &MatchTable,
    att_side: usize,
    att_slot: usize,
    def_side: usize,
    def_slot: usize,
    start_hp: i16,
) -> ([f64; 7], f64) {
    survival_curve_k_slots_with_mean_hp(
        cb, table, att_side, att_slot, def_side, def_slot, start_hp, 6, true,
    )
}

#[inline]
fn survival_curve_k_with_mean_hp(
    cb: &CompactBattle,
    table: &MatchTable,
    att_side: usize,
    def_side: usize,
    start_hp: i16,
    max_steps: usize,
    need_mean_hp: bool,
) -> ([f64; 7], f64) {
    let att_slot = cb.sides[att_side].active_idx as usize;
    let def_slot = cb.sides[def_side].active_idx as usize;
    survival_curve_k_slots_with_mean_hp(
        cb,
        table,
        att_side,
        att_slot,
        def_side,
        def_slot,
        start_hp,
        max_steps,
        need_mean_hp,
    )
}

#[allow(clippy::too_many_arguments)]
#[inline]
fn survival_curve_k_slots_with_mean_hp(
    cb: &CompactBattle,
    table: &MatchTable,
    att_side: usize,
    att_slot: usize,
    def_side: usize,
    def_slot: usize,
    start_hp: i16,
    max_steps: usize,
    need_mean_hp: bool,
) -> ([f64; 7], f64) {
    if start_hp <= 0 {
        return ([0.0; 7], 0.0);
    }
    let max_steps = max_steps.min(6);
    if max_steps == 0 && !need_mean_hp {
        return ([1.0; 7], 0.0);
    }
    let att_idx = att_side * 3 + att_slot;
    let def_idx = def_side * 3 + def_slot;
    let maxhp = table.mons[def_idx].maxhp.max(1);

    let def_mon = &cb.mons[def_idx];
    let att_mon = &cb.mons[att_idx];
    let def_spec = &table.mons[def_idx];
    let att_spec = &table.mons[att_idx];

    // Attacker sleep, freeze, paralysis, confusion, recharge
    let att_berry = att_mon.item != 0 && (att_spec.is_mintberry || att_spec.is_miracleberry);
    let att_miracle = att_mon.item != 0 && att_spec.is_miracleberry;
    let mut att_slp = if att_mon.status_raw() == Status::Slp as u8 && !att_berry {
        att_mon.status_counter().saturating_sub(1)
    } else {
        0
    };

    let kern_awake = DmgKernel1D::best_attack(cb, table, att_side, att_slot, def_side, def_slot);
    let kern_slp = if att_slp > 0 {
        DmgKernel1D::sleep_talk(cb, table, att_side, att_slot, def_side, def_slot)
    } else {
        DmgKernel1D::zero()
    };

    let to_b = |hp: i16| -> usize {
        if hp <= 0 {
            0
        } else {
            (((hp as i32 * B32 as i32 + maxhp as i32 - 1) / maxhp as i32) as usize).clamp(1, B32)
        }
    };
    let b0 = to_b(start_hp.min(maxhp));
    let b_to_hp = |b: usize, step: usize| -> i16 {
        if b == 0 {
            0
        } else if step == 1 && b == b0 {
            start_hp.min(maxhp)
        } else {
            (((b as i32 * maxhp as i32) + (B32 as i32 / 2)) / B32 as i32)
                .clamp(1, maxhp as i32) as i16
        }
    };

    let mut dist = [0.0f32; B32 + 1];
    dist[b0] = 1.0;
    let mut min_b = b0;
    let mut max_b = b0;

    let lefties: i16 = if def_mon.item != 0 && def_spec.is_leftovers {
        (maxhp / 16).max(1)
    } else {
        0
    };
    let leech_drain: i16 = if (def_mon.vol_flags & vol_bit::LEECH_SEED) != 0 {
        (maxhp / 8).max(1)
    } else {
        0
    };
    let leech_gain: i16 = if (att_mon.vol_flags & vol_bit::LEECH_SEED) != 0 {
        (att_spec.maxhp.max(1) / 8).max(1)
    } else {
        0
    };
    let def_st = def_mon.status_raw();
    let cnt_packed = def_mon.status_counter();
    let is_tox = def_st == Status::Tox as u8
        || ((cnt_packed & 0x10) != 0 && (def_st == Status::Psn as u8 || def_st == Status::Brn as u8));
    let mut tox_counter = (cnt_packed & 0x0F) as i16;
    let psn_brn_dmg: i16 = if def_st == Status::Brn as u8 || def_st == Status::Psn as u8 {
        (maxhp / 8).max(1)
    } else {
        0
    };
    let curse_dmg: i16 = if (def_mon.vol_flags & vol_bit::CURSE) != 0 {
        (maxhp / 4).max(1)
    } else {
        0
    };
    let mut weather_rem = cb.weather_turns;
    let sand_dmg: i16 =
        if cb.weather_kind == 3 && (def_spec.type_mask & table.sandstorm_immune_mask) == 0 {
            (maxhp / 8).max(1)
        } else {
            0
        };

    // Defender confusion self-hit
    let mut def_conf = if (def_mon.vol_flags & vol_bit::CONFUSION) != 0 {
        def_mon.timers[0].saturating_sub(1)
    } else {
        0
    };
    let def_conf_dmg = if def_conf > 0 {
        let def_atk = cb.mon_stat(table, def_side, def_slot, 0, false, false) as i32;
        let def_def = cb.mon_stat(table, def_side, def_slot, 1, false, false) as i32;
        (((def_spec.level as i32 * 2 / 5 + 2) * 40 * def_atk / def_def.max(1)) / 50 + 2).max(1)
            as i16
    } else {
        0
    };

    let mut p_still_frz = if att_mon.status_raw() == Status::Frz as u8 && !att_miracle {
        1.0f32
    } else {
        0.0f32
    };
    let att_par = att_mon.status_raw() == Status::Par as u8 && !att_miracle;
    let mut att_conf = if (att_mon.vol_flags & vol_bit::CONFUSION) != 0 && !att_miracle {
        att_mon.timers[0].saturating_sub(1)
    } else {
        0
    };
    let mut att_rech = (att_mon.slot_refs & 0x80) != 0;
    let mut p_sub_up = if (def_mon.vol_flags & vol_bit::SUBSTITUTE) != 0 && def_mon.hp_pool[1] > 0 {
        1.0f32
    } else {
        0.0f32
    };
    let mut rem_sub_hp = if p_sub_up > 0.0 {
        def_mon.sub_hp().max(1) as f32
    } else {
        0.0f32
    };

    let mut out = [1.0f64; 7];
    for step in 1..=max_steps {
        if min_b > max_b {
            for st in step..=max_steps {
                out[st] = 0.0;
            }
            break;
        }
        let mut next = [0.0f32; B32 + 1];
        next[0] = dist[0];
        let mut next_min = B32 + 1;
        let mut next_max = 0usize;

        let status_dot = if is_tox {
            tox_counter += 1;
            (maxhp / 16).max(1) * tox_counter
        } else {
            psn_brn_dmg
        };
        let sand_this_turn = if weather_rem > 0 {
            weather_rem -= 1;
            sand_dmg
        } else {
            0
        };
        let res_this_turn =
            lefties + leech_gain - leech_drain - status_dot - curse_dmg - sand_this_turn;

        let p_def_selfhit = if def_conf > 0 {
            def_conf -= 1;
            0.5f32
        } else {
            0.0f32
        };

        let mut p_att = 1.0f32;
        if att_rech {
            att_rech = false;
            p_att = 0.0;
        } else if p_still_frz > 0.0 {
            p_still_frz *= 231.0 / 256.0;
            p_att *= 1.0 - p_still_frz;
        } else {
            if att_conf > 0 {
                att_conf -= 1;
                p_att *= 0.5;
            }
            if att_par {
                p_att *= 0.75;
            }
        }

        let kern = if att_slp > 0 {
            att_slp -= 1;
            &kern_slp
        } else {
            &kern_awake
        };

        let p_sub_now = p_sub_up;
        if p_sub_now > 0.0 && p_att > 0.0 && kern.max_dmg > 0 {
            let mut p_break = 0.0f32;
            let mut exp_chip = 0.0f32;
            for i in 0..kern.len {
                let (dmg, prob) = kern.pts[i];
                if dmg as f32 >= rem_sub_hp {
                    p_break += prob;
                } else if dmg > 0 {
                    exp_chip += prob * (dmg as f32);
                }
            }
            rem_sub_hp = (rem_sub_hp - p_att * exp_chip).max(1.0);
            p_sub_up = (p_sub_up * (1.0 - p_att * p_break)).clamp(0.0, 1.0);
        }

        for b in min_b..=max_b {
            let p_state = dist[b];
            if p_state <= 1e-7 {
                continue;
            }
            let raw_hp = b_to_hp(b, step);

            let branches: [(i16, f32); 2] = if p_def_selfhit > 0.0 {
                [
                    (raw_hp, 1.0 - p_def_selfhit),
                    ((raw_hp - def_conf_dmg).max(0), p_def_selfhit),
                ]
            } else {
                [(raw_hp, 1.0), (0, 0.0)]
            };

            for &(hp_after_self, w_self) in &branches {
                if w_self <= 0.0 {
                    continue;
                }
                let pm = p_state * w_self;
                if hp_after_self <= 0 {
                    next[0] += pm;
                    continue;
                }
                if p_att < 1.0 {
                    let p_no_act = pm * (1.0 - p_att);
                    let final_hp = (hp_after_self + res_this_turn).clamp(0, maxhp);
                    let tb = to_b(final_hp);
                    next[tb] += p_no_act;
                    if tb > 0 && p_no_act > 0.0 {
                        if tb < next_min {
                            next_min = tb;
                        }
                        if tb > next_max {
                            next_max = tb;
                        }
                    }
                }
                if p_att > 0.0 {
                    let pa = pm * p_att;
                    for i in 0..kern.len {
                        let (dmg, prob) = kern.pts[i];
                        if dmg > 0 && p_sub_now > 0.0 {
                            let final_sub_hp = (hp_after_self + res_this_turn).clamp(0, maxhp);
                            let tb_sub = to_b(final_sub_hp);
                            let add_sub = pa * prob * p_sub_now;
                            next[tb_sub] += add_sub;
                            if tb_sub > 0 && add_sub > 0.0 {
                                if tb_sub < next_min {
                                    next_min = tb_sub;
                                }
                                if tb_sub > next_max {
                                    next_max = tb_sub;
                                }
                            }
                            let p_unsub = 1.0 - p_sub_now;
                            if p_unsub > 1e-6 {
                                let post_hit = (hp_after_self - dmg).max(0);
                                let final_hp = if post_hit <= 0 {
                                    0
                                } else {
                                    (post_hit + res_this_turn).clamp(0, maxhp)
                                };
                                let tb = to_b(final_hp);
                                let add = pa * prob * p_unsub;
                                next[tb] += add;
                                if tb > 0 && add > 0.0 {
                                    if tb < next_min {
                                        next_min = tb;
                                    }
                                    if tb > next_max {
                                        next_max = tb;
                                    }
                                }
                            }
                        } else {
                            let post_hit = (hp_after_self - dmg).max(0);
                            let final_hp = if post_hit <= 0 {
                                0
                            } else {
                                (post_hit + res_this_turn).clamp(0, maxhp)
                            };
                            let tb = to_b(final_hp);
                            let add = pa * prob;
                            next[tb] += add;
                            if tb > 0 && add > 0.0 {
                                if tb < next_min {
                                    next_min = tb;
                                }
                                if tb > next_max {
                                    next_max = tb;
                                }
                            }
                        }
                    }
                }
            }
        }
        dist = next;
        min_b = next_min;
        max_b = next_max;
        out[step] = if min_b > max_b {
            0.0
        } else {
            (1.0 - (dist[0] as f64)).clamp(0.0, 1.0)
        };
    }

    if !need_mean_hp || min_b > max_b {
        return (out, 0.0);
    }
    let mut alive_mass = 0.0f32;
    let mut weighted_b = 0.0f32;
    for b in min_b..=max_b {
        alive_mass += dist[b];
        weighted_b += dist[b] * (b as f32 / B32 as f32);
    }
    let mean_hp_frac = if alive_mass > 1e-6 {
        (weighted_b / alive_mass) as f64
    } else {
        0.0
    };
    (out, mean_hp_frac)
}

// =========================================================================
// 2. 2D Coupled Schur-Complement Bellman Solver & Rank-3 SVD
// =========================================================================

#[derive(Clone, Copy)]
struct BinDist16 {
    bins: [(u8, f32); 8],
    len: usize,
    ko_prob: f32,
}

impl BinDist16 {
    #[inline]
    const fn identity(b: u8) -> Self {
        let mut bins = [(0u8, 0.0f32); 8];
        bins[0] = (b, 1.0);
        Self {
            bins,
            len: 1,
            ko_prob: 0.0,
        }
    }

    #[inline]
    fn push(&mut self, b: u8, prob: f32) {
        if prob <= 0.0 {
            return;
        }
        if b == 0 {
            self.ko_prob += prob;
            return;
        }
        for i in 0..self.len {
            if self.bins[i].0 == b {
                self.bins[i].1 += prob;
                return;
            }
        }
        if self.len < 8 {
            self.bins[self.len] = (b, prob);
            self.len += 1;
        } else {
            self.bins[self.len - 1].1 += prob;
        }
    }
}

#[derive(Clone, Copy)]
struct Side1DOps {
    has_heal: bool,
    heal_is_rest: bool,
    has_rest: bool,
    has_st: bool,
    has_berry: bool,
    half_heal_bin: [u8; B16 + 1],
    /// `eot_bin[cured][b]` -> post-residual bin in `0..=16`
    eot_bin: [[u8; B16 + 1]; 2],
    /// Minimum and maximum non-zero surviving HP bin across `b in 1..=16` for `kind in 0..3`
    min_surv_bin: [u8; 3],
    max_surv_bin: [u8; 3],
    /// `incoming[kind][b]` where `kind = 0` (no foe attack), `1` (foe awake attack), `2` (foe Sleep Talk)
    incoming: [[BinDist16; B16 + 1]; 3],
}

impl Side1DOps {
    #[inline]
    fn to_b16(hp: i16, maxhp: i16) -> u8 {
        if hp <= 0 {
            0
        } else {
            (((hp as i32 * B16 as i32 + maxhp as i32 - 1) / maxhp as i32) as u8).clamp(1, B16 as u8)
        }
    }

    #[inline]
    fn b16_to_hp(b: usize, maxhp: i16) -> i16 {
        if b == 0 {
            0
        } else {
            (((b as i32 * maxhp as i32) + (B16 as i32 / 2)) / B16 as i32)
                .clamp(1, maxhp as i32) as i16
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn build_one(
        cb: &CompactBattle,
        table: &MatchTable,
        side: usize,
        slot: usize,
        foe_side: usize,
        foe_slot: usize,
        inst_heal_amt: i16,
        has_rest: bool,
        has_st: bool,
        max_s_me: usize,
        foe_k_att: &DmgKernel1D,
        foe_k_st: Option<&DmgKernel1D>,
    ) -> Self {
        let idx = side * 3 + slot;
        let foe_idx = foe_side * 3 + foe_slot;
        let mon = &cb.mons[idx];
        let foe_mon = &cb.mons[foe_idx];
        let spec = &table.mons[idx];
        let foe_spec = &table.mons[foe_idx];
        let maxhp = spec.maxhp.max(1);

        let has_half_heal = inst_heal_amt > 0;
        let perish_active = (mon.vol_flags & vol_bit::PERISH_SONG) != 0;
        let has_berry = mon.item != 0 && (spec.is_mintberry || spec.is_miracleberry);
        let is_frz =
            mon.status_raw() == Status::Frz as u8 && !(mon.item != 0 && spec.is_miracleberry);
        let has_heal = !perish_active && !is_frz && (has_half_heal || has_rest);
        let heal_is_rest = !has_half_heal && has_rest;

        let mut half_heal_bin = [0u8; B16 + 1];
        if has_heal && !heal_is_rest {
            for b in 1..=B16 {
                let hp = Self::b16_to_hp(b, maxhp);
                let healed = (hp + inst_heal_amt).min(maxhp);
                half_heal_bin[b] = Self::to_b16(healed, maxhp);
            }
        }

        let lefties = if mon.item != 0 && spec.is_leftovers {
            (maxhp / 16).max(1)
        } else {
            0
        };
        let leech_drain = if (mon.vol_flags & vol_bit::LEECH_SEED) != 0 {
            (maxhp / 8).max(1)
        } else {
            0
        };
        let leech_gain = if (foe_mon.vol_flags & vol_bit::LEECH_SEED) != 0 {
            (foe_spec.maxhp.max(1) / 8).max(1)
        } else {
            0
        };
        let st = mon.status_raw();
        let cnt_packed = mon.status_counter();
        let is_tox = st == Status::Tox as u8
            || ((cnt_packed & 0x10) != 0 && (st == Status::Psn as u8 || st == Status::Brn as u8));
        let tox_c = ((cnt_packed & 0x0F) as i16).max(2);
        let status_dot = if is_tox {
            (maxhp / 16).max(1) * tox_c
        } else if st == Status::Brn as u8 || st == Status::Psn as u8 {
            (maxhp / 8).max(1)
        } else {
            0
        };
        let curse_dot = if (mon.vol_flags & vol_bit::CURSE) != 0 {
            (maxhp / 4).max(1)
        } else {
            0
        };
        let sand_dot = if cb.weather_turns > 0
            && cb.weather_kind == 3
            && (spec.type_mask & table.sandstorm_immune_mask) == 0
        {
            (maxhp / 8).max(1)
        } else {
            0
        };
        let net_uncured = lefties + leech_gain - leech_drain - status_dot - curse_dot - sand_dot;
        let net_cured = lefties + leech_gain - leech_drain - curse_dot - sand_dot;
        let need_cured_eot = (max_s_me > 0 && has_rest) || (has_heal && heal_is_rest);

        let mut eot_bin = [[0u8; B16 + 1]; 2];
        for b in 1..=B16 {
            let hp = Self::b16_to_hp(b, maxhp);
            eot_bin[0][b] = Self::to_b16((hp + net_uncured).clamp(0, maxhp), maxhp);
            if need_cured_eot {
                eot_bin[1][b] = Self::to_b16((hp + net_cured).clamp(0, maxhp), maxhp);
            }
        }

        let foe_miracle = foe_mon.item != 0 && foe_spec.is_miracleberry;
        let foe_frz = foe_mon.status_raw() == Status::Frz as u8 && !foe_miracle;
        let foe_par = foe_mon.status_raw() == Status::Par as u8 && !foe_miracle;
        let foe_conf = (foe_mon.vol_flags & vol_bit::CONFUSION) != 0 && !foe_miracle;
        let p_foe_act = (if foe_frz { 25.0 / 256.0 } else { 1.0f32 })
            * (if foe_par { 0.75f32 } else { 1.0f32 })
            * (if foe_conf { 0.50f32 } else { 1.0f32 });

        let mut incoming = [[BinDist16::identity(0); B16 + 1]; 3];
        let mut min_surv_bin = [1u8, B16 as u8 + 1, B16 as u8 + 1];
        let mut max_surv_bin = [B16 as u8, 0u8, 0u8];
        for b in 1..=B16 {
            let hp = Self::b16_to_hp(b, maxhp);

            let mut d1 = BinDist16 {
                bins: [(0, 0.0); 8],
                len: 0,
                ko_prob: 0.0,
            };
            if foe_k_att.max_dmg <= 0 {
                d1.push(b as u8, 1.0);
            } else {
                if p_foe_act < 1.0 {
                    d1.push(b as u8, 1.0 - p_foe_act);
                }
                for i in 0..foe_k_att.len {
                    let (dmg, prob) = foe_k_att.pts[i];
                    let post_hp = (hp - dmg).max(0);
                    d1.push(Self::to_b16(post_hp, maxhp), p_foe_act * prob);
                }
            }
            incoming[1][b] = d1;

            if let Some(kern_st) = foe_k_st {
                let mut d2 = BinDist16 {
                    bins: [(0, 0.0); 8],
                    len: 0,
                    ko_prob: 0.0,
                };
                if kern_st.max_dmg <= 0 {
                    d2.push(b as u8, 1.0);
                } else {
                    if p_foe_act < 1.0 {
                        d2.push(b as u8, 1.0 - p_foe_act);
                    }
                    for i in 0..kern_st.len {
                        let (dmg, prob) = kern_st.pts[i];
                        let post_hp = (hp - dmg).max(0);
                        d2.push(Self::to_b16(post_hp, maxhp), p_foe_act * prob);
                    }
                }
                for i in 0..d2.len {
                    let mb = d2.bins[i].0;
                    if mb < min_surv_bin[2] {
                        min_surv_bin[2] = mb;
                    }
                    if mb > max_surv_bin[2] {
                        max_surv_bin[2] = mb;
                    }
                }
                incoming[2][b] = d2;
            }
            for i in 0..d1.len {
                let mb = d1.bins[i].0;
                if mb < min_surv_bin[1] {
                    min_surv_bin[1] = mb;
                }
                if mb > max_surv_bin[1] {
                    max_surv_bin[1] = mb;
                }
            }
        }

        Self {
            has_heal,
            heal_is_rest,
            has_rest,
            has_st,
            has_berry,
            half_heal_bin,
            eot_bin,
            min_surv_bin,
            max_surv_bin,
            incoming,
        }
    }

    fn build_pair(
        cb: &CompactBattle,
        table: &MatchTable,
        s0: usize,
        slot0: usize,
        s1: usize,
        slot1: usize,
    ) -> ([Self; 2], usize, usize, u8) {
        let scan_moves = |side: usize, slot: usize| -> (i16, bool, bool) {
            let idx = side * 3 + slot;
            let mon = &cb.mons[idx];
            let spec = &table.mons[idx];
            let maxhp = spec.maxhp.max(1);
            let dis = (mon.vol_flags & vol_bit::DISABLE) != 0;
            let dis_slot = ((mon.slot_refs >> 4) & 0x03) as usize;
            let mut hh_amt = 0i16;
            let mut hr = false;
            let mut hst = false;
            for m_i in 0..spec.num_moves as usize {
                if mon.pp[m_i] > 0 && !(dis && m_i == dis_slot) {
                    let ms = &table.move_specs[idx][m_i];
                    if let Some(amt) = instant_heal_amount(cb, ms, maxhp) {
                        if amt > hh_amt {
                            hh_amt = amt;
                        }
                    } else if ms.is_rest {
                        hr = true;
                    }
                    if ms.special_kind == 10 {
                        hst = true;
                    }
                }
            }
            (hh_amt, hr, hst)
        };

        let (hh_amt0, hr0, hst0) = scan_moves(s0, slot0);
        let (hh_amt1, hr1, hst1) = scan_moves(s1, slot1);
        let hh0 = hh_amt0 > 0;
        let hh1 = hh_amt1 > 0;

        let idx0 = s0 * 3 + slot0;
        let idx1 = s1 * 3 + slot1;
        let m0 = &cb.mons[idx0];
        let m1 = &cb.mons[idx1];
        let sp0 = &table.mons[idx0];
        let sp1 = &table.mons[idx1];
        let berry0 = m0.item != 0 && (sp0.is_mintberry || sp0.is_miracleberry);
        let berry1 = m1.item != 0 && (sp1.is_mintberry || sp1.is_miracleberry);
        let perish0 = (m0.vol_flags & vol_bit::PERISH_SONG) != 0;
        let perish1 = (m1.vol_flags & vol_bit::PERISH_SONG) != 0;
        let frz0 = m0.status_raw() == Status::Frz as u8 && !(m0.item != 0 && sp0.is_miracleberry);
        let frz1 = m1.status_raw() == Status::Frz as u8 && !(m1.item != 0 && sp1.is_miracleberry);
        let max_s0 = if !perish0 && !frz0 && !hh0 && hr0 && !berry0 {
            2usize
        } else if m0.status_raw() == Status::Slp as u8 && !berry0 && m0.status_counter() >= 3 {
            1usize
        } else {
            0usize
        };
        let max_s1 = if !perish1 && !frz1 && !hh1 && hr1 && !berry1 {
            2usize
        } else if m1.status_raw() == Status::Slp as u8 && !berry1 && m1.status_counter() >= 3 {
            1usize
        } else {
            0usize
        };

        let k01_att = DmgKernel1D::best_attack_grid(cb, table, s0, slot0, s1, slot1);
        let k10_att = DmgKernel1D::best_attack_grid(cb, table, s1, slot1, s0, slot0);
        let k01_st = if max_s0 > 0 && hst0 {
            Some(DmgKernel1D::sleep_talk_grid(cb, table, s0, slot0, s1, slot1))
        } else {
            None
        };
        let k10_st = if max_s1 > 0 && hst1 {
            Some(DmgKernel1D::sleep_talk_grid(cb, table, s1, slot1, s0, slot0))
        } else {
            None
        };

        let ops0 = Self::build_one(
            cb,
            table,
            s0,
            slot0,
            s1,
            slot1,
            hh_amt0,
            hr0,
            hst0,
            max_s0,
            &k10_att,
            k10_st.as_ref(),
        );
        let ops1 = Self::build_one(
            cb,
            table,
            s1,
            slot1,
            s0,
            slot0,
            hh_amt1,
            hr1,
            hst1,
            max_s1,
            &k01_att,
            k01_st.as_ref(),
        );

        let spd0 = cb.mon_action_speed(table, s0, slot0);
        let spd1 = cb.mon_action_speed(table, s1, slot1);
        let spd_ord = if spd0 < spd1 {
            0u8
        } else if spd0 == spd1 {
            1u8
        } else {
            2u8
        };

        ([ops0, ops1], max_s0, max_s1, spd_ord)
    }
}

#[inline]
fn eval_exact_root_table(
    v_table: &[[[[f32; B16 + 1]; B16 + 1]; 3]; 3],
    cb: &CompactBattle,
    table: &MatchTable,
    s0: usize,
    slot0: usize,
    s1: usize,
    slot1: usize,
) -> f64 {
    let idx = [s0 * 3 + slot0, s1 * 3 + slot1];
    let hp = [cb.mons[idx[0]].hp(), cb.mons[idx[1]].hp()];
    if hp[0] <= 0 && hp[1] <= 0 {
        return 0.5;
    }
    if hp[0] <= 0 {
        return 0.0;
    }
    if hp[1] <= 0 {
        return 1.0;
    }
    let maxhp = [
        table.mons[idx[0]].maxhp.max(1),
        table.mons[idx[1]].maxhp.max(1),
    ];
    let slp = [
        side_sleep_rem(cb, table, s0, slot0),
        side_sleep_rem(cb, table, s1, slot1),
    ];

    let mut has_heal = [false; 2];
    let mut heal_is_rest = [false; 2];
    let mut has_rest = [false; 2];
    let mut has_st = [false; 2];
    let mut has_berry = [false; 2];
    let mut p_act = [1.0f32; 2];
    let mut eot_net = [[0i16; 2]; 2];
    let mut sub_hp_init = [0i16; 2];

    for i in 0..2 {
        let m = &cb.mons[idx[i]];
        let fm = &cb.mons[idx[1 - i]];
        let sp = &table.mons[idx[i]];
        let fsp = &table.mons[idx[1 - i]];
        if (m.vol_flags & vol_bit::SUBSTITUTE) != 0 {
            sub_hp_init[i] = m.sub_hp().max(1);
        }
        let dis = (m.vol_flags & vol_bit::DISABLE) != 0;
        let dis_slot = ((m.slot_refs >> 4) & 0x03) as usize;
        let mut hh = false;
        let mut hr = false;
        let mut hst = false;
        for m_i in 0..sp.num_moves as usize {
            if m.pp[m_i] > 0 && !(dis && m_i == dis_slot) {
                let ms = &table.move_specs[idx[i]][m_i];
                if ms.is_half_heal {
                    hh = true;
                } else if ms.is_rest {
                    hr = true;
                }
                if ms.special_kind == 10 {
                    hst = true;
                }
            }
        }
        let perish = (m.vol_flags & vol_bit::PERISH_SONG) != 0;
        let miracle = m.item != 0 && sp.is_miracleberry;
        let berry = m.item != 0 && (sp.is_mintberry || sp.is_miracleberry);
        let st_raw = m.status_raw();
        let is_frz = st_raw == Status::Frz as u8 && !miracle;
        let is_par = st_raw == Status::Par as u8 && !miracle;
        let is_conf = (m.vol_flags & vol_bit::CONFUSION) != 0 && m.timers[0] > 1 && !miracle;
        let is_rech = (m.slot_refs & 0x80) != 0;

        has_heal[i] = !perish && !is_frz && !is_rech && (hh || hr);
        heal_is_rest[i] = !hh && hr;
        has_rest[i] = hr;
        has_st[i] = hst;
        has_berry[i] = berry;
        p_act[i] = if is_rech {
            0.0
        } else {
            (if is_frz { 25.0 / 256.0 } else { 1.0f32 })
                * (if is_par { 0.75f32 } else { 1.0f32 })
                * (if is_conf { 0.50f32 } else { 1.0f32 })
        };

        let lefties = if m.item != 0 && sp.is_leftovers {
            (maxhp[i] / 16).max(1)
        } else {
            0
        };
        let leech_drain = if (m.vol_flags & vol_bit::LEECH_SEED) != 0 {
            (maxhp[i] / 8).max(1)
        } else {
            0
        };
        let leech_gain = if (fm.vol_flags & vol_bit::LEECH_SEED) != 0 {
            (fsp.maxhp.max(1) / 8).max(1)
        } else {
            0
        };
        let cnt_packed = m.status_counter();
        let is_tox = st_raw == Status::Tox as u8
            || ((cnt_packed & 0x10) != 0
                && (st_raw == Status::Psn as u8 || st_raw == Status::Brn as u8));
        let tox_c = ((cnt_packed & 0x0F) as i16 + 1).max(2);
        let status_dot = if is_tox {
            (maxhp[i] / 16).max(1) * tox_c
        } else if st_raw == Status::Brn as u8 || st_raw == Status::Psn as u8 {
            (maxhp[i] / 8).max(1)
        } else {
            0
        };
        let curse_dot = if (m.vol_flags & vol_bit::CURSE) != 0 {
            (maxhp[i] / 4).max(1)
        } else {
            0
        };
        let sand_dot = if cb.weather_turns > 0
            && cb.weather_kind == 3
            && (sp.type_mask & table.sandstorm_immune_mask) == 0
        {
            (maxhp[i] / 8).max(1)
        } else {
            0
        };
        eot_net[i][0] = lefties + leech_gain - leech_drain - status_dot - curse_dot - sand_dot;
        eot_net[i][1] = lefties + leech_gain - leech_drain - curse_dot - sand_dot;
    }

    let k_att = [
        if slp[0] == 0 && p_act[0] > 0.0 {
            DmgKernel1D::best_attack(cb, table, s0, slot0, s1, slot1)
        } else {
            DmgKernel1D::zero()
        },
        if slp[1] == 0 && p_act[1] > 0.0 {
            DmgKernel1D::best_attack(cb, table, s1, slot1, s0, slot0)
        } else {
            DmgKernel1D::zero()
        },
    ];
    let k_st = [
        if slp[0] > 0 && has_st[0] && p_act[0] > 0.0 {
            DmgKernel1D::sleep_talk(cb, table, s0, slot0, s1, slot1)
        } else {
            DmgKernel1D::zero()
        },
        if slp[1] > 0 && has_st[1] && p_act[1] > 0.0 {
            DmgKernel1D::sleep_talk(cb, table, s1, slot1, s0, slot0)
        } else {
            DmgKernel1D::zero()
        },
    ];

    let opp_hit_buffer = [0, 1].map(|i| {
        let foe_k = &k_att[1 - i];
        let mut mean_dmg = 0.0f32;
        for p_i in 0..foe_k.len {
            mean_dmg += (foe_k.pts[p_i].0 as f32) * foe_k.pts[p_i].1;
        }
        (mean_dmg.round() as i16).clamp(maxhp[i] / 4, maxhp[i] / 2)
    });

    let interp_coords = |h: i16, mx: i16| -> (usize, usize, f32) {
        let x = ((h as f32) * (B16 as f32) / (mx.max(1) as f32)).clamp(0.0, B16 as f32);
        let lo = (x.floor() as usize).clamp(1, B16);
        let hi = (x.ceil() as usize).clamp(1, B16);
        let w = (x - (lo as f32)).clamp(0.0, 1.0);
        (lo, hi, w)
    };

    let eval_order = |first: usize, a0: u8, a1: u8| -> f64 {
        let second = 1 - first;
        let act = [a0, a1];
        let win_first = if first == 0 { 1.0f32 } else { 0.0f32 };
        let win_second = 1.0f32 - win_first;
        let sub_fir = sub_hp_init[first];
        let sub_sec = sub_hp_init[second];

        let (hp_fir_mid, ns_fir, cured_fir, k_fir_opt) = if slp[first] > 0 {
            (
                hp[first],
                slp[first] - 1,
                has_rest[first],
                if has_st[first] {
                    Some(&k_st[first])
                } else {
                    None
                },
            )
        } else if act[first] == 1 {
            if heal_is_rest[first] {
                (
                    maxhp[first],
                    if has_berry[first] { 0 } else { 2 },
                    true,
                    None,
                )
            } else {
                (
                    (hp[first] + maxhp[first] / 2).min(maxhp[first]),
                    0,
                    false,
                    None,
                )
            }
        } else if act[first] == 2 {
            (hp[first], 0, false, None)
        } else {
            (hp[first], 0, false, Some(&k_att[first]))
        };

        let mut sec_pts = [(0i16, 0i16, 0.0f32); 17];
        let mut sec_len = 0usize;
        let mut ko_sec_prob = 0.0f32;
        let push_pt = |pts: &mut [(i16, i16, f32); 17],
                       len: &mut usize,
                       ko: &mut f32,
                       h: i16,
                       sub_h: i16,
                       p: f32| {
            if p <= 0.0 {
                return;
            }
            if h <= 0 {
                *ko += p;
                return;
            }
            for idx_p in 0..*len {
                if pts[idx_p].0 == h && pts[idx_p].1 == sub_h {
                    pts[idx_p].2 += p;
                    return;
                }
            }
            if *len < 17 {
                pts[*len] = (h, sub_h, p);
                *len += 1;
            } else {
                pts[*len - 1].2 += p;
            }
        };

        let pa_fir = p_act[first];
        if let Some(kern) = k_fir_opt {
            if kern.max_dmg <= 0 || pa_fir <= 0.0 {
                push_pt(
                    &mut sec_pts,
                    &mut sec_len,
                    &mut ko_sec_prob,
                    hp[second],
                    sub_sec,
                    1.0,
                );
            } else {
                if pa_fir < 1.0 {
                    push_pt(
                        &mut sec_pts,
                        &mut sec_len,
                        &mut ko_sec_prob,
                        hp[second],
                        sub_sec,
                        1.0 - pa_fir,
                    );
                }
                for i in 0..kern.len {
                    let (dmg, prob) = kern.pts[i];
                    let (h_after, sub_after) = if sub_sec > 0 && dmg > 0 {
                        (hp[second], (sub_sec - dmg).max(0))
                    } else {
                        ((hp[second] - dmg).max(0), sub_sec)
                    };
                    push_pt(
                        &mut sec_pts,
                        &mut sec_len,
                        &mut ko_sec_prob,
                        h_after,
                        sub_after,
                        pa_fir * prob,
                    );
                }
            }
        } else {
            push_pt(
                &mut sec_pts,
                &mut sec_len,
                &mut ko_sec_prob,
                hp[second],
                sub_sec,
                1.0,
            );
        }

        let mut total_val = (ko_sec_prob as f64) * (win_first as f64);
        if sec_len == 0 {
            return total_val.clamp(0.0, 1.0);
        }

        let (sec_mode, ns_sec, cured_sec, k_sec_opt) = if slp[second] > 0 {
            (
                0u8,
                slp[second] - 1,
                has_rest[second],
                if has_st[second] {
                    Some(&k_st[second])
                } else {
                    None
                },
            )
        } else if act[second] == 1 {
            if heal_is_rest[second] {
                (1u8, if has_berry[second] { 0 } else { 2 }, true, None)
            } else {
                (2u8, 0, false, None)
            }
        } else if act[second] == 2 {
            (0u8, 0, false, None)
        } else {
            (0u8, 0, false, Some(&k_att[second]))
        };

        let mut fir_pts = [(0i16, 0i16, 0.0f32); 17];
        let mut fir_len = 0usize;
        let mut ko_fir_prob = 0.0f32;
        let pa_sec = p_act[second];
        if let Some(kern) = k_sec_opt {
            if kern.max_dmg <= 0 || pa_sec <= 0.0 {
                push_pt(
                    &mut fir_pts,
                    &mut fir_len,
                    &mut ko_fir_prob,
                    hp_fir_mid,
                    sub_fir,
                    1.0,
                );
            } else {
                if pa_sec < 1.0 {
                    push_pt(
                        &mut fir_pts,
                        &mut fir_len,
                        &mut ko_fir_prob,
                        hp_fir_mid,
                        sub_fir,
                        1.0 - pa_sec,
                    );
                }
                for i in 0..kern.len {
                    let (dmg, prob) = kern.pts[i];
                    let (h_after, sub_after) = if sub_fir > 0 && dmg > 0 {
                        (hp_fir_mid, (sub_fir - dmg).max(0))
                    } else {
                        ((hp_fir_mid - dmg).max(0), sub_fir)
                    };
                    push_pt(
                        &mut fir_pts,
                        &mut fir_len,
                        &mut ko_fir_prob,
                        h_after,
                        sub_after,
                        pa_sec * prob,
                    );
                }
            }
        } else {
            push_pt(
                &mut fir_pts,
                &mut fir_len,
                &mut ko_fir_prob,
                hp_fir_mid,
                sub_fir,
                1.0,
            );
        }

        let mut fir_end = [(0i16, 0usize, 0usize, 0.0f32, 0.0f32); 17];
        let eot_fir_val = eot_net[first][cured_fir as usize];
        for (idx_f, &(hp_fir_post, sub_fir_after, p_fir)) in fir_pts[..fir_len].iter().enumerate() {
            let h_end = (hp_fir_post + eot_fir_val).clamp(0, maxhp[first]);
            let (lo, hi, w) = if h_end > 0 {
                let h_eff = if sub_fir_after > 0 {
                    let buf = sub_fir_after.max(opp_hit_buffer[first]);
                    (h_end + buf).min(maxhp[first])
                } else {
                    h_end
                };
                interp_coords(h_eff, maxhp[first])
            } else {
                (1, 1, 0.0)
            };
            fir_end[idx_f] = (h_end, lo, hi, w, p_fir);
        }

        let (ns0, ns1) = if first == 0 {
            (ns_fir.min(2), ns_sec.min(2))
        } else {
            (ns_sec.min(2), ns_fir.min(2))
        };
        let tbl = &v_table[ns0][ns1];

        let eval_branch = |hp_sec_end: i16, sub_sec_after: i16| -> f64 {
            let mut branch_val = (ko_fir_prob as f64) * (win_second as f64);
            if hp_sec_end <= 0 {
                for &(hp_fir_end, _, _, _, p_fir) in &fir_end[..fir_len] {
                    let cont = if hp_fir_end <= 0 {
                        0.5f64
                    } else {
                        win_first as f64
                    };
                    branch_val += (p_fir as f64) * cont;
                }
            } else {
                let h_sec_eff = if sub_sec_after > 0 {
                    let buf = sub_sec_after.max(opp_hit_buffer[second]);
                    (hp_sec_end + buf).min(maxhp[second])
                } else {
                    hp_sec_end
                };
                let (s_lo, s_hi, s_w) = interp_coords(h_sec_eff, maxhp[second]);
                for &(hp_fir_end, f_lo, f_hi, f_w, p_fir) in &fir_end[..fir_len] {
                    let cont = if hp_fir_end <= 0 {
                        win_second as f64
                    } else {
                        let (b0_lo, b0_hi, w0, b1_lo, b1_hi, w1) = if first == 0 {
                            (f_lo, f_hi, f_w as f64, s_lo, s_hi, s_w as f64)
                        } else {
                            (s_lo, s_hi, s_w as f64, f_lo, f_hi, f_w as f64)
                        };
                        let c00 = (tbl[b0_lo][b1_lo] as f64) - 0.5;
                        let c01 = (tbl[b0_lo][b1_hi] as f64) - 0.5;
                        let c10 = (tbl[b0_hi][b1_lo] as f64) - 0.5;
                        let c11 = (tbl[b0_hi][b1_hi] as f64) - 0.5;
                        let c0 = c00 + w1 * (c01 - c00);
                        let c1 = c10 + w1 * (c11 - c10);
                        let c_cont = (c0 + w0 * (c1 - c0)).clamp(-0.5, 0.5);
                        0.5 + (GAMMA as f64) * c_cont
                    };
                    branch_val += (p_fir as f64) * cont;
                }
            }
            branch_val
        };

        let eot_sec_val = eot_net[second][cured_sec as usize];
        if sec_mode == 1 {
            let hp_sec_end = (maxhp[second] + eot_sec_val).clamp(0, maxhp[second]);
            for &(_, sub_sec_after, p_sec) in &sec_pts[..sec_len] {
                total_val += (p_sec as f64) * eval_branch(hp_sec_end, sub_sec_after);
            }
        } else {
            for &(hp_sec_mid, sub_sec_after, p_sec) in &sec_pts[..sec_len] {
                let hp_sec_post = if sec_mode == 2 {
                    (hp_sec_mid + maxhp[second] / 2).min(maxhp[second])
                } else {
                    hp_sec_mid
                };
                let hp_sec_end = (hp_sec_post + eot_sec_val).clamp(0, maxhp[second]);
                total_val += (p_sec as f64) * eval_branch(hp_sec_end, sub_sec_after);
            }
        }
        total_val.clamp(0.0, 1.0)
    };

    let spd0 = cb.mon_action_speed(table, s0, slot0);
    let spd1 = cb.mon_action_speed(table, s1, slot1);
    let n0 = if slp[0] == 0 && has_heal[0] && hp[0] < maxhp[0] {
        2
    } else {
        1
    };
    let n1 = if slp[1] == 0 && has_heal[1] && hp[1] < maxhp[1] {
        2
    } else {
        1
    };
    let eval_pair = |a0: u8, a1: u8| -> f64 {
        if spd0 > spd1 {
            eval_order(0, a0, a1)
        } else if spd1 > spd0 {
            eval_order(1, a0, a1)
        } else {
            0.5 * (eval_order(0, a0, a1) + eval_order(1, a0, a1))
        }
    };
    let mut m_cent = [[0.0f64; 3]; 3];
    for a0 in 0..n0 {
        let p0_ok = if a0 == 1 { p_act[0] as f64 } else { 1.0 };
        for a1 in 0..n1 {
            let p1_ok = if a1 == 1 { p_act[1] as f64 } else { 1.0 };
            let mut cell = p0_ok * p1_ok * eval_pair(a0 as u8, a1 as u8);
            if p0_ok < 1.0 {
                cell += (1.0 - p0_ok) * p1_ok * eval_pair(2, a1 as u8);
            }
            if p1_ok < 1.0 {
                cell += p0_ok * (1.0 - p1_ok) * eval_pair(a0 as u8, 2);
            }
            if p0_ok < 1.0 && p1_ok < 1.0 {
                cell += (1.0 - p0_ok) * (1.0 - p1_ok) * eval_pair(2, 2);
            }
            m_cent[a0][a1] = cell - 0.5;
        }
    }
    (0.5 + solve_centered_zerosum_3x3(&m_cent, n0, n1)).clamp(0.0, 1.0)
}

#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
struct Key128(u64, u64);

struct Hasher128 {
    h0: u64,
    h1: u64,
}

impl Hasher128 {
    #[inline(always)]
    fn new() -> Self {
        Self {
            h0: 0x9e3779b97f4a7c15,
            h1: 0xbf58476d1ce4e5b9,
        }
    }

    #[inline(always)]
    fn write_u64(&mut self, v: u64) {
        let mut a = self.h0 ^ v;
        a = (a ^ (a >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
        let mut b = self.h1 ^ v.rotate_left(29) ^ a;
        b = (b ^ (b >> 27)).wrapping_mul(0x94d049bb133111eb);
        self.h0 = a ^ (b >> 31);
        self.h1 = b ^ (a >> 29);
    }

    #[inline(always)]
    fn finish(self) -> Key128 {
        let mut a = self.h0 ^ (self.h1 >> 32);
        a = (a ^ (a >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
        a ^= a >> 31;
        let mut b = self.h1 ^ (self.h0 >> 32);
        b = (b ^ (b >> 27)).wrapping_mul(0x94d049bb133111eb);
        b ^= b >> 31;
        Key128(a | 1, b | 1)
    }
}

#[derive(Clone, Copy)]
struct CachedBellmanMasks {
    heal_mask0: [u32; B16 + 1],
    heal_mask1: [u32; B16 + 1],
}

impl CachedBellmanMasks {
    const fn empty() -> Self {
        Self {
            heal_mask0: [0; B16 + 1],
            heal_mask1: [0; B16 + 1],
        }
    }
}

#[derive(Clone, Copy)]
struct CachedBellman {
    op_key: Key128,
    v_table: [[[[f32; B16 + 1]; B16 + 1]; 3]; 3],
    heal_mask0: [u32; B16 + 1],
    heal_mask1: [u32; B16 + 1],
}

impl CachedBellman {
    const fn empty() -> Self {
        Self {
            op_key: Key128(0, 0),
            v_table: [[[[0.5; B16 + 1]; B16 + 1]; 3]; 3],
            heal_mask0: [0; B16 + 1],
            heal_mask1: [0; B16 + 1],
        }
    }
}

#[derive(Clone, Copy)]
struct FinitePpBellman {
    w0_5: [[[[u16; B16 + 1]; B16 + 1]; 3]; 3],
    w1_5: [[[[u16; B16 + 1]; B16 + 1]; 3]; 3],
    /// Stationary 1-turn breakthrough hazard rates `[kappa_0, kappa_1]` under `(Press, Press)`.
    kappa: [f32; 2],
    /// Stationary awake attack attempt frequencies `[r_atk_0, r_atk_1]`.
    r_atk_act: [f32; 2],
    /// Stationary awake heal attempt frequencies `[r_heal_0, r_heal_1]`.
    r_heal_act: [f32; 2],
    /// Stationary sleeping `Sleep Talk` attempt frequencies `[r_st_0, r_st_1]`.
    r_st_act: [f32; 2],
    /// Stationary mean HP fractions `[h_0, h_1]` on the slow manifold.
    mean_hp_frac: [f32; 2],
}

impl FinitePpBellman {
    const fn empty() -> Self {
        Self {
            w0_5: [[[[0; B16 + 1]; B16 + 1]; 3]; 3],
            w1_5: [[[[0; B16 + 1]; B16 + 1]; 3]; 3],
            kappa: [0.0; 2],
            r_atk_act: [1.0; 2],
            r_heal_act: [0.0; 2],
            r_st_act: [0.0; 2],
            mean_hp_frac: [0.5; 2],
        }
    }
}

const L1_SETS: usize = 8192;
const L1_WAYS: usize = 4;
const L2_SETS: usize = 1024;
const L2_WAYS: usize = 4;
const MAX_CACHE_SLOTS: usize = L2_SETS * L2_WAYS;
const META_SETS: usize = 2048;
const META_WAYS: usize = 2;
const MAX_META_SLOTS: usize = META_SETS * META_WAYS;

#[derive(Clone, Copy, Default)]
struct L1Way {
    key: Key128,
    gen: u32,
    slot: u16,
    meta_slot: u16,
}

#[derive(Clone, Copy, Default)]
struct L2Set {
    keys: [Key128; L2_WAYS],
    slots: [u16; L2_WAYS],
}

#[derive(Clone, Copy, Default)]
struct MetaWay {
    key: Key128,
    slot: u16,
}

#[derive(Clone, Copy)]
struct SideFastMeta {
    slot_base_kind: [u8; 4],
    slot_dmg_frac: [f64; 4],
    best_kern: DmgKernel1D,
    st_kern: DmgKernel1D,
    max_dmg_hp: i16,
    inst_heal_amt: i16,
    lefties_hp: i16,
    maxhp: i16,
    eot_net: [i16; 2],
    base_p_act: f32,
    net_eot_frac: f64,
    p_deduct: f64,
    flags: u8,
}

#[derive(Clone, Copy)]
struct L1PairFastMeta {
    sides: [SideFastMeta; 2],
    str_stats: StrugglePairStats,
    spd_ord: u8,
}

struct BellmanCache {
    l1: Vec<[L1Way; L1_WAYS]>,
    l2: Vec<L2Set>,
    slot_gen: Vec<u32>,
    masks: Vec<CachedBellmanMasks>,
    pool: Vec<CachedBellman>,
    slot_finite_gen: Vec<u32>,
    slot_finite_idx: Vec<u16>,
    finite_pool: Vec<FinitePpBellman>,
    meta_sets: Vec<[MetaWay; META_WAYS]>,
    meta_pool: Vec<L1PairFastMeta>,
}

impl BellmanCache {
    fn new() -> Self {
        Self {
            l1: vec![[L1Way::default(); L1_WAYS]; L1_SETS],
            l2: vec![L2Set::default(); L2_SETS],
            slot_gen: vec![0u32; MAX_CACHE_SLOTS],
            masks: Vec::with_capacity(64),
            pool: Vec::with_capacity(64),
            slot_finite_gen: vec![0u32; MAX_CACHE_SLOTS],
            slot_finite_idx: vec![u16::MAX; MAX_CACHE_SLOTS],
            finite_pool: Vec::with_capacity(32),
            meta_sets: vec![[MetaWay::default(); META_WAYS]; META_SETS],
            meta_pool: Vec::with_capacity(64),
        }
    }

    #[inline]
    fn record_l1(&mut self, l1_key: Key128, slot: u16, gen: u32) {
        let l1_idx = (l1_key.0 as usize) & (L1_SETS - 1);
        let set = &mut self.l1[l1_idx];
        for way in 0..L1_WAYS {
            if set[way].key == l1_key {
                let prev_meta = set[way].meta_slot;
                for w in (1..=way).rev() {
                    set[w] = set[w - 1];
                }
                set[0] = L1Way {
                    key: l1_key,
                    gen,
                    slot,
                    meta_slot: prev_meta,
                };
                return;
            }
        }
        for w in (1..L1_WAYS).rev() {
            set[w] = set[w - 1];
        }
        set[0] = L1Way {
            key: l1_key,
            gen,
            slot,
            meta_slot: u16::MAX,
        };
    }
}

thread_local! {
    static BELLMAN_CACHE: RefCell<BellmanCache> = RefCell::new(BellmanCache::new());
}

#[inline]
fn l1_state_key(
    cb: &CompactBattle,
    table: &MatchTable,
    s0: usize,
    slot0: usize,
    s1: usize,
    slot1: usize,
) -> Key128 {
    let idx0 = s0 * 3 + slot0;
    let idx1 = s1 * 3 + slot1;
    let m0 = &cb.mons[idx0];
    let m1 = &cb.mons[idx1];
    let sp0 = &table.mons[idx0];
    let sp1 = &table.mons[idx1];

    let mut h = Hasher128::new();
    h.write_u64(sp0.static_fp);
    h.write_u64(sp1.static_fp);

    let avail_mask = |m: &nc2000_engine::compact::CompactMon, num_moves: u8| -> u8 {
        let dis = (m.vol_flags & vol_bit::DISABLE) != 0;
        let dis_slot = (m.slot_refs >> 4) & 0x03;
        let mut mask = 0u8;
        for m_i in 0..num_moves {
            if m.pp[m_i as usize] > 0 && !(dis && m_i == dis_slot) {
                mask |= 1u8 << m_i;
            }
        }
        mask | (((m.item != 0) as u8) << 4)
    };
    let av0 = avail_mask(m0, sp0.num_moves);
    let av1 = avail_mask(m1, sp1.num_moves);

    let spd0 = cb.mon_action_speed(table, s0, slot0);
    let spd1 = cb.mon_action_speed(table, s1, slot1);
    let spd_ord = if spd0 < spd1 {
        0u8
    } else if spd0 == spd1 {
        1u8
    } else {
        2u8
    };

    let item_flags = |m: &nc2000_engine::compact::CompactMon,
                      sp: &nc2000_engine::compact::StaticMonSpec|
     -> u8 {
        if m.item != 0 {
            (sp.is_leftovers as u8)
                | ((sp.is_brightpowder as u8) << 1)
                | ((sp.is_mintberry as u8) << 2)
                | ((sp.is_miracleberry as u8) << 3)
                | ((sp.is_scope_lens as u8) << 4)
        } else {
            0
        }
    };

    let w_b0 = u64::from_le_bytes([
        m0.boosts[0] as u8,
        m0.boosts[1] as u8,
        m0.boosts[2] as u8,
        m0.boosts[3] as u8,
        m0.boosts[5] as u8,
        m0.boosts[6] as u8,
        av0,
        spd_ord | (item_flags(m0, sp0) << 2),
    ]);
    let w_b1 = u64::from_le_bytes([
        m1.boosts[0] as u8,
        m1.boosts[1] as u8,
        m1.boosts[2] as u8,
        m1.boosts[3] as u8,
        m1.boosts[5] as u8,
        m1.boosts[6] as u8,
        av1,
        item_flags(m1, sp1),
    ]);
    h.write_u64(w_b0);
    h.write_u64(w_b1);

    const VOL_MASK: u16 = vol_bit::SUBSTITUTE
        | vol_bit::LEECH_SEED
        | vol_bit::CONFUSION
        | vol_bit::CURSE
        | vol_bit::FORESIGHT
        | vol_bit::PERISH_SONG
        | vol_bit::BRN_ATK_DROP;
    let vol0 = m0.vol_flags & VOL_MASK;
    let vol1 = m1.vol_flags & VOL_MASK;

    let canon_st = |m: &nc2000_engine::compact::CompactMon| -> u8 {
        let raw = m.status_raw();
        let cnt = m.status_counter();
        if raw == Status::Tox as u8
            || ((cnt & 0x10) != 0 && (raw == Status::Psn as u8 || raw == Status::Brn as u8))
        {
            raw | 0x80 | (((cnt & 0x0F).max(2)) << 3)
        } else if raw == Status::Slp as u8 {
            raw | (((cnt >= 3) as u8) << 3)
        } else {
            raw
        }
    };
    let st0_packed = canon_st(m0);
    let st1_packed = canon_st(m1);

    let sub_bucket = |m: &nc2000_engine::compact::CompactMon,
                      sp: &nc2000_engine::compact::StaticMonSpec|
     -> u64 {
        if (m.vol_flags & vol_bit::SUBSTITUTE) == 0 {
            0
        } else {
            let max_sub = (sp.maxhp / 4).max(1) as i32;
            let sh = m.sub_hp().max(1) as i32;
            (((sh * 4 + max_sub - 1) / max_sub) as u64).clamp(1, 4)
        }
    };

    let weather_active = if cb.weather_turns > 0 {
        (cb.weather_kind as u64) + 1
    } else {
        0
    };
    let w_field = ((cb.sides[s0].alive_mask.count_ones() <= 1) as u64)
        | (((cb.sides[s1].alive_mask.count_ones() <= 1) as u64) << 1)
        | (((cb.sides[s0].reflect_turns > 0) as u64) << 2)
        | (((cb.sides[s0].lightscreen_turns > 0) as u64) << 3)
        | (((cb.sides[s1].reflect_turns > 0) as u64) << 4)
        | (((cb.sides[s1].lightscreen_turns > 0) as u64) << 5)
        | (weather_active << 6)
        | (sub_bucket(m0, sp0) << 9)
        | (sub_bucket(m1, sp1) << 12);

    let w_dyn = (st0_packed as u64)
        | ((st1_packed as u64) << 8)
        | ((vol0 as u64) << 16)
        | ((vol1 as u64) << 32)
        | (w_field << 48);
    h.write_u64(w_dyn);

    h.finish()
}

#[inline]
fn l2_operator_key(max_s0: usize, max_s1: usize, spd_ord: u8, ops: &[Side1DOps; 2]) -> Key128 {
    let mut h = Hasher128::new();
    let max_s = [max_s0, max_s1];
    let side_hdr = |i: usize| -> u64 {
        let op = &ops[i];
        let ms = max_s[i];
        (op.has_heal as u64)
            | (((op.has_heal && op.heal_is_rest) as u64) << 1)
            | (((ms > 0 && op.has_rest) as u64) << 2)
            | (((ms > 0 && op.has_st) as u64) << 3)
            | (((op.has_heal && op.heal_is_rest && op.has_berry) as u64) << 4)
    };
    let hdr = (max_s0 as u64)
        | ((max_s1 as u64) << 2)
        | ((spd_ord as u64) << 4)
        | (side_hdr(0) << 8)
        | (side_hdr(1) << 16);
    h.write_u64(hdr);

    let pack_16 = |arr: &[u8; B16 + 1]| -> (u64, u64) {
        let lo = u64::from_le_bytes([
            arr[1], arr[2], arr[3], arr[4], arr[5], arr[6], arr[7], arr[8],
        ]);
        let hi = u64::from_le_bytes([
            arr[9], arr[10], arr[11], arr[12], arr[13], arr[14], arr[15], arr[16],
        ]);
        (lo, hi)
    };

    for i in 0..2 {
        let op = &ops[i];
        if op.has_heal && !op.heal_is_rest {
            let (lo, hi) = pack_16(&op.half_heal_bin);
            h.write_u64(lo);
            h.write_u64(hi);
        }
        let (e0_lo, e0_hi) = pack_16(&op.eot_bin[0]);
        h.write_u64(e0_lo);
        h.write_u64(e0_hi);
        if (max_s[i] > 0 && op.has_rest) || (op.has_heal && op.heal_is_rest) {
            let (e1_lo, e1_hi) = pack_16(&op.eot_bin[1]);
            h.write_u64(e1_lo);
            h.write_u64(e1_hi);
        }
        for b in 1..=B16 {
            let d = &op.incoming[1][b];
            h.write_u64((d.ko_prob.to_bits() as u64) | ((d.len as u64) << 32));
            for p_i in 0..d.len {
                let (nb, pr) = d.bins[p_i];
                h.write_u64((nb as u64) | ((pr.to_bits() as u64) << 8));
            }
        }
        if max_s[1 - i] > 0 && ops[1 - i].has_st {
            for b in 1..=B16 {
                let d = &op.incoming[2][b];
                h.write_u64((d.ko_prob.to_bits() as u64) | ((d.len as u64) << 32));
                for p_i in 0..d.len {
                    let (nb, pr) = d.bins[p_i];
                    h.write_u64((nb as u64) | ((pr.to_bits() as u64) << 8));
                }
            }
        }
    }

    h.finish()
}

#[allow(clippy::too_many_arguments)]
fn fill_ordered_q_plane(
    first: usize,
    slp0: usize,
    slp1: usize,
    ops: &[Side1DOps; 2],
    v_table: &[[[[f32; B16 + 1]; B16 + 1]; 3]; 3],
    second_pass_avg: bool,
    q_plane: &mut [[[[f32; 2]; 2]; B16 + 1]; B16 + 1],
) {
    fill_ordered_q_plane_ext(
        first,
        slp0,
        slp1,
        ops,
        v_table,
        GAMMA,
        0.5,
        1.0,
        0.0,
        second_pass_avg,
        q_plane,
    );
}

#[allow(clippy::too_many_arguments)]
fn fill_ordered_q_plane_ext(
    first: usize,
    slp0: usize,
    slp1: usize,
    ops: &[Side1DOps; 2],
    v_table: &[[[[f32; B16 + 1]; B16 + 1]; 3]; 3],
    gamma: f32,
    tie_val: f32,
    win0_val: f32,
    win1_val: f32,
    second_pass_avg: bool,
    q_plane: &mut [[[[f32; 2]; 2]; B16 + 1]; B16 + 1],
) {
    let second = 1 - first;
    let s = [slp0, slp1];
    let win_first = if first == 0 { win0_val } else { win1_val };
    let win_second = if first == 0 { win1_val } else { win0_val };
    let bias = (1.0 - gamma) * tie_val;

    let max_a_fir = if s[first] == 0 && ops[first].has_heal {
        2usize
    } else {
        1usize
    };
    let max_a_sec = if s[second] == 0 && ops[second].has_heal {
        2usize
    } else {
        1usize
    };

    for a_fir in 0..max_a_fir {
        let (ns_fir, cured_fir, k_fir) = if s[first] > 0 {
            (
                s[first] - 1,
                ops[first].has_rest,
                if ops[first].has_st { 2usize } else { 0usize },
            )
        } else if a_fir == 1 {
            if ops[first].heal_is_rest {
                (if ops[first].has_berry { 0 } else { 2 }, true, 0usize)
            } else {
                (0, false, 0usize)
            }
        } else {
            (0, false, 1usize)
        };
        let b_fir_max = if a_fir == 1 { B16 - 1 } else { B16 };
        let eot_fir = &ops[first].eot_bin[cured_fir as usize];

        for a_sec in 0..max_a_sec {
            let (ns_sec, cured_sec, k_sec, sec_mode) = if s[second] > 0 {
                (
                    s[second] - 1,
                    ops[second].has_rest,
                    if ops[second].has_st { 2usize } else { 0usize },
                    0u8,
                )
            } else if a_sec == 1 {
                if ops[second].heal_is_rest {
                    (if ops[second].has_berry { 0 } else { 2 }, true, 0usize, 1u8)
                } else {
                    (0, false, 0usize, 2u8)
                }
            } else {
                (0, false, 1usize, 0u8)
            };
            if second_pass_avg && k_fir == 0 && k_sec == 0 {
                continue;
            }
            let b_sec_max = if a_sec == 1 { B16 - 1 } else { B16 };
            let eot_sec = &ops[second].eot_bin[cured_sec as usize];
            let (ns0, ns1) = if first == 0 {
                (ns_fir, ns_sec)
            } else {
                (ns_sec, ns_fir)
            };
            let v_plane = &v_table[ns0][ns1];

            let mut end_of_m = [0u8; B16 + 1];
            if sec_mode == 2 {
                for m in 1..=B16 {
                    end_of_m[m] = eot_sec[ops[second].half_heal_bin[m] as usize];
                }
            } else if sec_mode == 0 {
                end_of_m = *eot_sec;
            }
            let (min_end, max_end) = if sec_mode == 1 {
                let e = eot_sec[B16] as usize;
                (e, e)
            } else if k_fir == 0 {
                (end_of_m[1] as usize, end_of_m[b_sec_max] as usize)
            } else {
                let min_m = ops[second].min_surv_bin[k_fir] as usize;
                let max_m = ops[second].max_surv_bin[k_fir] as usize;
                if min_m <= max_m {
                    (end_of_m[min_m] as usize, end_of_m[max_m] as usize)
                } else {
                    (1usize, 0usize)
                }
            };

            let mut prev_row_key = usize::MAX;
            let mut last_row = [0.0f32; B16 + 1];

            for b_fir in 1..=b_fir_max {
                let b_fir_mid = if s[first] == 0 && a_fir == 1 {
                    if ops[first].heal_is_rest {
                        B16
                    } else {
                        ops[first].half_heal_bin[b_fir] as usize
                    }
                } else {
                    b_fir
                };
                let row_key = if k_sec == 0 {
                    eot_fir[b_fir_mid] as usize
                } else if ops[first].incoming[k_sec][b_fir_mid].len == 0 {
                    0usize
                } else {
                    b_fir_mid
                };

                if row_key != prev_row_key {
                    prev_row_key = row_key;
                    let mut bv_end = [0.0f32; B16 + 1];
                    if min_end <= max_end {
                        if k_sec == 0 {
                            let b_fir_end = row_key;
                            if min_end == 0 {
                                bv_end[0] = if b_fir_end == 0 { tie_val } else { win_first };
                            }
                            let e_lo = min_end.max(1);
                            if e_lo <= max_end {
                                if b_fir_end == 0 {
                                    for cell in &mut bv_end[e_lo..=max_end] {
                                        *cell = win_second;
                                    }
                                } else if first == 0 {
                                    let v_row = &v_plane[b_fir_end];
                                    for end in e_lo..=max_end {
                                        bv_end[end] = gamma * v_row[end] + bias;
                                    }
                                } else {
                                    for end in e_lo..=max_end {
                                        bv_end[end] = gamma * v_plane[end][b_fir_end] + bias;
                                    }
                                }
                            }
                        } else {
                            let dist_fir = &ops[first].incoming[k_sec][b_fir_mid];
                            let base_ko = dist_fir.ko_prob * win_second;
                            for cell in &mut bv_end[min_end..=max_end] {
                                *cell = base_ko;
                            }
                            let e_lo = min_end.max(1);
                            for i_fir in 0..dist_fir.len {
                                let (b_fir_post_u8, p_fir) = dist_fir.bins[i_fir];
                                let b_fir_end = eot_fir[b_fir_post_u8 as usize] as usize;
                                if min_end == 0 {
                                    let cont0 = if b_fir_end == 0 { tie_val } else { win_first };
                                    bv_end[0] += p_fir * cont0;
                                }
                                if e_lo <= max_end {
                                    if b_fir_end == 0 {
                                        let add = p_fir * win_second;
                                        for cell in &mut bv_end[e_lo..=max_end] {
                                            *cell += add;
                                        }
                                    } else if first == 0 {
                                        let v_row = &v_plane[b_fir_end];
                                        for end in e_lo..=max_end {
                                            let cont = gamma * v_row[end] + bias;
                                            bv_end[end] += p_fir * cont;
                                        }
                                    } else {
                                        for end in e_lo..=max_end {
                                            let cont = gamma * v_plane[end][b_fir_end] + bias;
                                            bv_end[end] += p_fir * cont;
                                        }
                                    }
                                }
                            }
                        }
                    }

                    if sec_mode == 1 {
                        let bv = bv_end[min_end];
                        if k_fir == 0 {
                            let clamped = bv.clamp(0.0, 1.0);
                            for cell in &mut last_row[1..=b_sec_max] {
                                *cell = clamped;
                            }
                        } else {
                            for b_sec in 1..=b_sec_max {
                                let dist_sec = &ops[second].incoming[k_fir][b_sec];
                                let mut total_val = dist_sec.ko_prob * win_first;
                                for i_sec in 0..dist_sec.len {
                                    total_val += dist_sec.bins[i_sec].1 * bv;
                                }
                                last_row[b_sec] = total_val.clamp(0.0, 1.0);
                            }
                        }
                    } else if k_fir == 0 {
                        for b_sec in 1..=b_sec_max {
                            let end = end_of_m[b_sec] as usize;
                            last_row[b_sec] = bv_end[end].clamp(0.0, 1.0);
                        }
                    } else {
                        let min_m = ops[second].min_surv_bin[k_fir] as usize;
                        let max_m = ops[second].max_surv_bin[k_fir] as usize;
                        let mut bv_m = [0.0f32; B16 + 1];
                        if min_m <= max_m {
                            for m in min_m..=max_m {
                                bv_m[m] = bv_end[end_of_m[m] as usize];
                            }
                        }
                        for b_sec in 1..=b_sec_max {
                            let dist_sec = &ops[second].incoming[k_fir][b_sec];
                            let mut total_val = dist_sec.ko_prob * win_first;
                            for i_sec in 0..dist_sec.len {
                                let (m_u8, p_sec) = dist_sec.bins[i_sec];
                                total_val += p_sec * bv_m[m_u8 as usize];
                            }
                            last_row[b_sec] = total_val.clamp(0.0, 1.0);
                        }
                    }
                }

                if first == 0 {
                    if second_pass_avg {
                        for b_sec in 1..=b_sec_max {
                            let cell = &mut q_plane[b_fir][b_sec][a_fir][a_sec];
                            *cell = 0.5 * (*cell + last_row[b_sec]);
                        }
                    } else {
                        for b_sec in 1..=b_sec_max {
                            q_plane[b_fir][b_sec][a_fir][a_sec] = last_row[b_sec];
                        }
                    }
                } else if second_pass_avg {
                    for b_sec in 1..=b_sec_max {
                        let cell = &mut q_plane[b_sec][b_fir][a_sec][a_fir];
                        *cell = 0.5 * (*cell + last_row[b_sec]);
                    }
                } else {
                    for b_sec in 1..=b_sec_max {
                        q_plane[b_sec][b_fir][a_sec][a_fir] = last_row[b_sec];
                    }
                }
            }
        }
    }
}

/// Factored 1D + 1D forward push of non-terminal occupancy density `src_plane` through $P_{\pi^*}^\top$
/// for a fixed action-order `first` with weight `order_w` (`1.0` or `0.5` on speed tie).
#[allow(clippy::too_many_arguments)]
fn push_ordered_plane_forward(
    first: usize,
    order_w: f32,
    slp0: usize,
    slp1: usize,
    ops: &[Side1DOps; 2],
    src_plane: &[[f32; B16 + 1]; B16 + 1],
    p_heal0_plane: &[[u8; B16 + 1]; B16 + 1],
    p_heal1_plane: &[[u8; B16 + 1]; B16 + 1],
    dst: &mut [[[[f32; B16 + 1]; B16 + 1]; 3]; 3],
) {
    let second = 1 - first;
    let s = [slp0, slp1];
    let max_a_fir = if s[first] == 0 && ops[first].has_heal {
        2usize
    } else {
        1usize
    };
    let max_a_sec = if s[second] == 0 && ops[second].has_heal {
        2usize
    } else {
        1usize
    };

    for a_fir in 0..max_a_fir {
        let (ns_fir, cured_fir, k_fir) = if s[first] > 0 {
            (
                s[first] - 1,
                ops[first].has_rest,
                if ops[first].has_st { 2usize } else { 0usize },
            )
        } else if a_fir == 1 {
            if ops[first].heal_is_rest {
                (if ops[first].has_berry { 0 } else { 2 }, true, 0usize)
            } else {
                (0, false, 0usize)
            }
        } else {
            (0, false, 1usize)
        };
        let b_fir_max = if a_fir == 1 { B16 - 1 } else { B16 };
        let eot_fir = &ops[first].eot_bin[cured_fir as usize];

        for a_sec in 0..max_a_sec {
            let (ns_sec, cured_sec, k_sec, sec_mode) = if s[second] > 0 {
                (
                    s[second] - 1,
                    ops[second].has_rest,
                    if ops[second].has_st { 2usize } else { 0usize },
                    0u8,
                )
            } else if a_sec == 1 {
                if ops[second].heal_is_rest {
                    (if ops[second].has_berry { 0 } else { 2 }, true, 0usize, 1u8)
                } else {
                    (0, false, 0usize, 2u8)
                }
            } else {
                (0, false, 1usize, 0u8)
            };
            let b_sec_max = if a_sec == 1 { B16 - 1 } else { B16 };
            let eot_sec = &ops[second].eot_bin[cured_sec as usize];
            let (ns0, ns1) = if first == 0 {
                (ns_fir, ns_sec)
            } else {
                (ns_sec, ns_fir)
            };

            let mut end_of_m = [0u8; B16 + 1];
            if sec_mode == 2 {
                for m in 1..=B16 {
                    end_of_m[m] = eot_sec[ops[second].half_heal_bin[m] as usize];
                }
            } else if sec_mode == 0 {
                end_of_m = *eot_sec;
            }

            // Step A: push along `b_fir -> b_fir_end` (`1..=16`) keeping `b_sec` intact.
            // Note: `b_sec` must FIRST survive `first`'s attack `k_fir` before `second` even acts;
            // any branch where `b_sec` faints (`ko_prob`) or `b_fir_end == 0` or `b_sec_end == 0` is terminal.
            let mut mid = [[0.0f32; B16 + 1]; B16 + 1]; // mid[b_fir_end][b_sec]
            let mut any_mid = false;

            for b_fir in 1..=b_fir_max {
                let b_fir_mid = if s[first] == 0 && a_fir == 1 {
                    if ops[first].heal_is_rest {
                        B16
                    } else {
                        ops[first].half_heal_bin[b_fir] as usize
                    }
                } else {
                    b_fir
                };
                for b_sec in 1..=b_sec_max {
                    let (b0, b1) = if first == 0 {
                        (b_fir, b_sec)
                    } else {
                        (b_sec, b_fir)
                    };
                    let mass0 = src_plane[b0][b1];
                    if mass0 <= 1e-4 {
                        continue;
                    }
                    let ph0 = p_heal0_plane[b0][b1] as f32 * (1.0 / 255.0);
                    let ph1 = p_heal1_plane[b0][b1] as f32 * (1.0 / 255.0);
                    let pi0 = if s[0] == 0 && ops[0].has_heal && b0 < B16 {
                        if (if first == 0 { a_fir } else { a_sec }) == 1 {
                            ph0
                        } else {
                            1.0 - ph0
                        }
                    } else {
                        1.0
                    };
                    let pi1 = if s[1] == 0 && ops[1].has_heal && b1 < B16 {
                        if (if first == 1 { a_fir } else { a_sec }) == 1 {
                            ph1
                        } else {
                            1.0 - ph1
                        }
                    } else {
                        1.0
                    };
                    let w = mass0 * order_w * pi0 * pi1;
                    if w <= 1e-4 {
                        continue;
                    }
                    if k_sec == 0 {
                        let b_fir_end = eot_fir[b_fir_mid] as usize;
                        if b_fir_end > 0 {
                            mid[b_fir_end][b_sec] += w;
                            any_mid = true;
                        }
                    } else {
                        let dist_fir = &ops[first].incoming[k_sec][b_fir_mid];
                        for i_fir in 0..dist_fir.len {
                            let (b_fir_post_u8, p_fir) = dist_fir.bins[i_fir];
                            let b_fir_end = eot_fir[b_fir_post_u8 as usize] as usize;
                            if b_fir_end > 0 {
                                mid[b_fir_end][b_sec] += w * p_fir;
                                any_mid = true;
                            }
                        }
                    }
                }
            }

            if !any_mid {
                continue;
            }

            // Step B: push `b_sec` through `k_fir` + `sec_mode` + `eot_sec` into `dst[ns0][ns1]`.
            let dst_plane = &mut dst[ns0][ns1];
            let rest_end = eot_sec[B16] as usize;
            for b_fir_end in 1..=B16 {
                for b_sec in 1..=b_sec_max {
                    let mw = mid[b_fir_end][b_sec];
                    if mw <= 1e-4 {
                        continue;
                    }
                    if k_fir == 0 {
                        let b_sec_end = if sec_mode == 1 {
                            rest_end
                        } else {
                            end_of_m[b_sec] as usize
                        };
                        if b_sec_end > 0 {
                            if first == 0 {
                                dst_plane[b_fir_end][b_sec_end] += mw;
                            } else {
                                dst_plane[b_sec_end][b_fir_end] += mw;
                            }
                        }
                    } else {
                        let dist_sec = &ops[second].incoming[k_fir][b_sec];
                        if sec_mode == 1 {
                            if rest_end > 0 {
                                let mut p_surv = 0.0f32;
                                for i_sec in 0..dist_sec.len {
                                    p_surv += dist_sec.bins[i_sec].1;
                                }
                                let add = mw * p_surv;
                                if add > 0.0 {
                                    if first == 0 {
                                        dst_plane[b_fir_end][rest_end] += add;
                                    } else {
                                        dst_plane[rest_end][b_fir_end] += add;
                                    }
                                }
                            }
                        } else {
                            for i_sec in 0..dist_sec.len {
                                let (m_u8, p_sec) = dist_sec.bins[i_sec];
                                let b_sec_end = end_of_m[m_u8 as usize] as usize;
                                if b_sec_end > 0 {
                                    let add = mw * p_sec;
                                    if first == 0 {
                                        dst_plane[b_fir_end][b_sec_end] += add;
                                    } else {
                                        dst_plane[b_sec_end][b_fir_end] += add;
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

fn solve_bellman_into(
    max_s0: usize,
    max_s1: usize,
    spd_ord: u8,
    ops: &[Side1DOps; 2],
    out: &mut CachedBellman,
) {
    let max_s_sum = max_s0 + max_s1;
    out.v_table = [[[[0.5f32; B16 + 1]; B16 + 1]; 3]; 3];
    for slp0 in 0..=max_s0 {
        for slp1 in 0..=max_s1 {
            for b in 1..=B16 {
                out.v_table[slp0][slp1][0][b] = 0.0;
                out.v_table[slp0][slp1][b][0] = 1.0;
            }
        }
    }

    let mut q_plane = [[[[0.5f32; 2]; 2]; B16 + 1]; B16 + 1];
    let eval_plane_q = |slp0: usize,
                        slp1: usize,
                        vt: &[[[[f32; B16 + 1]; B16 + 1]; 3]; 3],
                        qp: &mut [[[[f32; 2]; 2]; B16 + 1]; B16 + 1]| {
        if slp1 > 0 && !ops[1].has_st {
            fill_ordered_q_plane(0, slp0, slp1, ops, vt, false, qp);
        } else if slp0 > 0 && !ops[0].has_st {
            fill_ordered_q_plane(1, slp0, slp1, ops, vt, false, qp);
        } else if spd_ord == 2 {
            fill_ordered_q_plane(0, slp0, slp1, ops, vt, false, qp);
        } else if spd_ord == 0 {
            fill_ordered_q_plane(1, slp0, slp1, ops, vt, false, qp);
        } else {
            fill_ordered_q_plane(0, slp0, slp1, ops, vt, false, qp);
            fill_ordered_q_plane(1, slp0, slp1, ops, vt, true, qp);
        }
    };

    let mut heal_mask0 = [0u32; B16 + 1];
    let mut heal_mask1 = [0u32; B16 + 1];

    for _iter in 0..18 {
        // 1. Bottom-up Schur-complement elimination over the sleeping DAG `s_sum = 1..=max_s_sum`.
        for s_sum in 1..=max_s_sum {
            for slp0 in 0..=max_s0 {
                for slp1 in 0..=max_s1 {
                    if slp0 + slp1 != s_sum {
                        continue;
                    }
                    if slp0 > 0 && slp1 > 0 && !ops[0].has_st && !ops[1].has_st {
                        let eot0 = &ops[0].eot_bin[ops[0].has_rest as usize];
                        let eot1 = &ops[1].eot_bin[ops[1].has_rest as usize];
                        let prev = out.v_table[slp0 - 1][slp1 - 1];
                        let dst = &mut out.v_table[slp0][slp1];
                        for b0 in 1..=B16 {
                            let e0 = eot0[b0] as usize;
                            if e0 == 0 {
                                for b1 in 1..=B16 {
                                    let e1 = eot1[b1] as usize;
                                    dst[b0][b1] = if e1 == 0 { 0.5 } else { 0.0 };
                                }
                            } else {
                                for b1 in 1..=B16 {
                                    let e1 = eot1[b1] as usize;
                                    dst[b0][b1] = if e1 == 0 {
                                        1.0
                                    } else {
                                        (GAMMA * prev[e0][e1] + (1.0 - GAMMA) * 0.5).clamp(0.0, 1.0)
                                    };
                                }
                            }
                        }
                        continue;
                    }
                    eval_plane_q(slp0, slp1, &out.v_table, &mut q_plane);
                    let c0 = slp0 == 0 && ops[0].has_heal;
                    let c1 = slp1 == 0 && ops[1].has_heal;
                    if !c0 && !c1 {
                        for b0 in 1..=B16 {
                            for b1 in 1..=B16 {
                                out.v_table[slp0][slp1][b0][b1] = q_plane[b0][b1][0][0];
                            }
                        }
                    } else {
                        for b0 in 1..=B16 {
                            let n0 = if c0 && b0 < B16 { 2 } else { 1 };
                            for b1 in 1..=B16 {
                                let n1 = if c1 && b1 < B16 { 2 } else { 1 };
                                let (val, _, _) = solve_stage_game_2x2(&q_plane[b0][b1], n0, n1);
                                out.v_table[slp0][slp1][b0][b1] = val;
                            }
                        }
                    }
                }
            }
        }

        // 2. Awake stage game `(slp0, slp1) = (0, 0)` using the Schur-eliminated sleeping values
        eval_plane_q(0, 0, &out.v_table, &mut q_plane);
        let mut next_awake = out.v_table[0][0];
        let mut max_diff = 0.0f32;
        let mut hm0 = [0u32; B16 + 1];
        let mut hm1 = [0u32; B16 + 1];

        for b0 in 1..=B16 {
            let n0 = if ops[0].has_heal && b0 < B16 { 2 } else { 1 };
            for b1 in 1..=B16 {
                let n1 = if ops[1].has_heal && b1 < B16 { 2 } else { 1 };
                let (val, h0, h1) = solve_stage_game_2x2(&q_plane[b0][b1], n0, n1);
                if h0 {
                    hm0[b0] |= 1u32 << b1;
                }
                if h1 {
                    hm1[b1] |= 1u32 << b0;
                }
                let d = (val - out.v_table[0][0][b0][b1]).abs();
                if d > max_diff {
                    max_diff = d;
                }
                next_awake[b0][b1] = val;
            }
        }
        out.v_table[0][0] = next_awake;
        heal_mask0 = hm0;
        heal_mask1 = hm1;
        if _iter >= 4 && max_diff < 2e-4 {
            break;
        }
    }

    out.heal_mask0 = heal_mask0;
    out.heal_mask1 = heal_mask1;
}

/// Lazily populate Stage-B finite-PP transient & stationary operator statistics (`w0_5`, `w1_5`,
/// `kappa`, `r_atk_act`, `r_heal_act`, `r_st_act`, `mean_hp_frac`) from solved `v_table`.
/// Invoked ONLY when a `1v1` or `2v1` leaf/rollout actually queries finite-PP endurance!
fn solve_bellman_finite_into(
    max_s0: usize,
    max_s1: usize,
    spd_ord: u8,
    ops: &[Side1DOps; 2],
    v_table: &[[[[f32; B16 + 1]; B16 + 1]; 3]; 3],
    out: &mut FinitePpBellman,
) {
    let mut q_plane = [[[[0.5f32; 2]; 2]; B16 + 1]; B16 + 1];
    let eval_plane_q = |slp0: usize,
                        slp1: usize,
                        vt: &[[[[f32; B16 + 1]; B16 + 1]; 3]; 3],
                        qp: &mut [[[[f32; 2]; 2]; B16 + 1]; B16 + 1]| {
        if slp1 > 0 && !ops[1].has_st {
            fill_ordered_q_plane(0, slp0, slp1, ops, vt, false, qp);
        } else if slp0 > 0 && !ops[0].has_st {
            fill_ordered_q_plane(1, slp0, slp1, ops, vt, false, qp);
        } else if spd_ord == 2 {
            fill_ordered_q_plane(0, slp0, slp1, ops, vt, false, qp);
        } else if spd_ord == 0 {
            fill_ordered_q_plane(1, slp0, slp1, ops, vt, false, qp);
        } else {
            fill_ordered_q_plane(0, slp0, slp1, ops, vt, false, qp);
            fill_ordered_q_plane(1, slp0, slp1, ops, vt, true, qp);
        }
    };

    let mut local_vt = *v_table;
    let max_s_sum = max_s0 + max_s1;
    let mut p_heal0 = [[[0u8; B16 + 1]; B16 + 1]; 3];
    let mut p_heal1 = [[[0u8; B16 + 1]; B16 + 1]; 3];
    for s_sum in 1..=max_s_sum {
        for slp0 in 0..=max_s0 {
            for slp1 in 0..=max_s1 {
                if slp0 + slp1 != s_sum {
                    continue;
                }
                if slp0 > 0 && slp1 > 0 && !ops[0].has_st && !ops[1].has_st {
                    let eot0 = &ops[0].eot_bin[ops[0].has_rest as usize];
                    let eot1 = &ops[1].eot_bin[ops[1].has_rest as usize];
                    let prev = local_vt[slp0 - 1][slp1 - 1];
                    let dst = &mut local_vt[slp0][slp1];
                    for b0 in 1..=B16 {
                        let e0 = eot0[b0] as usize;
                        if e0 == 0 {
                            for b1 in 1..=B16 {
                                let e1 = eot1[b1] as usize;
                                dst[b0][b1] = if e1 == 0 { 0.5 } else { 0.0 };
                            }
                        } else {
                            for b1 in 1..=B16 {
                                let e1 = eot1[b1] as usize;
                                dst[b0][b1] = if e1 == 0 {
                                    1.0
                                } else {
                                    (GAMMA * prev[e0][e1] + (1.0 - GAMMA) * 0.5).clamp(0.0, 1.0)
                                };
                            }
                        }
                    }
                    continue;
                }
                eval_plane_q(slp0, slp1, &local_vt, &mut q_plane);
                let c0 = slp0 == 0 && ops[0].has_heal;
                let c1 = slp1 == 0 && ops[1].has_heal;
                if !c0 && !c1 {
                    for b0 in 1..=B16 {
                        for b1 in 1..=B16 {
                            local_vt[slp0][slp1][b0][b1] = q_plane[b0][b1][0][0];
                        }
                    }
                } else {
                    for b0 in 1..=B16 {
                        let n0 = if c0 && b0 < B16 { 2 } else { 1 };
                        for b1 in 1..=B16 {
                            let n1 = if c1 && b1 < B16 { 2 } else { 1 };
                            let (val, _, _, ph0, ph1) =
                                solve_stage_game_2x2_mixed(&q_plane[b0][b1], n0, n1);
                            local_vt[slp0][slp1][b0][b1] = val;
                            if c0 {
                                p_heal0[slp1][b0][b1] = (ph0 * 255.0 + 0.5) as u8;
                            }
                            if c1 {
                                p_heal1[slp0][b0][b1] = (ph1 * 255.0 + 0.5) as u8;
                            }
                        }
                    }
                }
            }
        }
    }
    if ops[0].has_heal || ops[1].has_heal {
        eval_plane_q(0, 0, &local_vt, &mut q_plane);
        for b0 in 1..=B16 {
            let n0 = if ops[0].has_heal && b0 < B16 { 2 } else { 1 };
            for b1 in 1..=B16 {
                let n1 = if ops[1].has_heal && b1 < B16 { 2 } else { 1 };
                let (_, _, _, ph0, ph1) = solve_stage_game_2x2_mixed(&q_plane[b0][b1], n0, n1);
                p_heal0[0][b0][b1] = (ph0 * 255.0 + 0.5) as u8;
                p_heal1[0][b0][b1] = (ph1 * 255.0 + 0.5) as u8;
            }
        }
    }

    // 1. Exact 5-turn undiscounted transient win tables `(w0_5, w1_5)` under equilibrium policy `pi*`
    //    (combined with exact 1-step root expansion, total fast horizon = K_0 = 6 undiscounted turns).
    let eval_plane_undisc = |slp0: usize,
                             slp1: usize,
                             win0_val: f32,
                             win1_val: f32,
                             wt: &[[[[f32; B16 + 1]; B16 + 1]; 3]; 3],
                             qp: &mut [[[[f32; 2]; 2]; B16 + 1]; B16 + 1]| {
        if slp1 > 0 && !ops[1].has_st {
            fill_ordered_q_plane_ext(0, slp0, slp1, ops, wt, 1.0, 0.5, win0_val, win1_val, false, qp);
        } else if slp0 > 0 && !ops[0].has_st {
            fill_ordered_q_plane_ext(1, slp0, slp1, ops, wt, 1.0, 0.5, win0_val, win1_val, false, qp);
        } else if spd_ord == 2 {
            fill_ordered_q_plane_ext(0, slp0, slp1, ops, wt, 1.0, 0.5, win0_val, win1_val, false, qp);
        } else if spd_ord == 0 {
            fill_ordered_q_plane_ext(1, slp0, slp1, ops, wt, 1.0, 0.5, win0_val, win1_val, false, qp);
        } else {
            fill_ordered_q_plane_ext(0, slp0, slp1, ops, wt, 1.0, 0.5, win0_val, win1_val, false, qp);
            fill_ordered_q_plane_ext(1, slp0, slp1, ops, wt, 1.0, 0.5, win0_val, win1_val, true, qp);
        }
    };

    let step_policy_undisc = |win0_val: f32,
                              win1_val: f32,
                              prev_w: &[[[[f32; B16 + 1]; B16 + 1]; 3]; 3],
                              qp: &mut [[[[f32; 2]; 2]; B16 + 1]; B16 + 1]|
     -> [[[[f32; B16 + 1]; B16 + 1]; 3]; 3] {
        let mut next_w = [[[[0.0f32; B16 + 1]; B16 + 1]; 3]; 3];
        for slp0 in 0..=max_s0 {
            for slp1 in 0..=max_s1 {
                if slp0 > 0 && slp1 > 0 && !ops[0].has_st && !ops[1].has_st {
                    let eot0 = &ops[0].eot_bin[ops[0].has_rest as usize];
                    let eot1 = &ops[1].eot_bin[ops[1].has_rest as usize];
                    let pr = &prev_w[slp0 - 1][slp1 - 1];
                    let dst = &mut next_w[slp0][slp1];
                    for b0 in 1..=B16 {
                        let e0 = eot0[b0] as usize;
                        for b1 in 1..=B16 {
                            let e1 = eot1[b1] as usize;
                            dst[b0][b1] = if e0 == 0 && e1 == 0 {
                                0.5
                            } else if e0 == 0 {
                                win1_val
                            } else if e1 == 0 {
                                win0_val
                            } else {
                                pr[e0][e1]
                            };
                        }
                    }
                    continue;
                }
                eval_plane_undisc(slp0, slp1, win0_val, win1_val, prev_w, qp);
                let c0 = slp0 == 0 && ops[0].has_heal;
                let c1 = slp1 == 0 && ops[1].has_heal;
                for b0 in 1..=B16 {
                    let n0 = if c0 && b0 < B16 { 2 } else { 1 };
                    for b1 in 1..=B16 {
                        let n1 = if c1 && b1 < B16 { 2 } else { 1 };
                        let ph0 = if n0 == 2 {
                            p_heal0[slp1][b0][b1] as f32 * (1.0 / 255.0)
                        } else {
                            0.0
                        };
                        let ph1 = if n1 == 2 {
                            p_heal1[slp0][b0][b1] as f32 * (1.0 / 255.0)
                        } else {
                            0.0
                        };
                        let q = &qp[b0][b1];
                        let val = (1.0 - ph0) * ((1.0 - ph1) * q[0][0] + ph1 * q[0][1])
                            + ph0 * ((1.0 - ph1) * q[1][0] + ph1 * q[1][1]);
                        next_w[slp0][slp1][b0][b1] = val.clamp(0.0, 1.0);
                    }
                }
            }
        }
        next_w
    };

    let zero_tbl = [[[[0.0f32; B16 + 1]; B16 + 1]; 3]; 3];
    let abs0 = step_policy_undisc(1.0, 0.0, &zero_tbl, &mut q_plane);
    let abs1 = step_policy_undisc(0.0, 1.0, &zero_tbl, &mut q_plane);
    let mut w0 = abs0;
    let mut w1 = abs1;
    for _ in 1..5 {
        w0 = step_policy_undisc(1.0, 0.0, &w0, &mut q_plane);
        w1 = step_policy_undisc(0.0, 1.0, &w1, &mut q_plane);
    }

    let mut max_psi = 0.0f32;
    for slp0 in 0..=max_s0 {
        for slp1 in 0..=max_s1 {
            for b0 in 1..=B16 {
                for b1 in 1..=B16 {
                    let psi = 1.0 - w0[slp0][slp1][b0][b1] - w1[slp0][slp1][b0][b1];
                    if psi > max_psi {
                        max_psi = psi;
                    }
                    out.w0_5[slp0][slp1][b0][b1] =
                        (w0[slp0][slp1][b0][b1].clamp(0.0, 1.0) * 65535.0 + 0.5) as u16;
                    out.w1_5[slp0][slp1][b0][b1] =
                        (w1[slp0][slp1][b0][b1].clamp(0.0, 1.0) * 65535.0 + 0.5) as u16;
                }
            }
        }
    }
    if max_psi < 1e-3 {
        out.kappa = [0.0, 0.0];
        out.r_atk_act = [1.0, 1.0];
        out.r_heal_act = [0.0, 0.0];
        out.r_st_act = [0.0, 0.0];
        out.mean_hp_frac = [0.5, 0.5];
        return;
    }

    // 2. Sparse symmetric occupancy attractor `\bar{\mu}`: 6-step burn-in (2 Rest cycles)
    //    followed by exact 6-step LCM(1,2,3) cycle accumulation (`12` sparse pushes total).
    let zero_u8_plane = [[0u8; B16 + 1]; B16 + 1];
    let push_all = |src: &[[[[f32; B16 + 1]; B16 + 1]; 3]; 3]|
     -> [[[[f32; B16 + 1]; B16 + 1]; 3]; 3] {
        let mut dst = [[[[0.0f32; B16 + 1]; B16 + 1]; 3]; 3];
        for slp0 in 0..=max_s0 {
            for slp1 in 0..=max_s1 {
                let sp = &src[slp0][slp1];
                let mut plane_has_mass = false;
                'check: for b0 in 1..=B16 {
                    for b1 in 1..=B16 {
                        if sp[b0][b1] > 1e-4 {
                            plane_has_mass = true;
                            break 'check;
                        }
                    }
                }
                if !plane_has_mass {
                    continue;
                }
                if slp0 > 0 && slp1 > 0 && !ops[0].has_st && !ops[1].has_st {
                    let eot0 = &ops[0].eot_bin[ops[0].has_rest as usize];
                    let eot1 = &ops[1].eot_bin[ops[1].has_rest as usize];
                    let dp = &mut dst[slp0 - 1][slp1 - 1];
                    for b0 in 1..=B16 {
                        let e0 = eot0[b0] as usize;
                        if e0 == 0 {
                            continue;
                        }
                        for b1 in 1..=B16 {
                            let e1 = eot1[b1] as usize;
                            if e1 > 0 {
                                dp[e0][e1] += sp[b0][b1];
                            }
                        }
                    }
                    continue;
                }
                let ph0_p = if slp0 == 0 { &p_heal0[slp1] } else { &zero_u8_plane };
                let ph1_p = if slp1 == 0 { &p_heal1[slp0] } else { &zero_u8_plane };
                if slp1 > 0 && !ops[1].has_st {
                    push_ordered_plane_forward(0, 1.0, slp0, slp1, ops, sp, ph0_p, ph1_p, &mut dst);
                } else if slp0 > 0 && !ops[0].has_st {
                    push_ordered_plane_forward(1, 1.0, slp0, slp1, ops, sp, ph0_p, ph1_p, &mut dst);
                } else if spd_ord == 2 {
                    push_ordered_plane_forward(0, 1.0, slp0, slp1, ops, sp, ph0_p, ph1_p, &mut dst);
                } else if spd_ord == 0 {
                    push_ordered_plane_forward(1, 1.0, slp0, slp1, ops, sp, ph0_p, ph1_p, &mut dst);
                } else {
                    push_ordered_plane_forward(0, 0.5, slp0, slp1, ops, sp, ph0_p, ph1_p, &mut dst);
                    push_ordered_plane_forward(1, 0.5, slp0, slp1, ops, sp, ph0_p, ph1_p, &mut dst);
                }
            }
        }
        dst
    };

    let mut d_init = [[[[0.0f32; B16 + 1]; B16 + 1]; 3]; 3];
    let inv_256 = 1.0f32 / ((B16 * B16) as f32);
    for b0 in 1..=B16 {
        for b1 in 1..=B16 {
            d_init[0][0][b0][b1] = inv_256;
        }
    }
    let mut d_burn = d_init;
    for _ in 0..6 {
        d_burn = push_all(&d_burn);
    }
    let mut burn_sum = 0.0f32;
    for slp0 in 0..=max_s0 {
        for slp1 in 0..=max_s1 {
            for b0 in 1..=B16 {
                for b1 in 1..=B16 {
                    burn_sum += d_burn[slp0][slp1][b0][b1];
                }
            }
        }
    }
    let mut d_cur = if burn_sum > 1e-6 {
        let inv = 1.0 / burn_sum;
        for slp0 in 0..=max_s0 {
            for slp1 in 0..=max_s1 {
                for b0 in 1..=B16 {
                    for b1 in 1..=B16 {
                        d_burn[slp0][slp1][b0][b1] *= inv;
                    }
                }
            }
        }
        d_burn
    } else {
        d_init
    };

    let mut nu = d_cur;
    let mut disc_w = GAMMA_RESOLVENT;
    for _ in 0..5 {
        d_cur = push_all(&d_cur);
        for slp0 in 0..=max_s0 {
            for slp1 in 0..=max_s1 {
                for b0 in 1..=B16 {
                    for b1 in 1..=B16 {
                        nu[slp0][slp1][b0][b1] += disc_w * d_cur[slp0][slp1][b0][b1];
                    }
                }
            }
        }
        disc_w *= GAMMA_RESOLVENT;
    }

    let mut total_mu = 0.0f64;
    for slp0 in 0..=max_s0 {
        for slp1 in 0..=max_s1 {
            for b0 in 1..=B16 {
                for b1 in 1..=B16 {
                    total_mu += nu[slp0][slp1][b0][b1] as f64;
                }
            }
        }
    }
    if total_mu <= 1e-12 {
        out.kappa = [0.0, 0.0];
        out.r_atk_act = [1.0, 1.0];
        out.r_heal_act = [0.0, 0.0];
        out.r_st_act = [0.0, 0.0];
        out.mean_hp_frac = [0.5, 0.5];
        return;
    }

    let inv_mu = 1.0 / total_mu;
    let mut kap0 = 0.0f64;
    let mut kap1 = 0.0f64;
    let mut ratk = [0.0f64; 2];
    let mut rheal = [0.0f64; 2];
    let mut rst = [0.0f64; 2];
    let mut mhp = [0.0f64; 2];

    for slp0 in 0..=max_s0 {
        for slp1 in 0..=max_s1 {
            for b0 in 1..=B16 {
                for b1 in 1..=B16 {
                    let mu = (nu[slp0][slp1][b0][b1] as f64) * inv_mu;
                    if mu <= 0.0 {
                        continue;
                    }
                    kap0 += mu * (abs0[slp0][slp1][b0][b1] as f64);
                    kap1 += mu * (abs1[slp0][slp1][b0][b1] as f64);
                    mhp[0] += mu * (b0 as f64 / B16 as f64);
                    mhp[1] += mu * (b1 as f64 / B16 as f64);

                    if slp0 > 0 {
                        if ops[0].has_st {
                            rst[0] += mu;
                        }
                    } else {
                        let ph0 = if ops[0].has_heal && b0 < B16 {
                            p_heal0[slp1][b0][b1] as f64 * (1.0 / 255.0)
                        } else {
                            0.0
                        };
                        rheal[0] += mu * ph0;
                        ratk[0] += mu * (1.0 - ph0);
                    }

                    if slp1 > 0 {
                        if ops[1].has_st {
                            rst[1] += mu;
                        }
                    } else {
                        let ph1 = if ops[1].has_heal && b1 < B16 {
                            p_heal1[slp0][b0][b1] as f64 * (1.0 / 255.0)
                        } else {
                            0.0
                        };
                        rheal[1] += mu * ph1;
                        ratk[1] += mu * (1.0 - ph1);
                    }
                }
            }
        }
    }

    out.kappa = [kap0.max(0.0) as f32, kap1.max(0.0) as f32];
    out.r_atk_act = [ratk[0].clamp(0.0, 1.0) as f32, ratk[1].clamp(0.0, 1.0) as f32];
    out.r_heal_act = [rheal[0].clamp(0.0, 1.0) as f32, rheal[1].clamp(0.0, 1.0) as f32];
    out.r_st_act = [rst[0].clamp(0.0, 1.0) as f32, rst[1].clamp(0.0, 1.0) as f32];
    out.mean_hp_frac = [mhp[0].clamp(0.05, 1.0) as f32, mhp[1].clamp(0.05, 1.0) as f32];
}

#[inline]
fn solve_stage_game_2x2(q: &[[f32; 2]; 2], n0: usize, n1: usize) -> (f32, bool, bool) {
    if n0 == 1 && n1 == 1 {
        return (q[0][0], false, false);
    }
    if n0 == 2 && n1 == 1 {
        let heal0 = q[1][0] > q[0][0] + 1e-5;
        return (q[0][0].max(q[1][0]), heal0, false);
    }
    if n0 == 1 && n1 == 2 {
        let heal1 = q[0][1] < q[0][0] - 1e-5;
        return (q[0][0].min(q[0][1]), false, heal1);
    }
    let r0 = q[0][0].min(q[0][1]);
    let r1 = q[1][0].min(q[1][1]);
    let maximin = r0.max(r1);

    let c0 = q[0][0].max(q[1][0]);
    let c1 = q[0][1].max(q[1][1]);
    let minimax = c0.min(c1);

    let heal0 = r1 > r0 + 1e-5;
    let heal1 = c1 < c0 - 1e-5;

    if (minimax - maximin).abs() <= 1e-5 {
        return (0.5 * (maximin + minimax), heal0, heal1);
    }
    let denom = q[0][0] + q[1][1] - q[0][1] - q[1][0];
    if denom.abs() <= 1e-6 {
        return (0.5 * (maximin + minimax), heal0, heal1);
    }
    let val = ((q[0][0] * q[1][1] - q[0][1] * q[1][0]) / denom).clamp(maximin, minimax);
    (val, heal0, heal1)
}

#[inline]
fn solve_stage_game_2x2_mixed(
    q: &[[f32; 2]; 2],
    n0: usize,
    n1: usize,
) -> (f32, bool, bool, f32, f32) {
    if n0 == 1 && n1 == 1 {
        return (q[0][0], false, false, 0.0, 0.0);
    }
    let c00 = (q[0][0] as f64) - 0.5;
    let c01 = (q[0][1] as f64) - 0.5;
    let c10 = (q[1][0] as f64) - 0.5;
    let c11 = (q[1][1] as f64) - 0.5;
    if n0 == 2 && n1 == 1 {
        let heal0 = c10 > c00 + 1e-5;
        let ph0 = if heal0 { 1.0 } else { 0.0 };
        return ((0.5 + c00.max(c10)) as f32, heal0, false, ph0, 0.0);
    }
    if n0 == 1 && n1 == 2 {
        let heal1 = c01 < c00 - 1e-5;
        let ph1 = if heal1 { 1.0 } else { 0.0 };
        return ((0.5 + c00.min(c01)) as f32, false, heal1, 0.0, ph1);
    }
    let r0 = c00.min(c01);
    let r1 = c10.min(c11);
    let maximin = r0.max(r1);

    let k0 = c00.max(c10);
    let k1 = c01.max(c11);
    let minimax = k0.min(k1);

    let heal0 = r1 > r0 + 1e-5;
    let heal1 = k1 < k0 - 1e-5;
    let ph0_pure = if heal0 { 1.0f32 } else { 0.0f32 };
    let ph1_pure = if heal1 { 1.0f32 } else { 0.0f32 };

    if (minimax - maximin).abs() <= 1e-5 {
        return (
            (0.5 + 0.5 * (maximin + minimax)) as f32,
            heal0,
            heal1,
            ph0_pure,
            ph1_pure,
        );
    }
    let denom = c00 + c11 - c01 - c10;
    if denom.abs() <= 1e-7 {
        return (
            (0.5 + 0.5 * (maximin + minimax)) as f32,
            heal0,
            heal1,
            ph0_pure,
            ph1_pure,
        );
    }
    let val_c = ((c00 * c11 - c01 * c10) / denom).clamp(maximin, minimax);
    let p0 = ((c00 - c01) / denom).clamp(0.0, 1.0) as f32;
    let p1 = ((c00 - c10) / denom).clamp(0.0, 1.0) as f32;
    ((0.5 + val_c) as f32, heal0, heal1, p0, p1)
}

#[inline(never)]
fn miss_base_slot(
    cache: &mut BellmanCache,
    cb: &CompactBattle,
    table: &MatchTable,
    s0: usize,
    slot0: usize,
    s1: usize,
    slot1: usize,
    l1_key: Key128,
) -> (usize, u32) {
    let (ops, max_s0, max_s1, spd_ord) = Side1DOps::build_pair(cb, table, s0, slot0, s1, slot1);
    let op_key = l2_operator_key(max_s0, max_s1, spd_ord, &ops);
    let l2_idx = (op_key.0 as usize) & (L2_SETS - 1);

    {
        let set_l2 = &mut cache.l2[l2_idx];
        for way in 0..L2_WAYS {
            if set_l2.keys[way] == op_key {
                let slot = set_l2.slots[way];
                if way > 0 {
                    for w in (1..=way).rev() {
                        set_l2.keys[w] = set_l2.keys[w - 1];
                        set_l2.slots[w] = set_l2.slots[w - 1];
                    }
                    set_l2.keys[0] = op_key;
                    set_l2.slots[0] = slot;
                }
                let gen = cache.slot_gen[slot as usize];
                cache.record_l1(l1_key, slot, gen);
                return (slot as usize, gen);
            }
        }
    }

    let last_empty = cache.l2[l2_idx].keys[L2_WAYS - 1] == Key128::default();
    let slot_idx = if last_empty && cache.pool.len() < MAX_CACHE_SLOTS {
        let idx = cache.pool.len();
        cache.pool.push(CachedBellman::empty());
        cache.masks.push(CachedBellmanMasks::empty());
        idx
    } else {
        cache.l2[l2_idx].slots[L2_WAYS - 1] as usize
    };
    let gen = cache.slot_gen[slot_idx].wrapping_add(1).max(1);
    cache.slot_gen[slot_idx] = gen;
    cache.slot_finite_gen[slot_idx] = 0;

    {
        let set_l2 = &mut cache.l2[l2_idx];
        for w in (1..L2_WAYS).rev() {
            set_l2.keys[w] = set_l2.keys[w - 1];
            set_l2.slots[w] = set_l2.slots[w - 1];
        }
        set_l2.keys[0] = op_key;
        set_l2.slots[0] = slot_idx as u16;
    }

    cache.pool[slot_idx] = CachedBellman::empty();
    cache.pool[slot_idx].op_key = op_key;
    solve_bellman_into(max_s0, max_s1, spd_ord, &ops, &mut cache.pool[slot_idx]);
    cache.masks[slot_idx] = CachedBellmanMasks {
        heal_mask0: cache.pool[slot_idx].heal_mask0,
        heal_mask1: cache.pool[slot_idx].heal_mask1,
    };
    cache.record_l1(l1_key, slot_idx as u16, gen);
    (slot_idx, gen)
}

#[inline(never)]
fn populate_finite_slot(
    cache: &mut BellmanCache,
    cb: &CompactBattle,
    table: &MatchTable,
    s0: usize,
    slot0: usize,
    s1: usize,
    slot1: usize,
    slot_idx: usize,
    gen: u32,
) -> usize {
    let existing = cache.slot_finite_idx[slot_idx] as usize;
    let f_idx = if existing < cache.finite_pool.len() {
        existing
    } else {
        let idx = cache.finite_pool.len();
        cache.finite_pool.push(FinitePpBellman::empty());
        cache.slot_finite_idx[slot_idx] = idx as u16;
        idx
    };
    let (ops, max_s0, max_s1, spd_ord) = Side1DOps::build_pair(cb, table, s0, slot0, s1, slot1);
    let vt = cache.pool[slot_idx].v_table;
    solve_bellman_finite_into(
        max_s0,
        max_s1,
        spd_ord,
        &ops,
        &vt,
        &mut cache.finite_pool[f_idx],
    );
    let idx0 = s0 * 3 + slot0;
    let idx1 = s1 * 3 + slot1;
    let strip0 = cb.mons[idx0].item != 0
        && (table.mons[idx0].is_mintberry || table.mons[idx0].is_miracleberry)
        && (table.mons[idx0].role_flags & role_bit::REST) != 0;
    let strip1 = cb.mons[idx1].item != 0
        && (table.mons[idx1].is_mintberry || table.mons[idx1].is_miracleberry)
        && (table.mons[idx1].role_flags & role_bit::REST) != 0;
    if strip0 || strip1 {
        let mut cb_post = *cb;
        if strip0 {
            cb_post.mons[idx0].item = 0;
        }
        if strip1 {
            cb_post.mons[idx1].item = 0;
        }
        let (ops_p, ms0_p, ms1_p, spd_p) =
            Side1DOps::build_pair(&cb_post, table, s0, slot0, s1, slot1);
        let mut post_bellman = CachedBellman::empty();
        solve_bellman_into(ms0_p, ms1_p, spd_p, &ops_p, &mut post_bellman);
        let mut post_fin = FinitePpBellman::empty();
        solve_bellman_finite_into(
            ms0_p,
            ms1_p,
            spd_p,
            &ops_p,
            &post_bellman.v_table,
            &mut post_fin,
        );
        let dst = &mut cache.finite_pool[f_idx];
        dst.kappa = post_fin.kappa;
        dst.r_atk_act = post_fin.r_atk_act;
        dst.r_heal_act = post_fin.r_heal_act;
        dst.r_st_act = post_fin.r_st_act;
        dst.mean_hp_frac = post_fin.mean_hp_frac;
    }
    cache.slot_finite_gen[slot_idx] = gen;
    f_idx
}

#[inline]
fn lookup_l1_slot(cache: &mut BellmanCache, l1_key: Key128) -> Option<(usize, u32, u16)> {
    let l1_idx = (l1_key.0 as usize) & (L1_SETS - 1);
    let set = cache.l1[l1_idx];
    for way in 0..L1_WAYS {
        let w_entry = set[way];
        if w_entry.key == l1_key {
            let slot = w_entry.slot as usize;
            if slot < cache.pool.len() && cache.slot_gen[slot] == w_entry.gen {
                if way > 0 {
                    for w in (1..=way).rev() {
                        cache.l1[l1_idx][w] = set[w - 1];
                    }
                    cache.l1[l1_idx][0] = w_entry;
                }
                return Some((slot, w_entry.gen, w_entry.meta_slot));
            }
        }
    }
    None
}

#[inline]
fn with_solved_bellman_masks<R>(
    cb: &CompactBattle,
    table: &MatchTable,
    s0: usize,
    slot0: usize,
    s1: usize,
    slot1: usize,
    f: impl FnOnce(&CachedBellmanMasks) -> R,
) -> R {
    let l1_key = l1_state_key(cb, table, s0, slot0, s1, slot1);
    BELLMAN_CACHE.with(|c| {
        let mut cache = c.borrow_mut();
        let slot_idx = match lookup_l1_slot(&mut cache, l1_key) {
            Some((s, _, _)) => s,
            None => miss_base_slot(&mut cache, cb, table, s0, slot0, s1, slot1, l1_key).0,
        };
        f(&cache.masks[slot_idx])
    })
}

#[inline]
fn with_solved_bellman<R>(
    cb: &CompactBattle,
    table: &MatchTable,
    s0: usize,
    slot0: usize,
    s1: usize,
    slot1: usize,
    f: impl FnOnce(&CachedBellman) -> R,
) -> R {
    let l1_key = l1_state_key(cb, table, s0, slot0, s1, slot1);
    BELLMAN_CACHE.with(|c| {
        let mut cache = c.borrow_mut();
        let slot_idx = match lookup_l1_slot(&mut cache, l1_key) {
            Some((s, _, _)) => s,
            None => miss_base_slot(&mut cache, cb, table, s0, slot0, s1, slot1, l1_key).0,
        };
        f(&cache.pool[slot_idx])
    })
}

#[inline(never)]
fn miss_meta_slot(
    cache: &mut BellmanCache,
    cb: &CompactBattle,
    table: &MatchTable,
    s0: usize,
    slot0: usize,
    s1: usize,
    slot1: usize,
    l1_key: Key128,
) -> usize {
    let m_set_idx = (l1_key.1 as usize) & (META_SETS - 1);
    let set_m = cache.meta_sets[m_set_idx];
    for way in 0..META_WAYS {
        if set_m[way].key == l1_key {
            let slot = set_m[way].slot as usize;
            if way > 0 {
                for w in (1..=way).rev() {
                    cache.meta_sets[m_set_idx][w] = set_m[w - 1];
                }
                cache.meta_sets[m_set_idx][0] = set_m[way];
            }
            let l1_idx = (l1_key.0 as usize) & (L1_SETS - 1);
            if cache.l1[l1_idx][0].key == l1_key {
                cache.l1[l1_idx][0].meta_slot = slot as u16;
            }
            return slot;
        }
    }
    let evicted = cache.meta_sets[m_set_idx][META_WAYS - 1];
    let last_empty = evicted.key == Key128::default();
    let meta_val = L1PairFastMeta::build(cb, table, s0, slot0, s1, slot1);
    let m_idx = if last_empty && cache.meta_pool.len() < MAX_META_SLOTS {
        let idx = cache.meta_pool.len();
        cache.meta_pool.push(meta_val);
        idx
    } else {
        if !last_empty {
            let ev_l1 = (evicted.key.0 as usize) & (L1_SETS - 1);
            for w in 0..L1_WAYS {
                if cache.l1[ev_l1][w].key == evicted.key {
                    cache.l1[ev_l1][w].meta_slot = u16::MAX;
                }
            }
        }
        let idx = evicted.slot as usize;
        cache.meta_pool[idx] = meta_val;
        idx
    };
    let set_m_mut = &mut cache.meta_sets[m_set_idx];
    for w in (1..META_WAYS).rev() {
        set_m_mut[w] = set_m_mut[w - 1];
    }
    set_m_mut[0] = MetaWay {
        key: l1_key,
        slot: m_idx as u16,
    };
    let l1_idx = (l1_key.0 as usize) & (L1_SETS - 1);
    if cache.l1[l1_idx][0].key == l1_key {
        cache.l1[l1_idx][0].meta_slot = m_idx as u16;
    }
    m_idx
}

#[inline]
fn with_solved_bellman_finite<R>(
    cb: &CompactBattle,
    table: &MatchTable,
    s0: usize,
    slot0: usize,
    s1: usize,
    slot1: usize,
    f: impl FnOnce(&FinitePpBellman, &L1PairFastMeta) -> R,
) -> R {
    let l1_key = l1_state_key(cb, table, s0, slot0, s1, slot1);
    BELLMAN_CACHE.with(|c| {
        let mut cache = c.borrow_mut();
        let (slot_idx, gen, meta_u16) = match lookup_l1_slot(&mut cache, l1_key) {
            Some(hit) => hit,
            None => {
                let (s, g) = miss_base_slot(&mut cache, cb, table, s0, slot0, s1, slot1, l1_key);
                (s, g, u16::MAX)
            }
        };
        let f_idx = if cache.slot_finite_gen[slot_idx] == gen {
            cache.slot_finite_idx[slot_idx] as usize
        } else {
            populate_finite_slot(&mut cache, cb, table, s0, slot0, s1, slot1, slot_idx, gen)
        };
        let m_idx = if (meta_u16 as usize) < cache.meta_pool.len() {
            meta_u16 as usize
        } else {
            miss_meta_slot(&mut cache, cb, table, s0, slot0, s1, slot1, l1_key)
        };
        f(&cache.finite_pool[f_idx], &cache.meta_pool[m_idx])
    })
}

#[inline]
fn side_sleep_rem(cb: &CompactBattle, table: &MatchTable, side: usize, slot: usize) -> usize {
    let idx = side * 3 + slot;
    let mon = &cb.mons[idx];
    let spec = &table.mons[idx];
    let berry = mon.item != 0 && (spec.is_mintberry || spec.is_miracleberry);
    if mon.status_raw() == Status::Slp as u8 && !berry {
        (mon.status_counter().saturating_sub(1) as usize).min(2)
    } else {
        0
    }
}

#[inline]
fn side_has_usable_heal_ext(
    cb: &CompactBattle,
    table: &MatchTable,
    side: usize,
    slot: usize,
    stall_pp: bool,
) -> bool {
    let idx = side * 3 + slot;
    let mon = &cb.mons[idx];
    if (mon.vol_flags & vol_bit::PERISH_SONG) != 0 {
        return false;
    }
    let dis = (mon.vol_flags & vol_bit::DISABLE) != 0;
    let dis_slot = ((mon.slot_refs >> 4) & 0x03) as usize;
    for m_i in 0..table.mons[idx].num_moves as usize {
        if mon.pp[m_i] > 0 && !(dis && m_i == dis_slot) {
            let ms = &table.move_specs[idx][m_i];
            if ms.is_half_heal || ms.is_rest || (stall_pp && ms.special_kind == 29) {
                return true;
            }
        }
    }
    false
}

#[derive(Clone, Copy)]
struct SidePpProfile {
    slot_kind: [u8; 4],
    slot_pp: [f32; 4],
    pp_inst_heal: f64,
    pp_rest: f64,
    pp_st: f64,
    pp_brk: f64,
    pp_sub_atk: f64,
    pp_cons_aux: f64,
    pp_self_cost: f64,
    pp_spite: f64,
    brk_dmg_frac: f64,
    sub_dmg_frac: f64,
    max_dmg_hp: i16,
    lefties_hp: i16,
    net_eot_frac: f64,
    has_passive_dot: bool,
    p_deduct: f64,
    has_berry: bool,
    can_heal: bool,
    heal_is_rest: bool,
    had_brk_initially: bool,
    inst_heal_amt: i16,
}

impl SideFastMeta {
    fn build(
        cb: &CompactBattle,
        table: &MatchTable,
        side: usize,
        slot: usize,
        foe_side: usize,
        foe_slot: usize,
    ) -> Self {
        let idx = side * 3 + slot;
        let foe_idx = foe_side * 3 + foe_slot;
        let mon = &cb.mons[idx];
        let foe_mon = &cb.mons[foe_idx];
        let spec = &table.mons[idx];
        let foe_spec = &table.mons[foe_idx];
        let maxhp = spec.maxhp.max(1);
        let foe_maxhp = foe_spec.maxhp.max(1);
        let num_moves = spec.num_moves as usize;
        let dis = (mon.vol_flags & vol_bit::DISABLE) != 0;
        let dis_slot = ((mon.slot_refs >> 4) & 0x03) as usize;
        let last_mon = cb.sides[side].alive_mask.count_ones() <= 1;

        let miracle = mon.item != 0 && spec.is_miracleberry;
        let has_berry = mon.item != 0 && (spec.is_mintberry || spec.is_miracleberry);
        let st_raw = mon.status_raw();
        let is_frz = st_raw == Status::Frz as u8 && !miracle;
        let is_par = st_raw == Status::Par as u8 && !miracle;
        let perish = (mon.vol_flags & vol_bit::PERISH_SONG) != 0;
        let p_deduct = if is_frz {
            0.0
        } else if is_par {
            0.75
        } else {
            1.0
        };

        let lefties_hp = if mon.item != 0 && spec.is_leftovers {
            (maxhp / 16).max(1)
        } else {
            0
        };
        let leech_drain = if (mon.vol_flags & vol_bit::LEECH_SEED) != 0 {
            (maxhp / 8).max(1)
        } else {
            0
        };
        let leech_gain = if (foe_mon.vol_flags & vol_bit::LEECH_SEED) != 0 {
            (foe_maxhp / 8).max(1)
        } else {
            0
        };
        let cnt_packed = mon.status_counter();
        let is_tox = st_raw == Status::Tox as u8
            || ((cnt_packed & 0x10) != 0
                && (st_raw == Status::Psn as u8 || st_raw == Status::Brn as u8));
        let tox_c = ((cnt_packed & 0x0F) as i16 + 1).max(2);
        let status_dot = if is_tox {
            (maxhp / 16).max(1) * tox_c
        } else if st_raw == Status::Brn as u8 || st_raw == Status::Psn as u8 {
            (maxhp / 8).max(1)
        } else {
            0
        };
        let curse_dot = if (mon.vol_flags & vol_bit::CURSE) != 0 {
            (maxhp / 4).max(1)
        } else {
            0
        };
        let sand_dot = if cb.weather_turns > 0
            && cb.weather_kind == 3
            && (spec.type_mask & table.sandstorm_immune_mask) == 0
        {
            (maxhp / 8).max(1)
        } else {
            0
        };
        let net_eot_hp = lefties_hp + leech_gain - leech_drain - status_dot - curse_dot - sand_dot;
        let net_eot_frac = (net_eot_hp as f64) / (maxhp as f64);
        let has_passive_dot = net_eot_hp < 0;

        let is_ghost = (spec.type_mask & (1u32 << (table.ghost_type & 31))) != 0;
        let foe_sub = (foe_mon.vol_flags & vol_bit::SUBSTITUTE) != 0;
        let sub_up = (mon.vol_flags & vol_bit::SUBSTITUTE) != 0;
        let atk_max = mon.boosts[0] >= 6;

        let mut kerns = [DmgKernel1D::zero(); 4];
        let mut best_kern = DmgKernel1D::zero();
        let mut best_mean = 0.0f32;
        let mut max_dmg_hp = 0i16;
        let mut has_st = false;
        let mut has_hp_dep = false;

        for m_i in 0..num_moves {
            let ms = &table.move_specs[idx][m_i];
            if ms.var_bp_kind == 4 || ms.special_kind == 15 {
                has_hp_dep = true;
            }
            let pp = mon.pp[m_i];
            if pp == 0 || (dis && m_i == dis_slot) {
                continue;
            }
            if ms.special_kind == 10 {
                has_st = true;
            }
            if ms.is_status || (last_mon && ms.is_selfdestruct) {
                continue;
            }
            let k =
                DmgKernel1D::for_move_impl(cb, table, side, slot, foe_side, foe_slot, m_i, true);
            if k.max_dmg > max_dmg_hp {
                max_dmg_hp = k.max_dmg;
            }
            if k.mean_dmg > best_mean {
                best_mean = k.mean_dmg;
                best_kern = k;
            }
            kerns[m_i] = k;
        }

        let mut slot_base_kind = [0u8; 4];
        let mut slot_dmg_frac = [0.0f64; 4];
        let mut inst_heal_amt = 0i16;

        for m_i in 0..num_moves {
            let pp_u8 = mon.pp[m_i];
            if pp_u8 == 0 || (dis && m_i == dis_slot) {
                continue;
            }
            let ms = &table.move_specs[idx][m_i];
            if last_mon && ms.is_selfdestruct {
                continue;
            }
            let k = &kerns[m_i];
            slot_dmg_frac[m_i] = (k.mean_dmg as f64) / (foe_maxhp as f64);

            if let Some(amt) = instant_heal_amount(cb, ms, maxhp) {
                slot_base_kind[m_i] = 3;
                if amt > inst_heal_amt {
                    inst_heal_amt = amt;
                }
            } else if ms.is_rest {
                slot_base_kind[m_i] = 4;
            } else if ms.special_kind == 10 {
                slot_base_kind[m_i] = 5;
            } else if k.max_dmg > 0 {
                slot_base_kind[m_i] = if k.mean_dmg >= 0.85 * best_mean { 1 } else { 2 };
            } else if ms.is_substitute {
                slot_base_kind[m_i] = 8;
            } else if ms.special_kind == 6 {
                slot_base_kind[m_i] = 9;
            } else {
                let harmless_gcurse = ms.special_kind == 5
                    && is_ghost
                    && (foe_sub || (foe_mon.vol_flags & vol_bit::CURSE) != 0);
                if ms.special_kind == 5 && is_ghost && !harmless_gcurse {
                    slot_base_kind[m_i] = 7;
                } else if is_spite_move(table, ms) && !foe_sub {
                    slot_base_kind[m_i] = 10;
                } else {
                    slot_base_kind[m_i] = 6;
                }
            }
        }

        let st_kern = if has_st && !has_hp_dep {
            DmgKernel1D::sleep_talk_grid(cb, table, side, slot, foe_side, foe_slot)
        } else {
            DmgKernel1D::zero()
        };

        let eot_cured = lefties_hp + leech_gain - leech_drain - curse_dot - sand_dot;
        let base_p_act = (if is_frz { 25.0 / 256.0 } else { 1.0f32 })
            * (if is_par { 0.75f32 } else { 1.0f32 });
        let flags = (has_passive_dot as u8)
            | ((has_berry as u8) << 1)
            | (((!perish && !is_frz) as u8) << 2)
            | ((sub_up as u8) << 3)
            | ((atk_max as u8) << 4)
            | (((has_st && has_hp_dep) as u8) << 5)
            | ((miracle as u8) << 6);

        Self {
            slot_base_kind,
            slot_dmg_frac,
            best_kern,
            st_kern,
            max_dmg_hp,
            inst_heal_amt,
            lefties_hp,
            maxhp,
            eot_net: [net_eot_hp, eot_cured],
            base_p_act,
            net_eot_frac,
            p_deduct,
            flags,
        }
    }

    #[inline]
    fn instantiate(&self, mon: &nc2000_engine::compact::CompactMon) -> SidePpProfile {
        let hp = mon.hp();
        let maxhp = self.maxhp;
        let sub_up = (self.flags & 8) != 0;
        let atk_max = (self.flags & 16) != 0;

        let mut slot_kind = [0u8; 4];
        let mut slot_pp = [0.0f32; 4];
        let mut pp_inst_heal = 0.0f64;
        let mut pp_rest = 0.0f64;
        let mut pp_st = 0.0f64;
        let mut pp_brk = 0.0f64;
        let mut pp_sub_atk = 0.0f64;
        let mut pp_cons_aux = 0.0f64;
        let mut pp_self_cost = 0.0f64;
        let mut pp_spite = 0.0f64;
        let mut brk_weighted = 0.0f64;
        let mut sub_weighted = 0.0f64;

        for m_i in 0..4 {
            let bk = self.slot_base_kind[m_i];
            if bk == 0 {
                continue;
            }
            let pp_u8 = mon.pp[m_i];
            if pp_u8 == 0 {
                continue;
            }
            let pp = pp_u8 as f64;
            slot_pp[m_i] = pp_u8 as f32;
            match bk {
                1 => {
                    slot_kind[m_i] = 1;
                    pp_brk += pp;
                    brk_weighted += pp * self.slot_dmg_frac[m_i];
                }
                2 => {
                    slot_kind[m_i] = 2;
                    pp_sub_atk += pp;
                    sub_weighted += pp * self.slot_dmg_frac[m_i];
                }
                3 => {
                    slot_kind[m_i] = 3;
                    pp_inst_heal += pp;
                }
                4 => {
                    slot_kind[m_i] = 4;
                    pp_rest += pp;
                }
                5 => {
                    slot_kind[m_i] = 5;
                    pp_st += pp;
                }
                6 => {
                    slot_kind[m_i] = 6;
                    pp_cons_aux += pp;
                }
                7 => {
                    slot_kind[m_i] = 7;
                    pp_self_cost += pp;
                }
                8 => {
                    if sub_up || hp <= maxhp / 4 {
                        slot_kind[m_i] = 6;
                        pp_cons_aux += pp;
                    } else {
                        slot_kind[m_i] = 7;
                        pp_self_cost += pp;
                    }
                }
                9 => {
                    if atk_max || hp <= maxhp / 2 {
                        slot_kind[m_i] = 6;
                        pp_cons_aux += pp;
                    } else {
                        slot_kind[m_i] = 7;
                        pp_self_cost += pp;
                    }
                }
                10 => {
                    slot_kind[m_i] = 6;
                    pp_cons_aux += pp;
                    pp_spite += pp;
                }
                _ => {}
            }
        }

        let brk_dmg_frac = if pp_brk > 0.0 {
            brk_weighted / pp_brk
        } else {
            0.0
        };
        let sub_dmg_frac = if pp_sub_atk > 0.0 {
            sub_weighted / pp_sub_atk
        } else {
            0.0
        };
        let can_heal = ((self.flags & 4) != 0) && (pp_inst_heal > 0.0 || pp_rest > 0.0);
        let heal_is_rest = pp_inst_heal <= 0.0 && pp_rest > 0.0;

        SidePpProfile {
            slot_kind,
            slot_pp,
            pp_inst_heal,
            pp_rest,
            pp_st,
            pp_brk,
            pp_sub_atk,
            pp_cons_aux,
            pp_self_cost,
            pp_spite,
            brk_dmg_frac,
            sub_dmg_frac,
            max_dmg_hp: self.max_dmg_hp,
            lefties_hp: self.lefties_hp,
            net_eot_frac: self.net_eot_frac,
            has_passive_dot: (self.flags & 1) != 0,
            p_deduct: self.p_deduct,
            has_berry: (self.flags & 2) != 0,
            can_heal,
            heal_is_rest,
            had_brk_initially: pp_brk > 0.0,
            inst_heal_amt: self.inst_heal_amt,
        }
    }
}

impl L1PairFastMeta {
    fn build(
        cb: &CompactBattle,
        table: &MatchTable,
        s0: usize,
        slot0: usize,
        s1: usize,
        slot1: usize,
    ) -> Self {
        let spd0 = cb.mon_action_speed(table, s0, slot0);
        let spd1 = cb.mon_action_speed(table, s1, slot1);
        let spd_ord = if spd0 < spd1 {
            0u8
        } else if spd0 == spd1 {
            1u8
        } else {
            2u8
        };
        Self {
            sides: [
                SideFastMeta::build(cb, table, s0, slot0, s1, slot1),
                SideFastMeta::build(cb, table, s1, slot1, s0, slot0),
            ],
            str_stats: StrugglePairStats::build(cb, table, s0, slot0, s1, slot1),
            spd_ord,
        }
    }
}

impl SidePpProfile {
    #[inline]
    fn build(
        cb: &CompactBattle,
        table: &MatchTable,
        side: usize,
        slot: usize,
        foe_side: usize,
        foe_slot: usize,
    ) -> Self {
        Self::build_with_best_kern(cb, table, side, slot, foe_side, foe_slot).0
    }

    #[inline]
    fn build_with_best_kern(
        cb: &CompactBattle,
        table: &MatchTable,
        side: usize,
        slot: usize,
        foe_side: usize,
        foe_slot: usize,
    ) -> (Self, DmgKernel1D) {
        let idx = side * 3 + slot;
        let foe_idx = foe_side * 3 + foe_slot;
        let mon = &cb.mons[idx];
        let foe_mon = &cb.mons[foe_idx];
        let spec = &table.mons[idx];
        let foe_spec = &table.mons[foe_idx];
        let maxhp = spec.maxhp.max(1);
        let foe_maxhp = foe_spec.maxhp.max(1);
        let num_moves = spec.num_moves as usize;
        let dis = (mon.vol_flags & vol_bit::DISABLE) != 0;
        let dis_slot = ((mon.slot_refs >> 4) & 0x03) as usize;
        let last_mon = cb.sides[side].alive_mask.count_ones() <= 1;

        let miracle = mon.item != 0 && spec.is_miracleberry;
        let has_berry = mon.item != 0 && (spec.is_mintberry || spec.is_miracleberry);
        let st_raw = mon.status_raw();
        let is_frz = st_raw == Status::Frz as u8 && !miracle;
        let is_par = st_raw == Status::Par as u8 && !miracle;
        let perish = (mon.vol_flags & vol_bit::PERISH_SONG) != 0;
        let p_deduct = if is_frz {
            0.0
        } else if is_par {
            0.75
        } else {
            1.0
        };

        let lefties_hp = if mon.item != 0 && spec.is_leftovers {
            (maxhp / 16).max(1)
        } else {
            0
        };
        let leech_drain = if (mon.vol_flags & vol_bit::LEECH_SEED) != 0 {
            (maxhp / 8).max(1)
        } else {
            0
        };
        let leech_gain = if (foe_mon.vol_flags & vol_bit::LEECH_SEED) != 0 {
            (foe_maxhp / 8).max(1)
        } else {
            0
        };
        let cnt_packed = mon.status_counter();
        let is_tox = st_raw == Status::Tox as u8
            || ((cnt_packed & 0x10) != 0
                && (st_raw == Status::Psn as u8 || st_raw == Status::Brn as u8));
        let tox_c = ((cnt_packed & 0x0F) as i16 + 1).max(2);
        let status_dot = if is_tox {
            (maxhp / 16).max(1) * tox_c
        } else if st_raw == Status::Brn as u8 || st_raw == Status::Psn as u8 {
            (maxhp / 8).max(1)
        } else {
            0
        };
        let curse_dot = if (mon.vol_flags & vol_bit::CURSE) != 0 {
            (maxhp / 4).max(1)
        } else {
            0
        };
        let sand_dot = if cb.weather_turns > 0
            && cb.weather_kind == 3
            && (spec.type_mask & table.sandstorm_immune_mask) == 0
        {
            (maxhp / 8).max(1)
        } else {
            0
        };
        let net_eot_hp = lefties_hp + leech_gain - leech_drain - status_dot - curse_dot - sand_dot;
        let net_eot_frac = (net_eot_hp as f64) / (maxhp as f64);
        let has_passive_dot = net_eot_hp < 0;

        let is_ghost = (spec.type_mask & (1u32 << (table.ghost_type & 31))) != 0;
        let foe_sub = (foe_mon.vol_flags & vol_bit::SUBSTITUTE) != 0;

        let mut kerns = [DmgKernel1D::zero(); 4];
        let mut best_kern = DmgKernel1D::zero();
        let mut best_mean = 0.0f32;
        let mut max_dmg_hp = 0i16;
        for m_i in 0..num_moves {
            let pp = mon.pp[m_i];
            if pp == 0 || (dis && m_i == dis_slot) {
                continue;
            }
            let ms = &table.move_specs[idx][m_i];
            if ms.is_status || (last_mon && ms.is_selfdestruct) {
                continue;
            }
            let k = DmgKernel1D::for_move_impl(cb, table, side, slot, foe_side, foe_slot, m_i, true);
            if k.max_dmg > max_dmg_hp {
                max_dmg_hp = k.max_dmg;
            }
            if k.mean_dmg > best_mean {
                best_mean = k.mean_dmg;
                best_kern = k;
            }
            kerns[m_i] = k;
        }

        let mut slot_kind = [0u8; 4];
        let mut slot_pp = [0.0f32; 4];
        let mut pp_inst_heal = 0.0f64;
        let mut pp_rest = 0.0f64;
        let mut pp_st = 0.0f64;
        let mut pp_brk = 0.0f64;
        let mut pp_sub_atk = 0.0f64;
        let mut pp_cons_aux = 0.0f64;
        let mut pp_self_cost = 0.0f64;
        let mut pp_spite = 0.0f64;
        let mut brk_weighted = 0.0f64;
        let mut sub_weighted = 0.0f64;
        let mut inst_heal_amt = 0i16;

        for m_i in 0..num_moves {
            let pp_u8 = mon.pp[m_i];
            if pp_u8 == 0 || (dis && m_i == dis_slot) {
                continue;
            }
            let ms = &table.move_specs[idx][m_i];
            if last_mon && ms.is_selfdestruct {
                continue;
            }
            let pp = pp_u8 as f64;
            slot_pp[m_i] = pp_u8 as f32;
            let k = &kerns[m_i];
            let d_frac = (k.mean_dmg as f64) / (foe_maxhp as f64);

            if let Some(amt) = instant_heal_amount(cb, ms, maxhp) {
                slot_kind[m_i] = 3;
                pp_inst_heal += pp;
                if amt > inst_heal_amt {
                    inst_heal_amt = amt;
                }
            } else if ms.is_rest {
                slot_kind[m_i] = 4;
                pp_rest += pp;
            } else if ms.special_kind == 10 {
                slot_kind[m_i] = 5;
                pp_st += pp;
            } else if k.max_dmg > 0 {
                if k.mean_dmg >= 0.85 * best_mean {
                    slot_kind[m_i] = 1;
                    pp_brk += pp;
                    brk_weighted += pp * d_frac;
                } else {
                    slot_kind[m_i] = 2;
                    pp_sub_atk += pp;
                    sub_weighted += pp * d_frac;
                }
            } else {
                let harmless_sub = ms.is_substitute
                    && ((mon.vol_flags & vol_bit::SUBSTITUTE) != 0 || mon.hp() <= maxhp / 4);
                let harmless_drum =
                    ms.special_kind == 6 && (mon.boosts[0] >= 6 || mon.hp() <= maxhp / 2);
                let harmless_gcurse = ms.special_kind == 5
                    && is_ghost
                    && (foe_sub || (foe_mon.vol_flags & vol_bit::CURSE) != 0);
                let self_hp_cost = (ms.is_substitute && !harmless_sub)
                    || (ms.special_kind == 6 && !harmless_drum)
                    || (ms.special_kind == 5 && is_ghost && !harmless_gcurse);
                if !self_hp_cost {
                    slot_kind[m_i] = 6;
                    pp_cons_aux += pp;
                    if is_spite_move(table, ms) && !foe_sub {
                        pp_spite += pp;
                    }
                } else {
                    slot_kind[m_i] = 7;
                    pp_self_cost += pp;
                }
            }
        }

        let brk_dmg_frac = if pp_brk > 0.0 {
            brk_weighted / pp_brk
        } else {
            0.0
        };
        let sub_dmg_frac = if pp_sub_atk > 0.0 {
            sub_weighted / pp_sub_atk
        } else {
            0.0
        };
        let can_heal = !perish && !is_frz && (pp_inst_heal > 0.0 || pp_rest > 0.0);
        let heal_is_rest = pp_inst_heal <= 0.0 && pp_rest > 0.0;

        (
            Self {
                slot_kind,
                slot_pp,
                pp_inst_heal,
                pp_rest,
                pp_st,
                pp_brk,
                pp_sub_atk,
                pp_cons_aux,
                pp_self_cost,
                pp_spite,
                brk_dmg_frac,
                sub_dmg_frac,
                max_dmg_hp,
                lefties_hp,
                net_eot_frac,
                has_passive_dot,
                p_deduct,
                has_berry,
                can_heal,
                heal_is_rest,
                had_brk_initially: pp_brk > 0.0,
                inst_heal_amt,
            },
            best_kern,
        )
    }

    #[inline]
    fn pp_heal_total(&self) -> f64 {
        self.pp_inst_heal + self.pp_rest
    }

    #[inline]
    fn raw_usable_pp(&self) -> f64 {
        self.pp_brk
            + self.pp_sub_atk
            + self.pp_cons_aux
            + self.pp_st
            + self.pp_inst_heal
            + self.pp_rest
            + 0.5 * self.pp_self_cost
    }

    #[inline]
    fn is_struggling(&self) -> bool {
        (self.pp_brk
            + self.pp_sub_atk
            + self.pp_cons_aux
            + self.pp_st
            + self.pp_inst_heal
            + self.pp_rest
            + self.pp_self_cost)
            <= 1e-6
    }

    #[inline]
    fn active_dmg_frac(&self) -> f64 {
        if self.pp_brk > 1e-6 {
            self.brk_dmg_frac
        } else if self.pp_sub_atk > 1e-6 {
            self.sub_dmg_frac
        } else {
            0.0
        }
    }

    #[inline]
    fn after_root_posture(&self, act: u8, slp_now: usize, is_rech: bool) -> Self {
        let mut out = *self;
        if is_rech || out.p_deduct <= 0.0 {
            return out;
        }
        let cost = out.p_deduct;
        if slp_now > 0 {
            if out.pp_st > 0.0 {
                out.pp_st = (out.pp_st - cost).max(0.0);
            }
            return out;
        }
        match act {
            0 => {
                if out.pp_brk > 0.0 {
                    out.pp_brk = (out.pp_brk - cost).max(0.0);
                } else if out.pp_sub_atk > 0.0 {
                    out.pp_sub_atk = (out.pp_sub_atk - cost).max(0.0);
                } else if out.pp_cons_aux > 0.0 {
                    out.pp_cons_aux = (out.pp_cons_aux - cost).max(0.0);
                } else if out.pp_st > 0.0 {
                    out.pp_st = (out.pp_st - cost).max(0.0);
                } else if out.pp_inst_heal > 0.0 {
                    out.pp_inst_heal = (out.pp_inst_heal - cost).max(0.0);
                } else if out.pp_rest > 0.0 {
                    out.pp_rest = (out.pp_rest - cost).max(0.0);
                    out.has_berry = false;
                } else if out.pp_self_cost > 0.0 {
                    out.pp_self_cost = (out.pp_self_cost - cost).max(0.0);
                }
            }
            1 => {
                if out.pp_inst_heal > 0.0 {
                    out.pp_inst_heal = (out.pp_inst_heal - cost).max(0.0);
                } else if out.pp_rest > 0.0 {
                    out.pp_rest = (out.pp_rest - cost).max(0.0);
                    out.has_berry = false;
                }
            }
            _ => {
                if out.pp_cons_aux > 0.0 {
                    out.pp_cons_aux = (out.pp_cons_aux - cost).max(0.0);
                } else if out.pp_st > 0.0 {
                    out.pp_st = (out.pp_st - cost).max(0.0);
                } else if out.pp_inst_heal > 0.0 {
                    out.pp_inst_heal = (out.pp_inst_heal - cost).max(0.0);
                } else if out.pp_rest > 0.0 {
                    out.pp_rest = (out.pp_rest - cost).max(0.0);
                    out.has_berry = false;
                }
            }
        }
        out.can_heal = out.can_heal && (out.pp_inst_heal > 0.0 || out.pp_rest > 0.0);
        out.heal_is_rest = out.pp_inst_heal <= 0.0 && out.pp_rest > 0.0;
        out
    }

    #[inline]
    fn after_slot_use(&self, m_i: usize, is_rech: bool) -> Self {
        let mut out = *self;
        if is_rech || out.p_deduct <= 0.0 || m_i >= 4 {
            return out;
        }
        let cost = out.p_deduct;
        out.slot_pp[m_i] = (out.slot_pp[m_i] - cost as f32).max(0.0);
        match out.slot_kind[m_i] {
            1 => out.pp_brk = (out.pp_brk - cost).max(0.0),
            2 => out.pp_sub_atk = (out.pp_sub_atk - cost).max(0.0),
            3 => out.pp_inst_heal = (out.pp_inst_heal - cost).max(0.0),
            4 => {
                out.pp_rest = (out.pp_rest - cost).max(0.0);
                out.has_berry = false;
            }
            5 => out.pp_st = (out.pp_st - cost).max(0.0),
            6 => out.pp_cons_aux = (out.pp_cons_aux - cost).max(0.0),
            7 => out.pp_self_cost = (out.pp_self_cost - cost).max(0.0),
            _ => {}
        }
        out.can_heal = out.can_heal && (out.pp_inst_heal > 0.0 || out.pp_rest > 0.0);
        out.heal_is_rest = out.pp_inst_heal <= 0.0 && out.pp_rest > 0.0;
        out
    }
}

/// Exact closed-form Shapley-Snow zero-sum solver on centered payoff matrix `m[0..nr][0..nc]` in `[-0.5, +0.5]`.
/// Satisfies `solve(-M^T) == -solve(M)` to machine precision for all `1 <= nr, nc <= 3`.
#[inline]
fn solve_centered_zerosum_3x3(m: &[[f64; 3]; 3], nr: usize, nc: usize) -> f64 {
    if nr == 1 && nc == 1 {
        return m[0][0];
    }
    let mut maximin = f64::NEG_INFINITY;
    for r in 0..nr {
        let mut rmin = f64::INFINITY;
        for c in 0..nc {
            if m[r][c] < rmin {
                rmin = m[r][c];
            }
        }
        if rmin > maximin {
            maximin = rmin;
        }
    }
    let mut minimax = f64::INFINITY;
    for c in 0..nc {
        let mut cmax = f64::NEG_INFINITY;
        for r in 0..nr {
            if m[r][c] > cmax {
                cmax = m[r][c];
            }
        }
        if cmax < minimax {
            minimax = cmax;
        }
    }
    if (minimax - maximin).abs() <= 1e-11 {
        return 0.5 * (maximin + minimax);
    }

    let solve_2x2_sub = |r0: usize, r1: usize, c0: usize, c1: usize| -> Option<(f64, f64, f64)> {
        let a = m[r0][c0];
        let b = m[r0][c1];
        let c = m[r1][c0];
        let d = m[r1][c1];
        let den = a + d - b - c;
        if den.abs() <= 1e-12 {
            return None;
        }
        let p = (d - c) / den;
        let q = (d - b) / den;
        if p > 0.0 && p < 1.0 && q > 0.0 && q < 1.0 {
            let v = (a * d - b * c) / den;
            Some((v, p, q))
        } else {
            None
        }
    };

    if nr == 2 && nc == 2 {
        if let Some((v, _, _)) = solve_2x2_sub(0, 1, 0, 1) {
            return v.clamp(maximin, minimax);
        }
        return 0.5 * (maximin + minimax);
    }

    // Check all 2x2 subgame supports `(r0 < r1, c0 < c1)` for global Nash feasibility
    let mut best_sub: Option<f64> = None;
    for r0 in 0..nr {
        for r1 in (r0 + 1)..nr {
            for c0 in 0..nc {
                for c1 in (c0 + 1)..nc {
                    if let Some((v, p, q)) = solve_2x2_sub(r0, r1, c0, c1) {
                        let mut ok = true;
                        for r_other in 0..nr {
                            if r_other != r0 && r_other != r1 {
                                let row_pay = q * m[r_other][c0] + (1.0 - q) * m[r_other][c1];
                                if row_pay > v + 1e-10 {
                                    ok = false;
                                }
                            }
                        }
                        for c_other in 0..nc {
                            if c_other != c0 && c_other != c1 {
                                let col_pay = p * m[r0][c_other] + (1.0 - p) * m[r1][c_other];
                                if col_pay < v - 1e-10 {
                                    ok = false;
                                }
                            }
                        }
                        if ok {
                            best_sub = Some(match best_sub {
                                Some(prev) => 0.5 * (prev + v),
                                None => v,
                            });
                        }
                    }
                }
            }
        }
    }
    if let Some(v) = best_sub {
        return v.clamp(maximin, minimax);
    }

    // Full 3x3 interior mixed strategy via exact 3x3 cofactors (`det(M) / sum_{i,j} cof(M)_{i,j}`)
    if nr == 3 && nc == 3 {
        let c00 = m[1][1] * m[2][2] - m[1][2] * m[2][1];
        let c01 = -(m[1][0] * m[2][2] - m[1][2] * m[2][0]);
        let c02 = m[1][0] * m[2][1] - m[1][1] * m[2][0];
        let c10 = -(m[0][1] * m[2][2] - m[0][2] * m[2][1]);
        let c11 = m[0][0] * m[2][2] - m[0][2] * m[2][0];
        let c12 = -(m[0][0] * m[2][1] - m[0][1] * m[2][0]);
        let c20 = m[0][1] * m[1][2] - m[0][2] * m[1][1];
        let c21 = -(m[0][0] * m[1][2] - m[0][2] * m[1][0]);
        let c22 = m[0][0] * m[1][1] - m[0][1] * m[1][0];
        let det = m[0][0] * c00 + m[0][1] * c01 + m[0][2] * c02;
        let cof_sum = c00 + c01 + c02 + c10 + c11 + c12 + c20 + c21 + c22;
        if cof_sum.abs() > 1e-12 {
            return (det / cof_sum).clamp(maximin, minimax);
        }
    }

    0.5 * (maximin + minimax)
}

#[inline]
fn bilinear_b16_u16(plane: &[[u16; B16 + 1]; B16 + 1], h0: f64, h1: f64) -> f64 {
    const INV_U16: f64 = 1.0 / 65535.0;
    let x0 = (h0 * (B16 as f64)).clamp(0.0, B16 as f64);
    let lo0 = (x0.floor() as usize).clamp(1, B16);
    let hi0 = (x0.ceil() as usize).clamp(1, B16);
    let w0 = (x0 - (lo0 as f64)).clamp(0.0, 1.0);

    let x1 = (h1 * (B16 as f64)).clamp(0.0, B16 as f64);
    let lo1 = (x1.floor() as usize).clamp(1, B16);
    let hi1 = (x1.ceil() as usize).clamp(1, B16);
    let w1 = (x1 - (lo1 as f64)).clamp(0.0, 1.0);

    let v00 = (plane[lo0][lo1] as f64) * INV_U16;
    let v01 = (plane[lo0][hi1] as f64) * INV_U16;
    let v10 = (plane[hi0][lo1] as f64) * INV_U16;
    let v11 = (plane[hi0][hi1] as f64) * INV_U16;
    let r0 = v00 + w1 * (v01 - v00);
    let r1 = v10 + w1 * (v11 - v10);
    (r0 + w0 * (r1 - r0)).clamp(0.0, 1.0)
}

#[inline]
fn alpha_fast_ko_gate(prof: &[SidePpProfile; 2], slp: [usize; 2], i: usize, h_foe: f64) -> f64 {
    let f = 1 - i;
    let awake_5 = (5.0 - slp[i] as f64).max(0.0);
    let need_brk = awake_5 * prof[i].p_deduct;
    if prof[i].pp_brk >= need_brk - 1e-6 && (!prof[f].can_heal || prof[f].pp_heal_total() >= 2.0) {
        return 1.0;
    }
    let c_i = prof[i].pp_brk * prof[i].brk_dmg_frac
        + prof[i]
            .pp_sub_atk
            .min((need_brk - prof[i].pp_brk).max(0.0))
            * prof[i].sub_dmg_frac
        + 5.0 * (-prof[f].net_eot_frac).max(0.0);
    let h_need = if prof[f].can_heal && prof[f].pp_heal_total() >= 1.0 {
        h_foe + 0.5 * prof[f].pp_heal_total().min(2.0)
    } else {
        h_foe.max(0.05)
    };
    let suff = (c_i / h_need.max(0.05)).clamp(0.0, 1.0);
    let c_full = need_brk * prof[i].brk_dmg_frac + 5.0 * (-prof[f].net_eot_frac).max(0.0);
    let cov = if c_full > 1e-6 && !prof[f].can_heal {
        (c_i / c_full).clamp(0.0, 1.0)
    } else if need_brk > 1e-6 && prof[i].had_brk_initially {
        let sub_ratio = if prof[i].brk_dmg_frac > 1e-6 && prof[i].sub_dmg_frac >= 0.32 {
            (prof[i].sub_dmg_frac / prof[i].brk_dmg_frac).clamp(0.0, 1.0)
        } else {
            0.0
        };
        ((prof[i].pp_brk + sub_ratio * (need_brk - prof[i].pp_brk).max(0.0)) / need_brk)
            .clamp(0.0, 1.0)
    } else {
        1.0
    };
    if prof[f].can_heal {
        cov.min(1.0).max(suff * suff * 0.5)
    } else {
        cov.max(suff * suff)
    }
}

/// Evaluate the slow-manifold `Press`/`Conserve` $2 \times 2$ posture game + diffused Gen 2
/// `Struggle` + `Leftovers` boundary kernel $u_{\text{slow}} \in [-0.5, +0.5]$.
#[inline]
fn eval_finite_pp_slow(
    cached: &FinitePpBellman,
    str_stats: &StrugglePairStats,
    prof: &[SidePpProfile; 2],
    slp: [usize; 2],
    h: [f64; 2],
    p_wake_damaged: [f64; 2],
    spd_ord: u8,
) -> f64 {
    let lambda_slow = LAMBDA_SLOW;
    let can_conserve = [0usize, 1usize].map(|i| {
        let f = 1 - i;
        let press_strictly_dominates = (cached.kappa[i] as f64) >= 0.05
            && prof[i].pp_brk >= 12.0
            && (!prof[i].can_heal || prof[i].pp_heal_total() >= 12.0)
            && prof[f].pp_spite <= 0.0;
        !press_strictly_dominates
            && (prof[i].pp_cons_aux > 0.0
                || prof[i].pp_st > 0.0
                || prof[i].pp_inst_heal > 0.0
                || prof[i].pp_rest > 0.0)
    });

    let foe_faster_or_tie = [spd_ord <= 1, spd_ord >= 1];

    let eval_cell = |a0: usize, a1: usize| -> f64 {
        let a = [a0, a1];
        let deals_atk = [0usize, 1usize].map(|i| {
            a[i] == 0
                && prof[i].p_deduct > 0.0
                && (prof[i].pp_brk > 1e-6 || prof[i].pp_sub_atk > 1e-6)
        });
        let being_damaged = [0usize, 1usize].map(|i| deals_atk[1 - i] || prof[i].has_passive_dot);

        let mut kap_1 = [0.0f64; 2];
        let mut kap_sub = [0.0f64; 2];
        let mut rate_atk = [0.0f64; 2];
        let mut tau_heal = [1e6f64; 2];
        let mut tau_brk = [1e6f64; 2];
        let mut mu_t = [0.0f64; 2];

        for i in 0..2 {
            let f = 1 - i;
            let p_act = prof[i].p_deduct;

            // 1. Effective break hazard `kap_1[i]` and post-breaker hazard `kap_sub[i]`
            if a[i] == 1 {
                if prof[f].has_passive_dot && !prof[f].can_heal {
                    let r_dot = (-prof[f].net_eot_frac).max(0.0);
                    kap_1[i] = r_dot;
                    kap_sub[i] = r_dot;
                }
            } else {
                let base_k = cached.kappa[i] as f64;
                let sub_scale = if prof[i].pp_sub_atk > 1e-6 && prof[i].brk_dmg_frac > 1e-6 {
                    let ratio = (prof[i].sub_dmg_frac / prof[i].brk_dmg_frac).clamp(0.0, 1.0);
                    if prof[f].can_heal && prof[i].sub_dmg_frac < 0.30 {
                        0.05 * ratio * ratio
                    } else {
                        ratio * ratio
                    }
                } else {
                    0.0
                };
                if prof[i].pp_brk > 1e-6 {
                    kap_1[i] = base_k;
                    kap_sub[i] = base_k * sub_scale;
                } else if prof[i].had_brk_initially {
                    kap_1[i] = base_k * sub_scale;
                    kap_sub[i] = kap_1[i];
                } else {
                    kap_1[i] = base_k;
                    kap_sub[i] = base_k;
                }
                if !prof[f].can_heal {
                    let act_d = prof[i].active_dmg_frac();
                    let drain_f = (act_d * prof[i].p_deduct - prof[f].net_eot_frac).max(0.0);
                    let crit_reach = (2.0 * act_d).max(0.05);
                    if h[f] > crit_reach {
                        let cap_k = (drain_f / (h[f] - 0.5 * crit_reach).max(0.10))
                            .max(0.004 * prof[i].p_deduct);
                        kap_1[i] = kap_1[i].min(cap_k);
                        kap_sub[i] = kap_sub[i].min(cap_k);
                    }
                }
            }

            if p_act <= 1e-6 {
                mu_t[i] = if prof[i].is_struggling() { 0.0 } else { 1e6 };
                continue;
            }

            // 2. Breaker depletion clock `tau_brk[i]`
            if a[i] == 0 {
                let r_a = if being_damaged[i] && prof[i].can_heal {
                    (cached.r_atk_act[i] as f64).max(0.08)
                } else {
                    1.0
                };
                rate_atk[i] = p_act * r_a;
                if prof[i].pp_brk > 1e-6 && prof[i].pp_brk < 64.0 {
                    let slp_wait = if slp[i] > 0 { slp[i] as f64 } else { 0.0 };
                    tau_brk[i] = slp_wait + prof[i].pp_brk / rate_atk[i].max(1e-4);
                }
            }
        }

        for i in 0..2 {
            let f = 1 - i;
            let p_act = prof[i].p_deduct;
            if p_act <= 1e-6 {
                continue;
            }

            // 3. Piecewise energy-conserving heal depletion clock `tau_heal[i]`
            //    bounded by opponent `f`'s actual finite `pp_brk` + `pp_sub_atk` (+ Sleep Talk) reserves
            if being_damaged[i] && prof[i].can_heal && prof[i].pp_heal_total() > 1e-6 {
                let r_eq = (cached.r_heal_act[i] as f64).max(0.0);
                let foe_r_atk = if deals_atk[f] {
                    if being_damaged[f] && prof[f].can_heal {
                        (cached.r_atk_act[f] as f64).max(0.25)
                    } else {
                        1.0
                    }
                } else {
                    0.0
                };
                let heal_cap = if prof[i].heal_is_rest { 0.90 } else { 0.45 };
                let max_rh = if prof[i].heal_is_rest { 0.34 } else { 0.90 };
                let total_h_pp = prof[i].pp_heal_total();

                if !deals_atk[f] {
                    // Passive DOT only (`Toxic` / `Leech Seed` / `Burn` / `Curse`)
                    let inc_dot = (-prof[i].net_eot_frac).max(0.0);
                    let r_h = r_eq.max((inc_dot / heal_cap).clamp(0.0, max_rh));
                    if r_h > 1e-4 {
                        tau_heal[i] = total_h_pp / (p_act * r_h);
                    }
                } else {
                    let st_bonus_brk = if prof[f].pp_st > 0.0 && prof[f].pp_rest > 0.0 {
                        0.33 * prof[f].pp_st
                    } else {
                        0.0
                    };
                    let f_rate = rate_atk[f].max(1e-4);
                    let dt_a = if prof[f].pp_brk > 1e-6 {
                        (prof[f].pp_brk + st_bonus_brk) / f_rate
                    } else {
                        0.0
                    };
                    let inc_a = (prof[f].brk_dmg_frac * prof[f].p_deduct * foe_r_atk
                        - prof[i].net_eot_frac)
                        .max(0.0);
                    let r_h_a = if dt_a > 0.0 && inc_a > 1e-4 {
                        r_eq.max((inc_a / heal_cap).clamp(0.02, max_rh))
                    } else {
                        0.0
                    };
                    let burn_a = p_act * r_h_a * dt_a;
                    if r_h_a > 1e-4 && total_h_pp <= burn_a {
                        tau_heal[i] = total_h_pp / (p_act * r_h_a);
                    } else {
                        let rem_h = (total_h_pp - burn_a).max(0.0);
                        let dt_b = if prof[f].pp_sub_atk > 1e-6 {
                            prof[f].pp_sub_atk / f_rate
                        } else {
                            0.0
                        };
                        let inc_b = (prof[f].sub_dmg_frac * prof[f].p_deduct * foe_r_atk
                            - prof[i].net_eot_frac)
                            .max(0.0);
                        let r_h_b = if dt_b > 0.0 && inc_b > 1e-4 {
                            (inc_b / heal_cap).clamp(0.02, max_rh)
                        } else if prof[i].has_passive_dot && (-prof[i].net_eot_frac) > 1e-4 {
                            ((-prof[i].net_eot_frac) / heal_cap).clamp(0.02, max_rh)
                        } else {
                            0.0
                        };
                        let burn_b = if prof[i].has_passive_dot && (-prof[i].net_eot_frac) > 1e-4 {
                            1e9
                        } else {
                            p_act * r_h_b * dt_b
                        };
                        if r_h_b > 1e-4 && rem_h <= burn_b {
                            tau_heal[i] = dt_a + rem_h / (p_act * r_h_b);
                        }
                    }
                }
            }

            // 4. Total `Struggle` arrival clock `mu_t[i]`
            let init_free_slp = if slp[i] > 0 && (a[i] == 1 || prof[i].pp_st <= 1e-6) {
                slp[i] as f64
            } else {
                0.0
            };
            let base_turns = prof[i].raw_usable_pp() / p_act;
            let eff_rest_pp = (prof[i].pp_rest - (prof[i].has_berry as u8 as f64)).max(0.0);
            let rest_sleep_bonus = if eff_rest_pp > 1e-6 {
                let f_has_chip_moves = (prof[f].pp_brk + prof[f].pp_sub_atk) > 1e-6
                    && prof[f].p_deduct > 1e-6
                    && (foe_faster_or_tie[i] || prof[f].max_dmg_hp > prof[i].lefties_hp);
                let can_chip_now = prof[i].has_passive_dot
                    || (deals_atk[f]
                        && (foe_faster_or_tie[i] || prof[f].max_dmg_hp > prof[i].lefties_hp));
                let adj_st = |raw_slp: f64| -> f64 {
                    if a[i] == 0 && prof[i].pp_st > 0.0 {
                        (raw_slp - prof[i].pp_st).max(0.0)
                    } else {
                        raw_slp
                    }
                };
                if can_chip_now {
                    adj_st(2.0 * eff_rest_pp)
                } else {
                    let init_rest = if prof[i].has_berry {
                        0.0
                    } else {
                        p_wake_damaged[i].clamp(0.0, 1.0)
                    };
                    let r_rem = (eff_rest_pp - init_rest).max(0.0);
                    let chipped_later = if f_has_chip_moves && r_rem > 1e-6 {
                        let pp_non_dmg_f =
                            (prof[f].raw_usable_pp() - prof[f].pp_brk - prof[f].pp_sub_atk).max(0.0);
                        let t_safe = pp_non_dmg_f / prof[f].p_deduct.max(1e-4);
                        let init_turns = (slp[i] as f64) + init_rest * (3.0 / p_act);
                        let t_safe_rem = (t_safe - init_turns).max(0.0);
                        let other_pp_i = (prof[i].raw_usable_pp() - prof[i].pp_rest).max(0.0);
                        let failed_rests = (p_act * t_safe_rem - other_pp_i).clamp(0.0, r_rem);
                        let f_atk_turns =
                            (prof[f].pp_brk + prof[f].pp_sub_atk) / prof[f].p_deduct.max(1e-4);
                        (r_rem - failed_rests).max(0.0).min(f_atk_turns)
                    } else {
                        0.0
                    };
                    adj_st(2.0 * (init_rest + chipped_later))
                }
            } else {
                0.0
            };
            let spite_loss = if prof[f].pp_spite > 0.0 && (a[f] == 1 || prof[f].brk_dmg_frac <= 0.05)
            {
                3.5 * prof[f].pp_spite / p_act
            } else {
                0.0
            };
            mu_t[i] = init_free_slp + (base_turns + rest_sleep_bonus - spite_loss).max(0.0);
            if prof[i].can_heal {
                tau_heal[i] = tau_heal[i].min(mu_t[i]);
            }
        }

        // Diffused `Struggle` terminal payoff `u_iii`
        let dt_0_over_1 = (mu_t[0] - mu_t[1]).max(0.0);
        let dt_1_over_0 = (mu_t[1] - mu_t[0]).max(0.0);
        let rem_atk_rate = |side_i: usize, dt: f64, foe_exhaust_t: f64| -> f64 {
            if dt <= 1e-6 || (prof[side_i].pp_brk + prof[side_i].pp_sub_atk) <= 1e-6 {
                return 0.0;
            }
            let p_i = prof[side_i].p_deduct.max(1e-4);
            let tot_atk_t = (prof[side_i].pp_brk + prof[side_i].pp_sub_atk) / p_i;
            let rem_atk_t = if a[side_i] == 1 {
                tot_atk_t.min(dt)
            } else {
                (tot_atk_t - foe_exhaust_t * (rate_atk[side_i] / p_i)).clamp(0.0, dt)
            };
            (rem_atk_t / dt) * prof[side_i].active_dmg_frac() * prof[side_i].p_deduct
        };
        let extra_0_to_1 = rem_atk_rate(0, dt_0_over_1, mu_t[1]);
        let extra_1_to_0 = rem_atk_rate(1, dt_1_over_0, mu_t[0]);
        let h0_str = if prof[0].can_heal {
            (cached.mean_hp_frac[0] as f64).max(h[0] * 0.8)
        } else {
            h[0]
        };
        let h1_str = if prof[1].can_heal {
            (cached.mean_hp_frac[1] as f64).max(h[1] * 0.8)
        } else {
            h[1]
        };
        let u_iii = struggle_arrival_payoff(
            str_stats,
            mu_t[0],
            mu_t[1],
            h0_str,
            h1_str,
            extra_0_to_1,
            extra_1_to_0,
            prof[0].can_heal && tau_heal[0] > mu_t[1],
            prof[1].can_heal && tau_heal[1] > mu_t[0],
        );

        // Post-heal continuation when side `0` or side `1`'s recovery PP expires at time `t_h`
        let eval_post_heal = |t_h: f64| -> f64 {
            let rem_cap = [0usize, 1usize].map(|i| {
                if a[i] != 0 {
                    return 0.0;
                }
                let f = 1 - i;
                let foe_gain = prof[f].net_eot_frac.max(0.0);
                let used = rate_atk[i] * t_h;
                let r_brk = (prof[i].pp_brk - used).max(0.0);
                let used_sub = (used - prof[i].pp_brk).max(0.0);
                let r_sub = (prof[i].pp_sub_atk - used_sub).max(0.0);
                r_brk * (prof[i].brk_dmg_frac - foe_gain).max(0.0)
                    + r_sub * (prof[i].sub_dmg_frac - foe_gain).max(0.0)
            });

            let h0_ex = tau_heal[0] <= t_h + 1e-5;
            let h1_ex = tau_heal[1] <= t_h + 1e-5;
            if h1_ex && !h0_ex {
                let h1_rem = (cached.mean_hp_frac[1] as f64).clamp(0.15, 0.85);
                let beta = (rem_cap[0] / h1_rem).clamp(0.0, 1.0);
                let drain_on_1 = if a[0] == 0 {
                    let used = rate_atk[0] * t_h;
                    let d = if prof[0].pp_brk > used {
                        prof[0].brk_dmg_frac
                    } else if prof[0].pp_brk + prof[0].pp_sub_atk > used {
                        prof[0].sub_dmg_frac
                    } else {
                        0.0
                    };
                    (d * prof[0].p_deduct - prof[1].net_eot_frac).max(0.0)
                } else {
                    (-prof[1].net_eot_frac).max(0.0)
                };
                let t1_die = if drain_on_1 > 1e-5 {
                    h1_rem / drain_on_1
                } else {
                    1e6
                };
                let t0_die = if prof[0].can_heal && tau_heal[0] > t_h + t1_die {
                    if kap_1[1] > 1e-5 {
                        1.0 / kap_1[1]
                    } else {
                        1e6
                    }
                } else {
                    let drain_on_0 = if a[1] == 0 {
                        (prof[1].active_dmg_frac() * prof[1].p_deduct - prof[0].net_eot_frac)
                            .max(0.0)
                    } else {
                        (-prof[0].net_eot_frac).max(0.0)
                    };
                    let h0_rem = (h[0] - t_h * drain_on_0 * 0.35).clamp(0.10, 1.0);
                    if drain_on_0 > 1e-5 {
                        h0_rem / drain_on_0
                    } else {
                        1e6
                    }
                };
                let u_ko = if t0_die >= 1e5 && t1_die < 1e5 {
                    0.492 * (-LAMBDA_SLOW * t1_die.min(30.0)).exp()
                } else {
                    let diff = (t0_die.min(200.0)) - (t1_die.min(200.0));
                    0.492 * diff / (diff * diff + 2.25).sqrt()
                };
                beta * u_ko + (1.0 - beta) * u_iii
            } else if h0_ex && !h1_ex {
                let h0_rem = (cached.mean_hp_frac[0] as f64).clamp(0.15, 0.85);
                let beta = (rem_cap[1] / h0_rem).clamp(0.0, 1.0);
                let drain_on_0 = if a[1] == 0 {
                    let used = rate_atk[1] * t_h;
                    let d = if prof[1].pp_brk > used {
                        prof[1].brk_dmg_frac
                    } else if prof[1].pp_brk + prof[1].pp_sub_atk > used {
                        prof[1].sub_dmg_frac
                    } else {
                        0.0
                    };
                    (d * prof[1].p_deduct - prof[0].net_eot_frac).max(0.0)
                } else {
                    (-prof[0].net_eot_frac).max(0.0)
                };
                let t0_die = if drain_on_0 > 1e-5 {
                    h0_rem / drain_on_0
                } else {
                    1e6
                };
                let t1_die = if prof[1].can_heal && tau_heal[1] > t_h + t0_die {
                    if kap_1[0] > 1e-5 {
                        1.0 / kap_1[0]
                    } else {
                        1e6
                    }
                } else {
                    let drain_on_1 = if a[0] == 0 {
                        (prof[0].active_dmg_frac() * prof[0].p_deduct - prof[1].net_eot_frac)
                            .max(0.0)
                    } else {
                        (-prof[1].net_eot_frac).max(0.0)
                    };
                    let h1_rem = (h[1] - t_h * drain_on_1 * 0.35).clamp(0.10, 1.0);
                    if drain_on_1 > 1e-5 {
                        h1_rem / drain_on_1
                    } else {
                        1e6
                    }
                };
                let u_ko = if t1_die >= 1e5 && t0_die < 1e5 {
                    -0.492 * (-LAMBDA_SLOW * t0_die.min(30.0)).exp()
                } else {
                    let diff = (t0_die.min(200.0)) - (t1_die.min(200.0));
                    0.492 * diff / (diff * diff + 2.25).sqrt()
                };
                beta * u_ko + (1.0 - beta) * u_iii
            } else {
                // Simultaneous heal expiration: symmetric un-healed race blended with `u_iii`
                let d10 = (prof[1].active_dmg_frac() * prof[1].p_deduct - prof[0].net_eot_frac).max(0.0);
                let d01 = (prof[0].active_dmg_frac() * prof[0].p_deduct - prof[1].net_eot_frac).max(0.0);
                if d10 <= 1e-5 && d01 <= 1e-5 {
                    u_iii
                } else {
                    let t0 = if d10 > 1e-5 {
                        (cached.mean_hp_frac[0] as f64).max(0.2) / d10
                    } else {
                        200.0
                    };
                    let t1 = if d01 > 1e-5 {
                        (cached.mean_hp_frac[1] as f64).max(0.2) / d01
                    } else {
                        200.0
                    };
                    let diff = t0.min(200.0) - t1.min(200.0);
                    let u_ko = 0.492 * diff / (diff * diff + 2.25).sqrt();
                    let beta = ((rem_cap[0] + rem_cap[1]) * 0.5).clamp(0.0, 1.0);
                    beta * u_ko + (1.0 - beta) * u_iii
                }
            }
        };

        let t_str = mu_t[0].min(mu_t[1]);
        let t_heal_min = tau_heal[0].min(tau_heal[1]);
        let t_brk_min = tau_brk[0].min(tau_brk[1]);
        let t1 = t_str.min(t_heal_min).min(t_brk_min).max(0.0);

        let str_horizon_atten = (-0.045 * (t_str - 22.0).max(0.0)).exp();
        let u_iii_eff = u_iii * str_horizon_atten;

        let u_after_t1 = if t1 >= t_str - 1e-5 {
            u_iii_eff
        } else if t_heal_min <= t1 + 1e-5 {
            eval_post_heal(t1)
        } else {
            // Breaker depleted before heal/Struggle: run Epoch 2 `[t1, t2]` with post-breaker hazards
            let k0_2 = if tau_brk[0] <= t1 + 1e-5 {
                kap_sub[0]
            } else {
                kap_1[0]
            };
            let k1_2 = if tau_brk[1] <= t1 + 1e-5 {
                kap_sub[1]
            } else {
                kap_1[1]
            };
            let t2 = t_str.min(t_heal_min).max(t1);
            let dt2 = (t2 - t1).max(0.0);
            let u_after_t2 = if t2 >= t_str - 1e-5 {
                u_iii_eff
            } else {
                eval_post_heal(t2)
            };
            let rate2 = k0_2 + k1_2 + lambda_slow;
            let dkap2 = 0.5 * (k0_2 - k1_2);
            let (phi2, exp2) = phi1_and_exp(rate2, dt2);
            phi2 * dkap2 + exp2 * u_after_t2
        };

        let rate1 = kap_1[0] + kap_1[1] + lambda_slow;
        let dkap1 = 0.5 * (kap_1[0] - kap_1[1]);
        let (phi1, exp1) = phi1_and_exp(rate1, t1);
        (phi1 * dkap1 + exp1 * u_after_t1).clamp(-0.5, 0.5)
    };

    let nr = if can_conserve[0] { 2 } else { 1 };
    let nc = if can_conserve[1] { 2 } else { 1 };
    let mut mat = [[0.0f64; 3]; 3];
    for r in 0..nr {
        for c in 0..nc {
            mat[r][c] = eval_cell(r, c);
        }
    }
    solve_centered_zerosum_3x3(&mat, nr, nc)
}

/// Evaluate expected finite-PP continuation $\mathbb{E}_{(h_0, h_1) \sim P_0 \times P_1}[V(h_0, h_1)]$
/// over normalized surviving Turn-1 HP distributions `pts0` and `pts1` (`h_frac > 0, sum p == 1`).
/// Uses exact 1D-separable bilinear expectation for `(w0_5, w1_5, alpha)` in `O(len0 + len1 + 9)` ops
/// and evaluates `eval_finite_pp_slow` at most ONCE at `[mean_h0, mean_h1]` when `psi_eff > 1e-5`.
#[inline]
fn eval_finite_pp_product_dist(
    cached: &FinitePpBellman,
    get_str_stats: impl FnOnce() -> StrugglePairStats,
    prof: &[SidePpProfile; 2],
    slp: [usize; 2],
    pts0: &[(f64, f64)],
    pts1: &[(f64, f64)],
    spd_ord: u8,
) -> f64 {
    let s0 = slp[0].min(2);
    let s1 = slp[1].min(2);

    let mut mean_h0 = 0.0f64;
    let mut p_dmg0 = 0.0f64;
    let wake_gain0 = if slp[0] == 0 {
        0.0
    } else {
        (slp[0] as f64) * prof[0].net_eot_frac
    };
    for &(h0, p0) in pts0 {
        mean_h0 += h0 * p0;
        if (h0 + wake_gain0).min(1.0) < 1.0 - 1e-4 {
            p_dmg0 += p0;
        }
    }
    let mut mean_h1 = 0.0f64;
    let mut p_dmg1 = 0.0f64;
    let wake_gain1 = if slp[1] == 0 {
        0.0
    } else {
        (slp[1] as f64) * prof[1].net_eot_frac
    };
    for &(h1, p1) in pts1 {
        mean_h1 += h1 * p1;
        if (h1 + wake_gain1).min(1.0) < 1.0 - 1e-4 {
            p_dmg1 += p1;
        }
    }
    mean_h0 = mean_h0.clamp(1e-4, 1.0);
    mean_h1 = mean_h1.clamp(1e-4, 1.0);

    let (u_fast_eff, psi_eff) = if prof[0].is_struggling() || prof[1].is_struggling() {
        (0.0, 1.0)
    } else {
        const INV_U16: f64 = 1.0 / 65535.0;
        let a1_const_one = prof[1].pp_brk >= (5.0 - slp[1] as f64).max(0.0) * prof[1].p_deduct - 1e-6
            && (!prof[0].can_heal || prof[0].pp_heal_total() >= 2.0);
        let a0_const_one = prof[0].pp_brk >= (5.0 - slp[0] as f64).max(0.0) * prof[0].p_deduct - 1e-6
            && (!prof[1].can_heal || prof[1].pp_heal_total() >= 2.0);
        let mut g0 = [0.0f64; B16 + 1];
        let mut g0_a1 = [0.0f64; B16 + 1];
        let mut b0_min = B16 + 1;
        let mut b0_max = 0usize;
        for &(h0, p0) in pts0 {
            if p0 <= 0.0 {
                continue;
            }
            let a1 = if a1_const_one {
                1.0
            } else {
                alpha_fast_ko_gate(prof, slp, 1, h0)
            };
            let x0 = (h0 * (B16 as f64)).clamp(0.0, B16 as f64);
            let lo = (x0.floor() as usize).clamp(1, B16);
            let hi = (x0.ceil() as usize).clamp(1, B16);
            let w = (x0 - (lo as f64)).clamp(0.0, 1.0);
            let w_lo = p0 * (1.0 - w);
            let w_hi = p0 * w;
            g0[lo] += w_lo;
            g0[hi] += w_hi;
            g0_a1[lo] += w_lo * a1;
            g0_a1[hi] += w_hi * a1;
            if lo < b0_min {
                b0_min = lo;
            }
            if hi > b0_max {
                b0_max = hi;
            }
        }

        let mut g1 = [0.0f64; B16 + 1];
        let mut g1_a0 = [0.0f64; B16 + 1];
        let mut b1_min = B16 + 1;
        let mut b1_max = 0usize;
        for &(h1, p1) in pts1 {
            if p1 <= 0.0 {
                continue;
            }
            let a0 = if a0_const_one {
                1.0
            } else {
                alpha_fast_ko_gate(prof, slp, 0, h1)
            };
            let x1 = (h1 * (B16 as f64)).clamp(0.0, B16 as f64);
            let lo = (x1.floor() as usize).clamp(1, B16);
            let hi = (x1.ceil() as usize).clamp(1, B16);
            let w = (x1 - (lo as f64)).clamp(0.0, 1.0);
            let w_lo = p1 * (1.0 - w);
            let w_hi = p1 * w;
            g1[lo] += w_lo;
            g1[hi] += w_hi;
            g1_a0[lo] += w_lo * a0;
            g1_a0[hi] += w_hi * a0;
            if lo < b1_min {
                b1_min = lo;
            }
            if hi > b1_max {
                b1_max = hi;
            }
        }

        let w0_plane = &cached.w0_5[s0][s1];
        let w1_plane = &cached.w1_5[s0][s1];
        let mut w0_g = 0.0f64;
        let mut w1_g = 0.0f64;
        if b0_min <= b0_max && b1_min <= b1_max {
            for b0 in b0_min..=b0_max {
                let c0 = g0[b0];
                let c0_a1 = g0_a1[b0];
                if c0 <= 0.0 && c0_a1 <= 0.0 {
                    continue;
                }
                let r0 = &w0_plane[b0];
                let r1 = &w1_plane[b0];
                let mut sum_w0 = 0.0f64;
                let mut sum_w1 = 0.0f64;
                for b1 in b1_min..=b1_max {
                    sum_w0 += g1_a0[b1] * (r0[b1] as f64);
                    sum_w1 += g1[b1] * (r1[b1] as f64);
                }
                w0_g += c0 * sum_w0 * INV_U16;
                w1_g += c0_a1 * sum_w1 * INV_U16;
            }
        }
        let psi_g = (1.0 - w0_g - w1_g).clamp(0.0, 1.0);
        (0.5 * (w0_g - w1_g), psi_g)
    };

    if psi_eff <= 1e-5 {
        return (0.5 + u_fast_eff).clamp(0.0, 1.0);
    }
    let str_stats = get_str_stats();
    let u_slow = eval_finite_pp_slow(
        cached,
        &str_stats,
        prof,
        slp,
        [mean_h0, mean_h1],
        [p_dmg0, p_dmg1],
        spd_ord,
    );
    (0.5 + u_fast_eff + psi_eff * u_slow).clamp(0.0, 1.0)
}

/// Exact 1-step Turn-1 stage game over `{Attack, Heal, Conserve}` feeding `eval_finite_pp_product_dist`.
#[inline]
fn eval_finite_pp_root(
    cached: &FinitePpBellman,
    meta: &L1PairFastMeta,
    cb: &CompactBattle,
    table: &MatchTable,
    s0: usize,
    slot0: usize,
    s1: usize,
    slot1: usize,
) -> f64 {
    let idx = [s0 * 3 + slot0, s1 * 3 + slot1];
    let hp = [cb.mons[idx[0]].hp(), cb.mons[idx[1]].hp()];
    if hp[0] <= 0 && hp[1] <= 0 {
        return 0.5;
    }
    if hp[0] <= 0 {
        return 0.0;
    }
    if hp[1] <= 0 {
        return 1.0;
    }
    let maxhp = [meta.sides[0].maxhp, meta.sides[1].maxhp];
    let slp = [
        side_sleep_rem(cb, table, s0, slot0),
        side_sleep_rem(cb, table, s1, slot1),
    ];
    let prof = [
        meta.sides[0].instantiate(&cb.mons[idx[0]]),
        meta.sides[1].instantiate(&cb.mons[idx[1]]),
    ];
    let pre_best_k = [meta.sides[0].best_kern, meta.sides[1].best_kern];
    let get_str_stats = || -> StrugglePairStats { meta.str_stats };

    let spd_ord = meta.spd_ord;
    let mut p_act = [1.0f32; 2];
    let mut is_rech = [false; 2];
    let eot_net = [meta.sides[0].eot_net, meta.sides[1].eot_net];
    let mut sub_hp_init = [0i16; 2];

    for i in 0..2 {
        let m = &cb.mons[idx[i]];
        if (m.vol_flags & vol_bit::SUBSTITUTE) != 0 {
            sub_hp_init[i] = m.sub_hp().max(1);
        }
        let miracle = (meta.sides[i].flags & 64) != 0;
        let is_conf = (m.vol_flags & vol_bit::CONFUSION) != 0 && m.timers[0] > 1 && !miracle;
        let rech = (m.slot_refs & 0x80) != 0;
        is_rech[i] = rech;
        p_act[i] = if rech {
            0.0
        } else {
            meta.sides[i].base_p_act * (if is_conf { 0.50f32 } else { 1.0f32 })
        };
    }

    let mut str_recoil_hp = [0i16; 2];
    let k_att = [
        if slp[0] == 0 && p_act[0] > 0.0 {
            if prof[0].is_struggling() {
                let (sk, mr) = DmgKernel1D::struggle(cb, table, s0, slot0, s1, slot1);
                str_recoil_hp[0] = (mr.round() as i16).max(1);
                sk
            } else {
                pre_best_k[0]
            }
        } else {
            DmgKernel1D::zero()
        },
        if slp[1] == 0 && p_act[1] > 0.0 {
            if prof[1].is_struggling() {
                let (sk, mr) = DmgKernel1D::struggle(cb, table, s1, slot1, s0, slot0);
                str_recoil_hp[1] = (mr.round() as i16).max(1);
                sk
            } else {
                pre_best_k[1]
            }
        } else {
            DmgKernel1D::zero()
        },
    ];
    let k_st = [
        if slp[0] > 0 && prof[0].pp_st > 0.0 && p_act[0] > 0.0 {
            if (meta.sides[0].flags & 32) != 0 {
                DmgKernel1D::sleep_talk(cb, table, s0, slot0, s1, slot1)
            } else {
                meta.sides[0].st_kern
            }
        } else {
            DmgKernel1D::zero()
        },
        if slp[1] > 0 && prof[1].pp_st > 0.0 && p_act[1] > 0.0 {
            if (meta.sides[1].flags & 32) != 0 {
                DmgKernel1D::sleep_talk(cb, table, s1, slot1, s0, slot0)
            } else {
                meta.sides[1].st_kern
            }
        } else {
            DmgKernel1D::zero()
        },
    ];

    let opp_hit_buffer = [0, 1].map(|i| {
        let foe_k = &k_att[1 - i];
        (foe_k.mean_dmg.round() as i16).clamp(maxhp[i] / 4, maxhp[i] / 2)
    });

    // `act` encoding: `0 = Attack`, `1 = Heal`, `2 = Conserve (0-dmg pass)`, `3 = Immobilized (no PP spent)`
    let eval_order = |first: usize,
                      a0: u8,
                      a1: u8,
                      next_prof: &[SidePpProfile; 2]|
     -> f64 {
        let second = 1 - first;
        let act = [a0, a1];
        let win_first = if first == 0 { 1.0f64 } else { 0.0f64 };
        let win_second = 1.0f64 - win_first;
        let sub_fir = sub_hp_init[first];
        let sub_sec = sub_hp_init[second];

        let (hp_fir_mid, ns_fir, cured_fir, k_fir_opt, rec_fir) = if slp[first] > 0 {
            (
                hp[first],
                slp[first] - 1,
                prof[first].pp_rest > 0.0,
                if prof[first].pp_st > 0.0 {
                    Some(&k_st[first])
                } else {
                    None
                },
                0i16,
            )
        } else if act[first] == 1 {
            if prof[first].heal_is_rest {
                (
                    maxhp[first],
                    if prof[first].has_berry { 0 } else { 2 },
                    true,
                    None,
                    0i16,
                )
            } else {
                (
                    (hp[first] + prof[first].inst_heal_amt).min(maxhp[first]),
                    0,
                    false,
                    None,
                    0i16,
                )
            }
        } else if act[first] >= 2 {
            (hp[first], 0, false, None, 0i16)
        } else {
            (hp[first], 0, false, Some(&k_att[first]), str_recoil_hp[first])
        };

        let mut sec_pts = [(0i16, 0i16, 0.0f32); 17];
        let mut sec_len = 0usize;
        let mut ko_sec_prob = 0.0f32;
        let push_pt = |pts: &mut [(i16, i16, f32); 17],
                       len: &mut usize,
                       ko: &mut f32,
                       h: i16,
                       sub_h: i16,
                       p: f32| {
            if p <= 0.0 {
                return;
            }
            if h <= 0 {
                *ko += p;
                return;
            }
            for idx_p in 0..*len {
                if pts[idx_p].0 == h && pts[idx_p].1 == sub_h {
                    pts[idx_p].2 += p;
                    return;
                }
            }
            if *len < 17 {
                pts[*len] = (h, sub_h, p);
                *len += 1;
            } else {
                pts[*len - 1].2 += p;
            }
        };

        let pa_fir = if act[first] == 3 { 0.0 } else { p_act[first] };
        let mut hp_fir_after_self = hp_fir_mid;
        if let Some(kern) = k_fir_opt {
            if kern.max_dmg <= 0 || pa_fir <= 0.0 {
                push_pt(&mut sec_pts, &mut sec_len, &mut ko_sec_prob, hp[second], sub_sec, 1.0);
            } else {
                if rec_fir > 0 {
                    hp_fir_after_self =
                        (hp_fir_mid - ((rec_fir as f32 * pa_fir).round() as i16)).max(0);
                }
                if pa_fir < 1.0 {
                    push_pt(
                        &mut sec_pts,
                        &mut sec_len,
                        &mut ko_sec_prob,
                        hp[second],
                        sub_sec,
                        1.0 - pa_fir,
                    );
                }
                for i in 0..kern.len {
                    let (dmg, prob) = kern.pts[i];
                    let (h_after, sub_after) = if sub_sec > 0 && dmg > 0 {
                        (hp[second], (sub_sec - dmg).max(0))
                    } else {
                        ((hp[second] - dmg).max(0), sub_sec)
                    };
                    push_pt(
                        &mut sec_pts,
                        &mut sec_len,
                        &mut ko_sec_prob,
                        h_after,
                        sub_after,
                        pa_fir * prob,
                    );
                }
            }
        } else {
            push_pt(&mut sec_pts, &mut sec_len, &mut ko_sec_prob, hp[second], sub_sec, 1.0);
        }

        // Stadium 2 skips Struggle recoil on a 1v1 KO (`ko_sec_prob` wins cleanly for `first`)
        let mut total_val = (ko_sec_prob as f64) * win_first;
        if sec_len == 0 {
            return total_val.clamp(0.0, 1.0);
        }
        if hp_fir_after_self <= 0 {
            let surv_sec: f64 = sec_pts[..sec_len].iter().map(|x| x.2 as f64).sum();
            return (total_val + surv_sec * win_second).clamp(0.0, 1.0);
        }

        let (sec_mode, ns_sec, cured_sec, k_sec_opt, rec_sec) = if slp[second] > 0 {
            (
                0u8,
                slp[second] - 1,
                prof[second].pp_rest > 0.0,
                if prof[second].pp_st > 0.0 {
                    Some(&k_st[second])
                } else {
                    None
                },
                0i16,
            )
        } else if act[second] == 1 {
            if prof[second].heal_is_rest {
                (1u8, if prof[second].has_berry { 0 } else { 2 }, true, None, 0i16)
            } else {
                (2u8, 0, false, None, 0i16)
            }
        } else if act[second] >= 2 {
            (0u8, 0, false, None, 0i16)
        } else {
            (0u8, 0, false, Some(&k_att[second]), str_recoil_hp[second])
        };

        let mut fir_pts = [(0i16, 0i16, 0.0f32); 17];
        let mut fir_len = 0usize;
        let mut ko_fir_prob = 0.0f32;
        let pa_sec = if act[second] == 3 { 0.0 } else { p_act[second] };
        let sec_rec_applied = if k_sec_opt.is_some_and(|k| k.max_dmg > 0) && pa_sec > 0.0 {
            (rec_sec as f32 * pa_sec).round() as i16
        } else {
            0
        };
        if let Some(kern) = k_sec_opt {
            if kern.max_dmg <= 0 || pa_sec <= 0.0 {
                push_pt(
                    &mut fir_pts,
                    &mut fir_len,
                    &mut ko_fir_prob,
                    hp_fir_after_self,
                    sub_fir,
                    1.0,
                );
            } else {
                if pa_sec < 1.0 {
                    push_pt(
                        &mut fir_pts,
                        &mut fir_len,
                        &mut ko_fir_prob,
                        hp_fir_after_self,
                        sub_fir,
                        1.0 - pa_sec,
                    );
                }
                for i in 0..kern.len {
                    let (dmg, prob) = kern.pts[i];
                    let (h_after, sub_after) = if sub_fir > 0 && dmg > 0 {
                        (hp_fir_after_self, (sub_fir - dmg).max(0))
                    } else {
                        ((hp_fir_after_self - dmg).max(0), sub_fir)
                    };
                    push_pt(
                        &mut fir_pts,
                        &mut fir_len,
                        &mut ko_fir_prob,
                        h_after,
                        sub_after,
                        pa_sec * prob,
                    );
                }
            }
        } else {
            push_pt(
                &mut fir_pts,
                &mut fir_len,
                &mut ko_fir_prob,
                hp_fir_after_self,
                sub_fir,
                1.0,
            );
        }

        let mut fir_alive = [(0.0f64, 0.0f64); 17];
        let mut fir_alive_len = 0usize;
        let mut p_fir_ko_eot = 0.0f64;
        let mut p_fir_alive = 0.0f64;
        let eot_fir_val = eot_net[first][cured_fir as usize];
        for &(hp_fir_post, sub_fir_after, p_fir_f32) in &fir_pts[..fir_len] {
            let p_fir = p_fir_f32 as f64;
            let h_end = (hp_fir_post + eot_fir_val).clamp(0, maxhp[first]);
            if h_end <= 0 {
                p_fir_ko_eot += p_fir;
            } else {
                let h_eff = if sub_fir_after > 0 {
                    let buf = sub_fir_after.max(opp_hit_buffer[first]);
                    (h_end + buf).min(maxhp[first])
                } else {
                    h_end
                };
                let h_eff_frac = (h_eff as f64 / maxhp[first] as f64).clamp(1e-4, 1.0);
                fir_alive[fir_alive_len] = (h_eff_frac, p_fir);
                fir_alive_len += 1;
                p_fir_alive += p_fir;
            }
        }

        let mut sec_alive = [(0.0f64, 0.0f64); 17];
        let mut sec_alive_len = 0usize;
        let mut p_sec_ko_eot = 0.0f64;
        let mut p_sec_alive = 0.0f64;
        let eot_sec_val = eot_net[second][cured_sec as usize];
        for &(hp_sec_mid, sub_sec_after, p_sec_f32) in &sec_pts[..sec_len] {
            let p_sec = p_sec_f32 as f64;
            let hp_sec_end = if sec_mode == 1 {
                (maxhp[second] + eot_sec_val).clamp(0, maxhp[second])
            } else {
                let hp_sec_post = if sec_mode == 2 {
                    (hp_sec_mid + prof[second].inst_heal_amt).min(maxhp[second])
                } else {
                    (hp_sec_mid - sec_rec_applied).max(0)
                };
                if hp_sec_post <= 0 && sec_rec_applied > 0 {
                    0
                } else {
                    (hp_sec_post + eot_sec_val).clamp(0, maxhp[second])
                }
            };
            if hp_sec_end <= 0 {
                p_sec_ko_eot += p_sec;
            } else {
                let h_sec_eff = if sub_sec_after > 0 {
                    let buf = sub_sec_after.max(opp_hit_buffer[second]);
                    (hp_sec_end + buf).min(maxhp[second])
                } else {
                    hp_sec_end
                };
                let h_sec_frac = (h_sec_eff as f64 / maxhp[second] as f64).clamp(1e-4, 1.0);
                sec_alive[sec_alive_len] = (h_sec_frac, p_sec);
                sec_alive_len += 1;
                p_sec_alive += p_sec;
            }
        }

        let ko_fir_direct = ko_fir_prob as f64;
        total_val += (p_sec_ko_eot + p_sec_alive) * ko_fir_direct * win_second;
        if p_sec_ko_eot > 0.0 {
            total_val += p_sec_ko_eot * (p_fir_ko_eot * 0.5 + p_fir_alive * win_first);
        }
        if p_sec_alive > 0.0 {
            total_val += p_sec_alive * p_fir_ko_eot * win_second;
        }
        let p_both_alive = p_sec_alive * p_fir_alive;
        if p_both_alive > 1e-9 && fir_alive_len > 0 && sec_alive_len > 0 {
            let inv_fir = 1.0 / p_fir_alive;
            for item in &mut fir_alive[..fir_alive_len] {
                item.1 *= inv_fir;
            }
            let inv_sec = 1.0 / p_sec_alive;
            for item in &mut sec_alive[..sec_alive_len] {
                item.1 *= inv_sec;
            }
            let next_slp = if first == 0 {
                [ns_fir.min(2), ns_sec.min(2)]
            } else {
                [ns_sec.min(2), ns_fir.min(2)]
            };
            let (pts0, pts1) = if first == 0 {
                (&fir_alive[..fir_alive_len], &sec_alive[..sec_alive_len])
            } else {
                (&sec_alive[..sec_alive_len], &fir_alive[..fir_alive_len])
            };
            let v_cont = eval_finite_pp_product_dist(
                cached,
                get_str_stats,
                next_prof,
                next_slp,
                pts0,
                pts1,
                spd_ord,
            );
            total_val += p_both_alive * (0.5 + GAMMA_SLOW * (v_cont - 0.5));
        }

        total_val.clamp(0.0, 1.0)
    };

    let build_acts = |i: usize| -> ([u8; 3], usize) {
        let mut acts = [0u8; 3];
        let mut n = 1usize;
        if slp[i] == 0 && !is_rech[i] {
            if prof[i].can_heal && hp[i] < maxhp[i] {
                acts[n] = 1;
                n += 1;
            }
            let f = 1 - i;
            let foe_awake_full_rest = hp[f] >= maxhp[f]
                && slp[f] == 0
                && prof[f].pp_rest > 0.0
                && prof[f].pp_inst_heal <= 0.0;
            let want_cons = foe_awake_full_rest
                || prof[i].pp_brk <= 6.0
                || (prof[i].pp_brk + prof[i].pp_sub_atk) <= 10.0
                || prof[f].pp_spite > 0.0;
            let can_cons = prof[i].pp_cons_aux > 0.0
                || prof[i].pp_st > 0.0
                || (hp[i] >= maxhp[i] && (prof[i].pp_inst_heal > 0.0 || prof[i].pp_rest > 0.0));
            if want_cons && can_cons && (prof[i].pp_brk > 0.0 || prof[i].pp_sub_atk > 0.0) {
                acts[n] = 2;
                n += 1;
            }
        }
        (acts, n)
    };
    let (acts0, n0) = build_acts(0);
    let (acts1, n1) = build_acts(1);

    let eval_pair = |a0: u8, a1: u8, np: &[SidePpProfile; 2]| -> f64 {
        if spd_ord == 2 {
            eval_order(0, a0, a1, np)
        } else if spd_ord == 0 {
            eval_order(1, a0, a1, np)
        } else {
            0.5 * (eval_order(0, a0, a1, np) + eval_order(1, a0, a1, np))
        }
    };

    let mut mat = [[0.0f64; 3]; 3];
    for (i0, &a0) in acts0[..n0].iter().enumerate() {
        let np0 = prof[0].after_root_posture(a0, slp[0], is_rech[0]);
        let p0_ok = if a0 == 1 { p_act[0] as f64 } else { 1.0 };
        for (i1, &a1) in acts1[..n1].iter().enumerate() {
            let np1 = prof[1].after_root_posture(a1, slp[1], is_rech[1]);
            let np = [np0, np1];
            let p1_ok = if a1 == 1 { p_act[1] as f64 } else { 1.0 };
            let mut cell = p0_ok * p1_ok * eval_pair(a0, a1, &np);
            if p0_ok < 1.0 {
                let np_f0 = [prof[0], np1];
                cell += (1.0 - p0_ok) * p1_ok * eval_pair(3, a1, &np_f0);
            }
            if p1_ok < 1.0 {
                let np_f1 = [np0, prof[1]];
                cell += p0_ok * (1.0 - p1_ok) * eval_pair(a0, 3, &np_f1);
            }
            if p0_ok < 1.0 && p1_ok < 1.0 {
                cell += (1.0 - p0_ok) * (1.0 - p1_ok) * eval_pair(3, 3, &prof);
            }
            mat[i0][i1] = cell - 0.5;
        }
    }
    (0.5 + solve_centered_zerosum_3x3(&mat, n0, n1)).clamp(0.0, 1.0)
}

/// Evaluate the 1-step Bellman Q-value (from `side`'s perspective, in `[0, 1]`) of choosing
/// specific legal move slot `m_slot < 4` against the opponent's minimax response on Turn 1.
#[inline]
fn eval_one_step_move_q(
    cached: &FinitePpBellman,
    meta: &L1PairFastMeta,
    cb: &CompactBattle,
    table: &MatchTable,
    side: usize,
    m_slot: usize,
) -> f64 {
    let s0_slot = cb.sides[0].active_idx as usize;
    let s1_slot = cb.sides[1].active_idx as usize;
    let idx = [s0_slot, 3 + s1_slot];
    let hp = [cb.mons[idx[0]].hp(), cb.mons[idx[1]].hp()];
    if hp[0] <= 0 || hp[1] <= 0 {
        return 0.5;
    }
    let maxhp = [meta.sides[0].maxhp, meta.sides[1].maxhp];
    let slp = [
        side_sleep_rem(cb, table, 0, s0_slot),
        side_sleep_rem(cb, table, 1, s1_slot),
    ];
    let prof = [
        meta.sides[0].instantiate(&cb.mons[idx[0]]),
        meta.sides[1].instantiate(&cb.mons[idx[1]]),
    ];
    let get_str_stats = || -> StrugglePairStats { meta.str_stats };
    let spd0 = cb.mon_action_speed(table, 0, s0_slot);
    let spd1 = cb.mon_action_speed(table, 1, s1_slot);
    let spd_ord = if spd0 < spd1 {
        0u8
    } else if spd0 == spd1 {
        1u8
    } else {
        2u8
    };

    let me = side;
    let foe = 1 - side;
    let me_slot = if me == 0 { s0_slot } else { s1_slot };
    let foe_slot = if foe == 0 { s0_slot } else { s1_slot };
    let me_idx = idx[me];
    let ms = &table.move_specs[me_idx][m_slot];
    let me_rech = (cb.mons[me_idx].slot_refs & 0x80) != 0;
    let foe_rech = (cb.mons[idx[foe]].slot_refs & 0x80) != 0;

    let me_kern = DmgKernel1D::for_move(cb, table, me, me_slot, foe, foe_slot, m_slot);
    let me_heal_amt = instant_heal_amount(cb, ms, maxhp[me]);
    let me_is_rest = ms.is_rest && hp[me] < maxhp[me];

    let foe_k_att = if slp[foe] == 0 && !foe_rech {
        meta.sides[foe].best_kern
    } else if slp[foe] > 0 && prof[foe].pp_st > 0.0 {
        if (meta.sides[foe].flags & 32) != 0 {
            DmgKernel1D::sleep_talk(cb, table, foe, foe_slot, me, me_slot)
        } else {
            meta.sides[foe].st_kern
        }
    } else {
        DmgKernel1D::zero()
    };

    let np_me = prof[me].after_slot_use(m_slot, me_rech);
    let foe_acts: ([u8; 3], usize) = if slp[foe] > 0 || foe_rech {
        ([0, 0, 0], 1)
    } else {
        let mut a = [0u8; 3];
        let mut n = 1usize;
        if prof[foe].can_heal && hp[foe] < maxhp[foe] {
            a[n] = 1;
            n += 1;
        }
        let can_c = prof[foe].pp_cons_aux > 0.0
            || prof[foe].pp_st > 0.0
            || (hp[foe] >= maxhp[foe] && (prof[foe].pp_inst_heal > 0.0 || prof[foe].pp_rest > 0.0));
        if can_c && (prof[foe].pp_brk > 0.0 || prof[foe].pp_sub_atk > 0.0) {
            a[n] = 2;
            n += 1;
        }
        (a, n)
    };

    let mut worst_for_me = 1.0f64;
    for &fa in &foe_acts.0[..foe_acts.1] {
        let np_foe = prof[foe].after_root_posture(fa, slp[foe], foe_rech);
        let next_prof = if me == 0 {
            [np_me, np_foe]
        } else {
            [np_foe, np_me]
        };

        let ns_me = if slp[me] > 0 {
            slp[me] - 1
        } else if me_is_rest {
            if prof[me].has_berry { 0 } else { 2 }
        } else {
            0
        };
        let ns_foe = if slp[foe] > 0 {
            slp[foe] - 1
        } else if fa == 1 && prof[foe].heal_is_rest {
            if prof[foe].has_berry { 0 } else { 2 }
        } else {
            0
        };
        let next_slp = if me == 0 {
            [ns_me.min(2), ns_foe.min(2)]
        } else {
            [ns_foe.min(2), ns_me.min(2)]
        };

        let hp_me_base = if me_is_rest {
            maxhp[me] as f64
        } else if let Some(amt) = me_heal_amt {
            ((hp[me] + amt).min(maxhp[me])) as f64
        } else {
            hp[me] as f64
        };
        let foe_dmg_to_me = if slp[foe] > 0 {
            if prof[foe].pp_st > 0.0 {
                foe_k_att.mean_dmg as f64 * prof[foe].p_deduct
            } else {
                0.0
            }
        } else if fa == 0 {
            foe_k_att.mean_dmg as f64 * prof[foe].p_deduct
        } else {
            0.0
        };
        let h_me_end = ((hp_me_base - foe_dmg_to_me) / (maxhp[me] as f64) + prof[me].net_eot_frac)
            .clamp(0.0, 1.0);

        let mut val_me = 0.0f64;
        if slp[me] == 0 && !me_rech && me_kern.max_dmg > 0 {
            let pa = prof[me].p_deduct;
            let mut foe_surv = [(0.0f64, 0.0f64); 17];
            let mut foe_surv_len = 0usize;
            let mut p_foe_surv = 0.0f64;
            if pa < 1.0 {
                let h_foe_miss = if fa == 1 {
                    if prof[foe].heal_is_rest {
                        1.0
                    } else {
                        ((hp[foe] + prof[foe].inst_heal_amt).min(maxhp[foe]) as f64)
                            / (maxhp[foe] as f64)
                    }
                } else {
                    (hp[foe] as f64) / (maxhp[foe] as f64)
                };
                let hf = (h_foe_miss + prof[foe].net_eot_frac).clamp(1e-4, 1.0);
                foe_surv[foe_surv_len] = (hf, 1.0 - pa);
                foe_surv_len += 1;
                p_foe_surv += 1.0 - pa;
            }
            for k_i in 0..me_kern.len {
                let (dmg, prob) = me_kern.pts[k_i];
                let w = pa * (prob as f64);
                let hp_foe_hit = hp[foe] - dmg;
                if hp_foe_hit <= 0
                    && (fa != 1
                        || spd0 == spd1
                        || (me == 0 && spd0 > spd1)
                        || (me == 1 && spd1 > spd0))
                {
                    val_me += w * 1.0;
                } else {
                    let hp_foe_post = if fa == 1 {
                        if prof[foe].heal_is_rest {
                            maxhp[foe]
                        } else {
                            (hp_foe_hit.max(1) + prof[foe].inst_heal_amt).min(maxhp[foe])
                        }
                    } else {
                        hp_foe_hit.max(0)
                    };
                    if hp_foe_post <= 0 {
                        val_me += w * 1.0;
                    } else {
                        let hf = ((hp_foe_post as f64) / (maxhp[foe] as f64)
                            + prof[foe].net_eot_frac)
                            .clamp(1e-4, 1.0);
                        foe_surv[foe_surv_len] = (hf, w);
                        foe_surv_len += 1;
                        p_foe_surv += w;
                    }
                }
            }
            if p_foe_surv > 1e-9 && foe_surv_len > 0 {
                if h_me_end <= 1e-7 {
                    val_me += p_foe_surv * (0.5 + GAMMA_SLOW * (0.0 - 0.5));
                } else {
                    let inv_s = 1.0 / p_foe_surv;
                    for item in &mut foe_surv[..foe_surv_len] {
                        item.1 *= inv_s;
                    }
                    let me_pt = [(h_me_end, 1.0f64)];
                    let (pts0, pts1) = if me == 0 {
                        (&me_pt[..], &foe_surv[..foe_surv_len])
                    } else {
                        (&foe_surv[..foe_surv_len], &me_pt[..])
                    };
                    let v0 = eval_finite_pp_product_dist(
                        cached,
                        get_str_stats,
                        &next_prof,
                        next_slp,
                        pts0,
                        pts1,
                        spd_ord,
                    );
                    let vm = if me == 0 { v0 } else { 1.0 - v0 };
                    val_me += p_foe_surv * (0.5 + GAMMA_SLOW * (vm - 0.5));
                }
            }
        } else {
            let h_foe_base = if fa == 1 {
                if prof[foe].heal_is_rest {
                    1.0
                } else {
                    ((hp[foe] + prof[foe].inst_heal_amt).min(maxhp[foe]) as f64)
                        / (maxhp[foe] as f64)
                }
            } else {
                (hp[foe] as f64) / (maxhp[foe] as f64)
            };
            let hf = (h_foe_base + prof[foe].net_eot_frac).clamp(1e-4, 1.0);
            if h_me_end <= 1e-7 {
                val_me = 0.5 + GAMMA_SLOW * (0.0 - 0.5);
            } else {
                let me_pt = [(h_me_end, 1.0f64)];
                let foe_pt = [(hf, 1.0f64)];
                let (pts0, pts1) = if me == 0 {
                    (&me_pt[..], &foe_pt[..])
                } else {
                    (&foe_pt[..], &me_pt[..])
                };
                let v0 = eval_finite_pp_product_dist(
                    cached,
                    get_str_stats,
                    &next_prof,
                    next_slp,
                    pts0,
                    pts1,
                    spd_ord,
                );
                let vm = if me == 0 { v0 } else { 1.0 - v0 };
                val_me = 0.5 + GAMMA_SLOW * (vm - 0.5);
            }
        }

        if val_me < worst_for_me {
            worst_for_me = val_me;
        }
    }
    worst_for_me.clamp(0.0, 1.0)
}

/// Evaluate the 1v1 race & endurance differential in `[-1.0, 1.0]` between `(s0, slot0)` and `(s1, slot1)`.
#[inline]
pub fn toeplitz_coupled_race(
    cb: &CompactBattle,
    table: &MatchTable,
    s0: usize,
    slot0: usize,
    s1: usize,
    slot1: usize,
) -> f64 {
    toeplitz_coupled_race_ext(cb, table, s0, slot0, s1, slot1, true)
}

#[inline]
fn side_canon_key(
    cb: &CompactBattle,
    table: &MatchTable,
    side: usize,
    slot: usize,
) -> (u64, u64) {
    let idx = side * 3 + slot;
    let m = &cb.mons[idx];
    let sp = &table.mons[idx];
    let sd = &cb.sides[side];
    let mut h = Hasher128::new();
    h.write_u64(sp.static_fp);
    h.write_u64(
        (m.hp_pool[0] as u16 as u64)
            | ((m.hp_pool[1] as u16 as u64) << 16)
            | ((m.item as u64) << 32)
            | ((m.status_packed as u64) << 40)
            | ((m.vol_flags as u64) << 48),
    );
    h.write_u64(
        u32::from_le_bytes(m.pp) as u64
            | ((u32::from_le_bytes([m.timers[0], m.timers[1], m.timers[2], m.slot_refs]) as u64)
                << 32),
    );
    let mut b_pack = 0u64;
    for (i, &b) in m.boosts.iter().enumerate() {
        b_pack |= (((b + 6) as u8 as u64) & 0xF) << (i * 4);
    }
    b_pack |= ((sd.reflect_turns as u64) << 32)
        | ((sd.lightscreen_turns as u64) << 40)
        | ((sd.safeguard_turns as u64) << 48)
        | (((sd.side_flags & 0x03) as u64) << 56)
        | (((cb.sides[side].alive_mask.count_ones() <= 1) as u64) << 60);
    h.write_u64(b_pack);
    h.write_u64(
        (cb.hurt_damage_this_turn[side] as u64) | ((cb.hurt_cat_this_turn[side] as u64) << 16),
    );
    let k = h.finish();
    (k.0, k.1)
}

#[inline]
fn side_pp_scarcity_lambda(
    cb: &CompactBattle,
    table: &MatchTable,
    side: usize,
    slot: usize,
    foe_side: usize,
    foe_slot: usize,
) -> f64 {
    let idx = side * 3 + slot;
    let mon = &cb.mons[idx];
    let spec = &table.mons[idx];
    let num_moves = spec.num_moves as usize;
    let dis = (mon.vol_flags & vol_bit::DISABLE) != 0;
    let dis_slot = ((mon.slot_refs >> 4) & 0x03) as usize;
    let last_mon = cb.sides[side].alive_mask.count_ones() <= 1;

    let mut tot_pp = 0.0f64;
    let mut has_heal_slot = false;
    let mut heal_equiv = 0.0f64;
    let mut slot_eff = [0.0f64; 4];
    let mut slot_atk_pp = [0.0f64; 4];
    let mut best_eff = 0.0f64;
    let mut has_spite = false;

    for m_i in 0..num_moves {
        let ms = &table.move_specs[idx][m_i];
        if last_mon && ms.is_selfdestruct {
            continue;
        }
        let blocked = dis && m_i == dis_slot;
        let pp = if blocked { 0.0 } else { mon.pp[m_i] as f64 };
        tot_pp += pp;

        if ms.is_half_heal || ms.special_kind == 29 {
            has_heal_slot = true;
            heal_equiv += pp;
        } else if ms.is_rest {
            has_heal_slot = true;
            heal_equiv += 2.2 * pp;
        } else if !ms.is_status
            && (ms.base_power > 0
                || ms.fixed_damage != 0
                || ms.special_kind == 15
                || ms.var_bp_kind != 0)
        {
            if !blocked {
                let eff =
                    cb.expected_hit_fraction(table, side, slot, foe_side, foe_slot, m_i, true);
                slot_eff[m_i] = eff;
                slot_atk_pp[m_i] = pp;
                if eff > best_eff {
                    best_eff = eff;
                }
            }
        } else if pp > 0.0 && is_spite_move(table, ms) {
            has_spite = true;
        }
    }

    if has_spite {
        return 1.0;
    }
    let mut eff_brk_pp = 0.0f64;
    if best_eff > 0.0 {
        for m_i in 0..num_moves {
            if slot_eff[m_i] >= 0.55 * best_eff {
                eff_brk_pp += slot_atk_pp[m_i];
            }
        }
    }
    let lam_tot = ((22.0 - tot_pp) / 6.0).clamp(0.0, 1.0);
    let lam_heal = if has_heal_slot {
        ((10.0 - heal_equiv) / 2.0).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let lam_brk = if best_eff > 0.0 {
        (5.0 - eff_brk_pp).clamp(0.0, 1.0)
    } else {
        0.0
    };
    lam_tot.max(lam_heal).max(lam_brk)
}

#[inline]
pub fn is_1v1_pp_scarce(cb: &CompactBattle, table: &MatchTable) -> bool {
    let s0 = cb.sides[0].active_idx as usize;
    let s1 = cb.sides[1].active_idx as usize;
    if (cb.sides[0].alive_mask & (1 << s0)) == 0 || (cb.sides[1].alive_mask & (1 << s1)) == 0 {
        return false;
    }
    side_pp_scarcity_lambda(cb, table, 0, s0, 1, s1) > 0.0
        || side_pp_scarcity_lambda(cb, table, 1, s1, 0, s0) > 0.0
}

#[inline]
pub fn toeplitz_coupled_race_ext(
    cb: &CompactBattle,
    table: &MatchTable,
    s0: usize,
    slot0: usize,
    s1: usize,
    slot1: usize,
    stall_pp: bool,
) -> f64 {
    let idx0 = s0 * 3 + slot0;
    let idx1 = s1 * 3 + slot1;
    let hp0 = cb.mons[idx0].hp();
    let hp1 = cb.mons[idx1].hp();
    if hp0 <= 0 || hp1 <= 0 {
        return 0.0;
    }
    if !stall_pp {
        return toeplitz_coupled_race_raw(cb, table, s0, slot0, s1, slot1, false);
    }
    let eff_stall = side_pp_scarcity_lambda(cb, table, s0, slot0, s1, slot1) > 0.0
        || side_pp_scarcity_lambda(cb, table, s1, slot1, s0, slot0) > 0.0;
    // Canonical pair ordering guarantees exact 0-ULP antisymmetry (`race(A,B) == -race(B,A)`),
    // 0-ULP bit identity between `stall` and `-stall` when `lam == 0`, and 2x BellmanCache hits.
    let fp_ord = table.mons[idx0].static_fp.cmp(&table.mons[idx1].static_fp);
    let ord = if fp_ord != std::cmp::Ordering::Equal {
        fp_ord
    } else {
        side_canon_key(cb, table, s0, slot0).cmp(&side_canon_key(cb, table, s1, slot1))
    };
    match ord {
        std::cmp::Ordering::Less => {
            toeplitz_coupled_race_raw(cb, table, s0, slot0, s1, slot1, eff_stall)
        }
        std::cmp::Ordering::Greater => {
            -toeplitz_coupled_race_raw(cb, table, s1, slot1, s0, slot0, eff_stall)
        }
        std::cmp::Ordering::Equal => {
            let r01 = toeplitz_coupled_race_raw(cb, table, s0, slot0, s1, slot1, eff_stall);
            let r10 = toeplitz_coupled_race_raw(cb, table, s1, slot1, s0, slot0, eff_stall);
            0.5 * (r01 - r10)
        }
    }
}

#[inline]
fn side_all_usable_pp_at_least_6(
    cb: &CompactBattle,
    table: &MatchTable,
    side: usize,
    slot: usize,
) -> bool {
    let idx = side * 3 + slot;
    let mon = &cb.mons[idx];
    let num_moves = table.mons[idx].num_moves as usize;
    let dis = (mon.vol_flags & vol_bit::DISABLE) != 0;
    let dis_slot = ((mon.slot_refs >> 4) & 0x03) as usize;
    let last_mon = cb.sides[side].alive_mask.count_ones() <= 1;
    let mut has_any = false;
    for m_i in 0..num_moves {
        let pp = mon.pp[m_i];
        if pp == 0 || (dis && m_i == dis_slot) {
            continue;
        }
        let ms = &table.move_specs[idx][m_i];
        if last_mon && ms.is_selfdestruct {
            continue;
        }
        if pp < 6 {
            return false;
        }
        has_any = true;
    }
    has_any
}

#[inline]
fn toeplitz_coupled_race_raw(
    cb: &CompactBattle,
    table: &MatchTable,
    s0: usize,
    slot0: usize,
    s1: usize,
    slot1: usize,
    stall_pp: bool,
) -> f64 {
    let idx0 = s0 * 3 + slot0;
    let idx1 = s1 * 3 + slot1;
    let hp0 = cb.mons[idx0].hp();
    let hp1 = cb.mons[idx1].hp();
    if hp0 <= 0 || hp1 <= 0 {
        return 0.0;
    }

    if side_has_usable_heal_ext(cb, table, s0, slot0, stall_pp)
        || side_has_usable_heal_ext(cb, table, s1, slot1, stall_pp)
    {
        let v = if stall_pp {
            let lam = side_pp_scarcity_lambda(cb, table, s0, slot0, s1, slot1)
                .max(side_pp_scarcity_lambda(cb, table, s1, slot1, s0, slot0));
            if lam <= 0.0 {
                with_solved_bellman(cb, table, s0, slot0, s1, slot1, |cached| {
                    eval_exact_root_table(&cached.v_table, cb, table, s0, slot0, s1, slot1)
                })
            } else if lam >= 1.0 {
                with_solved_bellman_finite(cb, table, s0, slot0, s1, slot1, |cached, meta| {
                    eval_finite_pp_root(cached, meta, cb, table, s0, slot0, s1, slot1)
                })
            } else {
                let v_inf = with_solved_bellman(cb, table, s0, slot0, s1, slot1, |cached| {
                    eval_exact_root_table(&cached.v_table, cb, table, s0, slot0, s1, slot1)
                });
                let v_fin =
                    with_solved_bellman_finite(cb, table, s0, slot0, s1, slot1, |cached, meta| {
                        eval_finite_pp_root(cached, meta, cb, table, s0, slot0, s1, slot1)
                    });
                (1.0 - lam) * v_inf + lam * v_fin
            }
        } else {
            with_solved_bellman(cb, table, s0, slot0, s1, slot1, |cached| {
                eval_exact_root_table(&cached.v_table, cb, table, s0, slot0, s1, slot1)
            })
        };
        return (2.0 * v - 1.0).clamp(-1.0, 1.0);
    }

    let spd0 = cb.mon_action_speed(table, s0, slot0);
    let spd1 = cb.mon_action_speed(table, s1, slot1);
    let spd_sign = if spd0 > spd1 {
        1.0
    } else if spd1 > spd0 {
        -1.0
    } else {
        0.0
    };

    if !stall_pp {
        let (s_curve_0, rem0) = survival_curve_6_with_mean_hp(cb, table, s1, slot1, s0, slot0, hp0);
        let (s_curve_1, rem1) = survival_curve_6_with_mean_hp(cb, table, s0, slot0, s1, slot1, hp1);

        let mut margin = 0.0f64;
        for k in 1..=6usize {
            let f0 = (s_curve_0[k - 1] - s_curve_0[k]).max(0.0);
            let f1 = (s_curve_1[k - 1] - s_curve_1[k]).max(0.0);
            margin += f1 * s_curve_0[k] - f0 * s_curve_1[k] + f0 * f1 * spd_sign;
        }
        margin += s_curve_0[6] * s_curve_1[6] * (rem0 - rem1);
        return margin.clamp(-1.0, 1.0);
    }

    // Fast 6-step exact race check when neither side can exhaust a usable move in <= 6 turns
    if side_all_usable_pp_at_least_6(cb, table, s0, slot0)
        && side_all_usable_pp_at_least_6(cb, table, s1, slot1)
    {
        let (s_curve_0, rem0) = survival_curve_6_with_mean_hp(cb, table, s1, slot1, s0, slot0, hp0);
        let (s_curve_1, rem1) = survival_curve_6_with_mean_hp(cb, table, s0, slot0, s1, slot1, hp1);
        if s_curve_0[6] * s_curve_1[6] < 1e-5 {
            let mut margin = 0.0f64;
            for k in 1..=6usize {
                let f0 = (s_curve_0[k - 1] - s_curve_0[k]).max(0.0);
                let f1 = (s_curve_1[k - 1] - s_curve_1[k]).max(0.0);
                margin += f1 * s_curve_0[k] - f0 * s_curve_1[k] + f0 * f1 * spd_sign;
            }
            margin += s_curve_0[6] * s_curve_1[6] * (rem0 - rem1);
            return margin.clamp(-1.0, 1.0);
        }
    }

    // Piecewise-constant finite-PP + Gen 2 Struggle stopping-time race (`T_max = 32`)
    let sched0 = SideRaceSchedule::build(cb, table, s0, slot0, s1, slot1);
    let sched1 = SideRaceSchedule::build(cb, table, s1, slot1, s0, slot0);
    let (s_curve_0, steps0, rem0_raw) =
        survival_curve_32_piecewise(cb, table, s1, slot1, s0, slot0, &sched1, &sched0);
    let (s_curve_1, steps1, rem1_raw) =
        survival_curve_32_piecewise(cb, table, s0, slot0, s1, slot1, &sched0, &sched1);
    let max_k = steps0.max(steps1).min(32);

    let mut margin = 0.0f64;
    for k in 1..=max_k {
        let f0 = (s_curve_0[k - 1] - s_curve_0[k]).max(0.0);
        let f1 = (s_curve_1[k - 1] - s_curve_1[k]).max(0.0);
        margin += f1 * s_curve_0[k] - f0 * s_curve_1[k] + f0 * f1 * spd_sign;
    }

    let surv_both = s_curve_0[32] * s_curve_1[32];
    if surv_both > 1e-8 {
        let str_stats = StrugglePairStats::build(cb, table, s0, slot0, s1, slot1);
        let maxhp0 = table.mons[idx0].maxhp.max(1) as f64;
        let maxhp1 = table.mons[idx1].maxhp.max(1) as f64;
        let h0_init = (hp0 as f64 / maxhp0).clamp(0.0, 1.0);
        let h1_init = (hp1 as f64 / maxhp1).clamp(0.0, 1.0);
        let str_steps0 = (32.0 - sched0.turns_before_struggle.max(sched1.turns_before_struggle)).max(0.0);
        let rem0 = if str_stats.delta[0] > 0.0 && str_steps0 > 0.0 {
            rem0_raw.min((h0_init - str_steps0 * str_stats.delta[0]).clamp(0.02, 1.0))
        } else {
            rem0_raw
        };
        let rem1 = if str_stats.delta[1] > 0.0 && str_steps0 > 0.0 {
            rem1_raw.min((h1_init - str_steps0 * str_stats.delta[1]).clamp(0.02, 1.0))
        } else {
            rem1_raw
        };
        let rem_mu0 = (sched0.turns_before_struggle - 32.0).max(0.0);
        let rem_mu1 = (sched1.turns_before_struggle - 32.0).max(0.0);
        let u_tail = struggle_arrival_payoff(
            &str_stats, rem_mu0, rem_mu1, rem0, rem1, 0.0, 0.0, false, false,
        );
        margin += surv_both * (2.0 * u_tail);
    }
    margin.clamp(-1.0, 1.0)
}

/// Perish Song / Mean Look countdown survival probability differential in `[-1.0, 1.0]`
/// computed directly from the Toeplitz survival curve $S(k) = (\mathbf{1}^\top T^k \mathbf{e}_{h_0})$.
#[inline]
pub fn compact_perish_margin(cb: &CompactBattle, table: &MatchTable) -> f64 {
    let slot0 = cb.sides[0].active_idx as usize;
    let slot1 = cb.sides[1].active_idx as usize;
    if (cb.sides[0].alive_mask & (1 << slot0)) == 0 || (cb.sides[1].alive_mask & (1 << slot1)) == 0
    {
        return 0.0;
    }
    let idx0 = slot0;
    let idx1 = 3 + slot1;
    let hp0 = cb.mons[idx0].hp();
    let hp1 = cb.mons[idx1].hp();
    if hp0 <= 0 || hp1 <= 0 {
        return 0.0;
    }

    let p0_perish = if (cb.mons[idx0].vol_flags & vol_bit::PERISH_SONG) != 0 {
        cb.mons[idx0].perish_turns()
    } else {
        0
    };
    let p1_perish = if (cb.mons[idx1].vol_flags & vol_bit::PERISH_SONG) != 0 {
        cb.mons[idx1].perish_turns()
    } else {
        0
    };
    let p0_trapped = (cb.mons[idx0].vol_flags & vol_bit::TRAPPED_OR_LOCKED) != 0
        || cb.sides[0].alive_mask.count_ones() <= 1;
    let p1_trapped = (cb.mons[idx1].vol_flags & vol_bit::TRAPPED_OR_LOCKED) != 0
        || cb.sides[1].alive_mask.count_ones() <= 1;

    if p0_perish == 0 && p1_perish == 0 && p0_trapped == p1_trapped {
        return 0.0;
    }

    let mut need_s0 = 0usize;
    let mut need_s1 = 0usize;
    let mut any_term = false;

    let mut b1_info: Option<(usize, bool)> = None;
    if p1_perish > 0 && p1_trapped && !p0_trapped {
        let hits = p1_perish.saturating_sub(1).min(3) as usize;
        let p0_prot = cb.mons[idx0].stall_counter() == 0
            && (0..table.mons[idx0].num_moves as usize)
                .any(|m| cb.mons[idx0].pp[m] > 0 && table.move_specs[idx0][m].is_protect);
        let p1_phaze = (0..table.mons[idx1].num_moves as usize)
            .any(|m| cb.mons[idx1].pp[m] > 0 && table.move_specs[idx1][m].special_kind == 7);
        let eff_hits = if p0_prot && hits > 0 { hits - 1 } else { hits };
        let use_phaze = p1_phaze && p1_perish >= 2;
        need_s0 = need_s0.max(eff_hits);
        if use_phaze {
            need_s1 = need_s1.max(1);
        }
        b1_info = Some((eff_hits, use_phaze));
        any_term = true;
    }

    let mut b2_info: Option<(usize, bool)> = None;
    if p0_perish > 0 && p0_trapped && !p1_trapped {
        let hits = p0_perish.saturating_sub(1).min(3) as usize;
        let p1_prot = cb.mons[idx1].stall_counter() == 0
            && (0..table.mons[idx1].num_moves as usize)
                .any(|m| cb.mons[idx1].pp[m] > 0 && table.move_specs[idx1][m].is_protect);
        let p0_phaze = (0..table.mons[idx0].num_moves as usize)
            .any(|m| cb.mons[idx0].pp[m] > 0 && table.move_specs[idx0][m].special_kind == 7);
        let eff_hits = if p1_prot && hits > 0 { hits - 1 } else { hits };
        let use_phaze = p0_phaze && p0_perish >= 2;
        need_s1 = need_s1.max(eff_hits);
        if use_phaze {
            need_s0 = need_s0.max(1);
        }
        b2_info = Some((eff_hits, use_phaze));
        any_term = true;
    }

    let mut b3_hits: Option<usize> = None;
    if p1_perish == 0 && p1_trapped && !p0_trapped {
        let p0_has_song = (0..table.mons[idx0].num_moves as usize)
            .any(|m| cb.mons[idx0].pp[m] > 0 && table.move_specs[idx0][m].special_kind == 19);
        if p0_has_song {
            let p0_prot = (0..table.mons[idx0].num_moves as usize)
                .any(|m| cb.mons[idx0].pp[m] > 0 && table.move_specs[idx0][m].is_protect);
            let hits = if p0_prot { 2 } else { 3 };
            need_s0 = need_s0.max(hits);
            b3_hits = Some(hits);
            any_term = true;
        }
    }

    let mut b4_hits: Option<usize> = None;
    if p0_perish == 0 && p0_trapped && !p1_trapped {
        let p1_has_song = (0..table.mons[idx1].num_moves as usize)
            .any(|m| cb.mons[idx1].pp[m] > 0 && table.move_specs[idx1][m].special_kind == 19);
        if p1_has_song {
            let p1_prot = (0..table.mons[idx1].num_moves as usize)
                .any(|m| cb.mons[idx1].pp[m] > 0 && table.move_specs[idx1][m].is_protect);
            let hits = if p1_prot { 2 } else { 3 };
            need_s1 = need_s1.max(hits);
            b4_hits = Some(hits);
            any_term = true;
        }
    }

    if !any_term {
        return 0.0;
    }

    let (s0, _) = survival_curve_k_with_mean_hp(cb, table, 1, 0, hp0, need_s0, false);
    let (s1, _) = survival_curve_k_with_mean_hp(cb, table, 0, 1, hp1, need_s1, false);
    let mut margin = 0.0f64;

    if let Some((eff_hits, use_phaze)) = b1_info {
        let p_not_phazed = if use_phaze { 1.0 - s1[1] } else { 1.0 };
        margin += s0[eff_hits] * p_not_phazed;
    }
    if let Some((eff_hits, use_phaze)) = b2_info {
        let p_not_phazed = if use_phaze { 1.0 - s0[1] } else { 1.0 };
        margin -= s1[eff_hits] * p_not_phazed;
    }
    // Trapped before Perish Song is sung: probability trapper survives 1 extra turn to sing + countdown
    if let Some(hits) = b3_hits {
        margin += s0[hits];
    }
    if let Some(hits) = b4_hits {
        margin -= s1[hits];
    }

    margin.clamp(-1.0, 1.0)
}

#[inline]
fn side_best_move_low_pp(
    cb: &CompactBattle,
    table: &MatchTable,
    att_side: usize,
    att_slot: usize,
    def_side: usize,
    def_slot: usize,
) -> bool {
    let att_idx = att_side * 3 + att_slot;
    let mon = &cb.mons[att_idx];
    let spec = &table.mons[att_idx];
    let num_moves = spec.num_moves as usize;
    let dis = (mon.vol_flags & vol_bit::DISABLE) != 0;
    let dis_slot = ((mon.slot_refs >> 4) & 0x03) as usize;
    let last_mon = cb.sides[att_side].alive_mask.count_ones() <= 1;

    let mut best_dmg = 0.0f32;
    let mut best_pp = 0u8;
    let mut any_usable = false;
    for m_i in 0..num_moves {
        let pp = mon.pp[m_i];
        if pp == 0 || (dis && m_i == dis_slot) {
            continue;
        }
        let ms = &table.move_specs[att_idx][m_i];
        if last_mon && ms.is_selfdestruct {
            continue;
        }
        any_usable = true;
        let k = DmgKernel1D::for_move(cb, table, att_side, att_slot, def_side, def_slot, m_i);
        if k.mean_dmg > best_dmg {
            best_dmg = k.mean_dmg;
            best_pp = pp;
        }
    }
    if !any_usable {
        return true;
    }
    best_dmg > 0.0 && best_pp <= 2
}

#[inline]
fn side_has_non1d_1v1_wincon(
    cb: &CompactBattle,
    table: &MatchTable,
    side: usize,
    slot: usize,
    foe_side: usize,
    foe_slot: usize,
    is_trailing_1d: bool,
) -> bool {
    let idx = side * 3 + slot;
    let mon = &cb.mons[idx];
    let spec = &table.mons[idx];
    let foe_idx = foe_side * 3 + foe_slot;
    let foe = &cb.mons[foe_idx];
    let foe_spec = &table.mons[foe_idx];
    let dis = (mon.vol_flags & vol_bit::DISABLE) != 0;
    let dis_slot = ((mon.slot_refs >> 4) & 0x03) as usize;
    let foe_can_rest = (foe_spec.role_flags & role_bit::REST) != 0
        && (0..foe_spec.num_moves as usize)
            .any(|m| foe.pp[m] > 0 && table.move_specs[foe_idx][m].is_rest);

    for m_i in 0..spec.num_moves as usize {
        let pp = mon.pp[m_i];
        if pp == 0 || (dis && m_i == dis_slot) {
            continue;
        }
        let ms = &table.move_specs[idx][m_i];
        // Perish Song, Destiny Bond, Counter, Mirror Coat
        if matches!(ms.special_kind, 16 | 17 | 19 | 24) {
            return true;
        }
        if is_trailing_1d {
            // Unstacked offensive setup (`Curse`, `Swords Dance`, `Belly Drum`, `Growth`, `Double Team`)
            let can_boost_more = mon.boosts[0] < 3 || mon.boosts[2] < 3 || mon.boosts[6] < 3;
            if can_boost_more && (ms.boosts_self || matches!(ms.pseudo_kind, 5..=8)) {
                return true;
            }
            // Toxic / Leech Seed vs unstatused non-Rest foe
            if !foe_can_rest
                && ((foe.status_raw() == 0 && ms.primary_status == Status::Tox as u8)
                    || ms.primary_vol == 1)
            {
                return true;
            }
        }
    }
    false
}

/// Evaluate the `2v1` sequential attrition + trap-guarded `ZeroPpPivotLock` differential in `[-1.0, 1.0]`
/// strictly from `two_side`'s perspective (`2` alive mons vs `1` lone active mon on `one_side`),
/// along with `override_w in [0.0, 1.0]` indicating stall / `ZeroPpPivotLock` dominance.
#[inline]
fn eval_2v1_margin(
    cb: &CompactBattle,
    table: &MatchTable,
    two_side: usize,
    one_side: usize,
    need_base_seq: bool,
) -> (f64, f64) {
    let act_a = cb.sides[two_side].active_idx as usize;
    let lone_d = cb.sides[one_side].active_idx as usize;
    if (cb.sides[two_side].alive_mask & (1 << act_a)) == 0
        || (cb.sides[one_side].alive_mask & (1 << lone_d)) == 0
    {
        return (0.0, 0.0);
    }
    let mut bench_opt: Option<usize> = None;
    for slot in 0..3usize {
        if slot != act_a
            && (cb.sides[two_side].alive_mask & (1 << slot)) != 0
            && cb.mons[two_side * 3 + slot].hp() > 0
        {
            bench_opt = Some(slot);
            break;
        }
    }
    let Some(bench_b) = bench_opt else {
        return (0.0, 0.0);
    };

    let idx_a = two_side * 3 + act_a;
    let idx_b = two_side * 3 + bench_b;
    let idx_d = one_side * 3 + lone_d;
    let hp_a = cb.mons[idx_a].hp();
    let hp_b = cb.mons[idx_b].hp();
    let hp_d = cb.mons[idx_d].hp();
    if hp_a <= 0 || hp_b <= 0 || hp_d <= 0 {
        return (0.0, 0.0);
    }
    let maxhp_a = table.mons[idx_a].maxhp.max(1);
    let maxhp_b = table.mons[idx_b].maxhp.max(1);
    let maxhp_d = table.mons[idx_d].maxhp.max(1);

    let a_trapped = (cb.mons[idx_a].vol_flags & vol_bit::TRAPPED_OR_LOCKED) != 0;
    let b_trapped = (cb.mons[idx_b].vol_flags & vol_bit::TRAPPED_OR_LOCKED) != 0;
    let a_perish = (cb.mons[idx_a].vol_flags & vol_bit::PERISH_SONG) != 0;
    let d_dis = (cb.mons[idx_d].vol_flags & vol_bit::DISABLE) != 0;
    let d_dis_slot = ((cb.mons[idx_d].slot_refs >> 4) & 0x03) as usize;
    let num_m_d = table.mons[idx_d].num_moves as usize;
    let d_has_trap_move = (0..num_m_d).any(|m_i| {
        cb.mons[idx_d].pp[m_i] > 0
            && !(d_dis && m_i == d_dis_slot)
            && table.move_specs[idx_d][m_i].special_kind == 26
    });
    let d_heals = side_has_usable_heal_ext(cb, table, one_side, lone_d, true);
    let d_miracle = cb.mons[idx_d].item != 0 && table.mons[idx_d].is_miracleberry;
    let d_st_raw = cb.mons[idx_d].status_raw();
    let d_p_deduct = if d_st_raw == Status::Frz as u8 && !d_miracle {
        0.0
    } else if d_st_raw == Status::Par as u8 && !d_miracle {
        0.75
    } else {
        1.0
    };

    let can_try_lock = !a_trapped && !b_trapped && !a_perish && !d_has_trap_move && d_p_deduct > 0.0;
    // Fast O(1) early-exit: when `old_race_margin != 0` (`!need_base_seq`), `!d_heals`, and
    // `ZeroPpPivotLock` is blocked, `override_w` and `base_term` are both identically `0.0`.
    if !need_base_seq && !d_heals && !can_try_lock {
        return (0.0, 0.0);
    }

    // Fast path for full-PP `2v1` endgames when `lone_d` has ample PP (`d_lam == 0`)
    // and neither `act_a` nor `bench_b` is immune to `lone_d`'s attacks (`v_mat` parity).
    {
        let d_lam = side_pp_scarcity_lambda(cb, table, one_side, lone_d, two_side, act_a)
            .max(side_pp_scarcity_lambda(cb, table, one_side, lone_d, two_side, bench_b));
        if d_lam <= 0.0 {
            let mut d_atk_pp_raw = 0u32;
            let mut any_hits_a = false;
            let mut any_hits_b = false;
            let foresighted_a = (cb.mons[idx_a].vol_flags & vol_bit::FORESIGHT) != 0;
            let foresighted_b = (cb.mons[idx_b].vol_flags & vol_bit::FORESIGHT) != 0;
            for m_i in 0..num_m_d {
                let pp = cb.mons[idx_d].pp[m_i];
                if pp == 0 || (d_dis && m_i == d_dis_slot) {
                    continue;
                }
                let ms = &table.move_specs[idx_d][m_i];
                if ms.is_status || ms.is_selfdestruct {
                    continue;
                }
                d_atk_pp_raw += pp as u32;
                let (_, imm_a) =
                    table.effectiveness_ext(ms.move_type, &table.mons[idx_a], foresighted_a);
                let (_, imm_b) =
                    table.effectiveness_ext(ms.move_type, &table.mons[idx_b], foresighted_b);
                if !imm_a {
                    any_hits_a = true;
                }
                if !imm_b {
                    any_hits_b = true;
                }
            }
            if d_atk_pp_raw > 8 && any_hits_a && any_hits_b {
                let r1 =
                    toeplitz_coupled_race_ext(cb, table, two_side, act_a, one_side, lone_d, true);
                if r1 >= 0.999 {
                    return (r1, 0.0);
                }
                let r2 =
                    toeplitz_coupled_race_ext(cb, table, two_side, bench_b, one_side, lone_d, true);
                return (r1.max(r2), 0.0);
            }
        }
    }

    let flying_mask = 1u32 << (table.flying_type & 31);
    let spikes_layers = (cb.sides[two_side].side_flags & 0x03) as usize;
    const SPK_NUM: [i32; 4] = [0, 3, 4, 6];
    let spk_hp_a = if spikes_layers > 0 && (table.mons[idx_a].type_mask & flying_mask) == 0 {
        ((maxhp_a as i32 * SPK_NUM[spikes_layers]) / 24).max(1) as i16
    } else {
        0
    };
    let spk_hp_b = if spikes_layers > 0 && (table.mons[idx_b].type_mask & flying_mask) == 0 {
        ((maxhp_b as i32 * SPK_NUM[spikes_layers]) / 24).max(1) as i16
    } else {
        0
    };

    // Helper: evaluate sequential `first_slot -> second_slot` attrition against `lone_d` on `state`
    let eval_seq_with_p1 = |state: &CompactBattle,
                            first_s: usize,
                            second_s: usize,
                            p1_opt: Option<f64>|
     -> f64 {
        let p1 = match p1_opt {
            Some(v) => v,
            None => {
                let r1 =
                    toeplitz_coupled_race_ext(state, table, two_side, first_s, one_side, lone_d, true);
                (0.5 * (1.0 + r1)).clamp(0.0, 1.0)
            }
        };
        if p1 >= 0.999 {
            return p1;
        }
        if !d_heals && p1 >= 0.90 {
            let r2 =
                toeplitz_coupled_race_ext(state, table, two_side, second_s, one_side, lone_d, true);
            let p2 = (0.5 * (1.0 + r2)).clamp(0.0, 1.0);
            return (p1 + (1.0 - p1) * p2).clamp(0.0, 1.0);
        }
        // Estimate expected turns `e_tau` spent in fight 1 (`first_s` vs `lone_d`)
        let first_idx = two_side * 3 + first_s;
        let first_h = (state.mons[first_idx].hp() as f64)
            / (table.mons[first_idx].maxhp.max(1) as f64);
        let (prof_f, prof_d, e_tau, r_d_atk, r_d_heal, r_d_st) = if side_has_usable_heal_ext(
            state, table, two_side, first_s, true,
        )
            || side_has_usable_heal_ext(state, table, one_side, lone_d, true)
        {
            let fp_ord = table.mons[first_idx]
                .static_fp
                .cmp(&table.mons[idx_d].static_fp);
            let swapped = if fp_ord != std::cmp::Ordering::Equal {
                fp_ord == std::cmp::Ordering::Greater
            } else {
                side_canon_key(state, table, two_side, first_s)
                    > side_canon_key(state, table, one_side, lone_d)
            };
            let (cs0, cslot0, cs1, cslot1) = if !swapped {
                (two_side, first_s, one_side, lone_d)
            } else {
                (one_side, lone_d, two_side, first_s)
            };
            with_solved_bellman_finite(state, table, cs0, cslot0, cs1, cslot1, |cached, meta| {
                let d_h = (state.mons[idx_d].hp() as f64) / (maxhp_d as f64);
                let slp_f = side_sleep_rem(state, table, two_side, first_s).min(2);
                let slp_d = side_sleep_rem(state, table, one_side, lone_d).min(2);
                let (w0, w1, d_side, pf, pd_prof) = if !swapped {
                    (
                        bilinear_b16_u16(&cached.w0_5[slp_f][slp_d], first_h, d_h),
                        bilinear_b16_u16(&cached.w1_5[slp_f][slp_d], first_h, d_h),
                        1usize,
                        meta.sides[0].instantiate(&state.mons[first_idx]),
                        meta.sides[1].instantiate(&state.mons[idx_d]),
                    )
                } else {
                    (
                        bilinear_b16_u16(&cached.w0_5[slp_d][slp_f], d_h, first_h),
                        bilinear_b16_u16(&cached.w1_5[slp_d][slp_f], d_h, first_h),
                        0usize,
                        meta.sides[1].instantiate(&state.mons[first_idx]),
                        meta.sides[0].instantiate(&state.mons[idx_d]),
                    )
                };
                let psi = (1.0 - w0 - w1).clamp(0.0, 1.0);
                let kap_tot = (cached.kappa[0] as f64 + cached.kappa[1] as f64).max(0.02);
                let hor = pf
                    .raw_usable_pp()
                    .max(1.0)
                    .min(pd_prof.raw_usable_pp().max(1.0) * 1.5);
                let et = 5.0 * (1.0 - psi) + psi * (5.0 + phi1_decay(kap_tot, hor));
                (
                    pf,
                    pd_prof,
                    et,
                    cached.r_atk_act[d_side] as f64,
                    cached.r_heal_act[d_side] as f64,
                    cached.r_st_act[d_side] as f64,
                )
            })
        } else {
            let pf = SidePpProfile::build(state, table, two_side, first_s, one_side, lone_d);
            let pd_prof = SidePpProfile::build(state, table, one_side, lone_d, two_side, first_s);
            let d_atk = if pd_prof.is_struggling() {
                StrugglePairStats::build(state, table, two_side, first_s, one_side, lone_d)
                    .inc_str[0]
            } else {
                pd_prof.active_dmg_frac() * pd_prof.p_deduct
            };
            let d_drain = (d_atk - pf.net_eot_frac).max(0.08);
            let et = (first_h / d_drain).clamp(1.0, 24.0);
            (pf, pd_prof, et, 1.0, 0.0, 0.0)
        };

        // Deduct expected PP spent by `lone_d` onto stack copy `cb_next`
        let mut cb_next = *state;
        let pd = prof_d.p_deduct;
        let mut atk_burn = (e_tau * pd * r_d_atk).max(0.0);
        let mut heal_burn = (e_tau * pd * r_d_heal).max(0.0);
        let mut st_burn = (e_tau * pd * r_d_st).max(0.0);
        for pass_kind in [1u8, 2u8] {
            for m_i in 0..num_m_d {
                if prof_d.slot_kind[m_i] == pass_kind && atk_burn > 0.0 {
                    let cur = cb_next.mons[idx_d].pp[m_i] as f64;
                    let take = cur.min(atk_burn);
                    cb_next.mons[idx_d].pp[m_i] = (cur - take).floor().max(0.0) as u8;
                    atk_burn -= take;
                }
            }
        }
        for m_i in 0..num_m_d {
            match prof_d.slot_kind[m_i] {
                3 | 4 if heal_burn > 0.0 => {
                    let cur = cb_next.mons[idx_d].pp[m_i] as f64;
                    let take = cur.min(heal_burn);
                    cb_next.mons[idx_d].pp[m_i] = (cur - take).floor().max(0.0) as u8;
                    heal_burn -= take;
                }
                5 if st_burn > 0.0 => {
                    let cur = cb_next.mons[idx_d].pp[m_i] as f64;
                    let take = cur.min(st_burn);
                    cb_next.mons[idx_d].pp[m_i] = (cur - take).floor().max(0.0) as u8;
                    st_burn -= take;
                }
                _ => {}
            }
        }
        if !prof_d.can_heal {
            let str_rec = if prof_d.is_struggling() {
                StrugglePairStats::build(state, table, two_side, first_s, one_side, lone_d)
                    .self_rec[1]
            } else {
                0.0
            };
            let d_faster = state.mon_action_speed(table, one_side, lone_d)
                > state.mon_action_speed(table, two_side, first_s);
            let f_turns = if d_faster { (e_tau - 1.0).max(0.0) } else { e_tau };
            let atk_chip =
                (f_turns * prof_f.active_dmg_frac() * prof_f.p_deduct * (maxhp_d as f64)).round()
                    as i16;
            let mut sub_hp = state.mons[idx_d].hp_pool[1];
            let mut rem_atk_chip = atk_chip.max(0);
            if sub_hp > 0 && rem_atk_chip > 0 {
                if rem_atk_chip >= sub_hp {
                    rem_atk_chip = 0;
                    sub_hp = 0;
                    cb_next.mons[idx_d].vol_flags &= !vol_bit::SUBSTITUTE;
                } else {
                    sub_hp -= rem_atk_chip;
                    rem_atk_chip = 0;
                }
                cb_next.mons[idx_d].hp_pool[1] = sub_hp;
            }
            let self_net_chip =
                (e_tau * (str_rec - prof_d.net_eot_frac) * (maxhp_d as f64)).round() as i16;
            let total_hp_chip = (rem_atk_chip + self_net_chip).max(0);
            cb_next.mons[idx_d].hp_pool[0] = (state.mons[idx_d].hp() - total_hp_chip).max(1);
        }

        let r2 =
            toeplitz_coupled_race_ext(&cb_next, table, two_side, second_s, one_side, lone_d, true);
        let p2 = (0.5 * (1.0 + r2)).clamp(0.0, 1.0);
        (p1 + (1.0 - p1) * p2).clamp(0.0, 1.0)
    };

    // Path 1: Stay in with active `act_a`, followed by `bench_b`
    let r_stay_1v1 =
        toeplitz_coupled_race_ext(cb, table, two_side, act_a, one_side, lone_d, true);
    let p_stay_1v1 = (0.5 * (1.0 + r_stay_1v1)).clamp(0.0, 1.0);
    if p_stay_1v1 >= 0.999 {
        return (1.0, 0.0);
    }
    let mut best_p = eval_seq_with_p1(cb, act_a, bench_b, Some(p_stay_1v1));

    // Path 2: Voluntary switch `act_a -> bench_b` on Turn 1 (only when `act_a` is passive/losing)
    if !a_trapped && best_p < 0.80 && p_stay_1v1 < 0.55 {
        let hp_b_post_spk = hp_b - spk_hp_b;
        if hp_b_post_spk > 0 {
            let d_slp = side_sleep_rem(cb, table, one_side, lone_d);
            let k_d_to_b = if d_slp == 0 {
                DmgKernel1D::best_attack(cb, table, one_side, lone_d, two_side, bench_b)
            } else {
                DmgKernel1D::sleep_talk(cb, table, one_side, lone_d, two_side, bench_b)
            };
            let prof_b = SidePpProfile::build(cb, table, two_side, bench_b, one_side, lone_d);
            let entry_hit = k_d_to_b.mean_dmg.round() as i16;
            let eot_b = (prof_b.net_eot_frac * maxhp_b as f64).round() as i16;
            let hp_b_after = (hp_b_post_spk - entry_hit + eot_b).clamp(0, maxhp_b);
            if hp_b_after > 0 {
                let mut cb_sw = *cb;
                cb_sw.sides[two_side].active_idx = bench_b as u8;
                cb_sw.mons[idx_a].vol_flags = 0;
                cb_sw.mons[idx_a].boosts = [0; 7];
                cb_sw.mons[idx_a].hp_pool[1] = 0;
                cb_sw.mons[idx_b].hp_pool[0] = hp_b_after;
                let p_sw = eval_seq_with_p1(&cb_sw, bench_b, act_a, None);
                if p_sw > best_p {
                    best_p = p_sw;
                }
            }
        }
    }

    let mut lock_override = 0.0f64;

    // Lazy `ZeroPpPivotLock`: evaluated ONLY when direct sequential attrition fails (`best_p < 0.72`)
    if can_try_lock && best_p < 0.72 {
        let mut d_eff_a = 0.0f64;
        let mut d_mean_a = 0i16;
        let mut d_max_a = 0i16;
        let mut d_eff_b = 0.0f64;
        let mut d_mean_b = 0i16;
        let mut d_max_b = 0i16;
        let mut d_atk_pp = 0.0f64;
        let mut d_pas_pp = 0.0f64;

        for m_i in 0..num_m_d {
            let pp_u8 = cb.mons[idx_d].pp[m_i];
            if pp_u8 == 0 || (d_dis && m_i == d_dis_slot) {
                continue;
            }
            let ms = &table.move_specs[idx_d][m_i];
            if ms.is_selfdestruct {
                continue;
            }
            let pp = pp_u8 as f64;
            let k_a = DmgKernel1D::for_move(cb, table, one_side, lone_d, two_side, act_a, m_i);
            let k_b = DmgKernel1D::for_move(cb, table, one_side, lone_d, two_side, bench_b, m_i);

            let ma = k_a.mean_dmg.round() as i16;
            if ma > d_mean_a {
                d_mean_a = ma;
            }
            if k_a.max_dmg > d_max_a {
                d_max_a = k_a.max_dmg;
            }
            let ea = (k_a.mean_dmg as f64 + 0.35 * ((k_a.max_dmg as f64) - (k_a.mean_dmg as f64)))
                / (maxhp_a as f64);
            if ea > d_eff_a {
                d_eff_a = ea;
            }

            let mb = k_b.mean_dmg.round() as i16;
            if mb > d_mean_b {
                d_mean_b = mb;
            }
            if k_b.max_dmg > d_max_b {
                d_max_b = k_b.max_dmg;
            }
            let eb = (k_b.mean_dmg as f64 + 0.35 * ((k_b.max_dmg as f64) - (k_b.mean_dmg as f64)))
                / (maxhp_b as f64);
            if eb > d_eff_b {
                d_eff_b = eb;
            }

            if k_a.max_dmg > 0 || k_b.max_dmg > 0 {
                d_atk_pp += pp;
            } else if ms.special_kind == 10 {
                d_atk_pp += 0.34 * pp;
                d_pas_pp += 0.66 * pp;
            } else {
                d_pas_pp += pp;
            }
        }

        if hp_a > spk_hp_a
            && hp_b > spk_hp_b
            && (hp_b > spk_hp_b + d_mean_b || d_max_a == 0)
            && (hp_a > spk_hp_a + d_mean_a || d_max_b == 0)
        {
            let spk_frac_a = (spk_hp_a as f64) / (maxhp_a as f64);
            let spk_frac_b = (spk_hp_b as f64) / (maxhp_b as f64);
            let prof_a = SidePpProfile::build(cb, table, two_side, act_a, one_side, lone_d);
            let prof_b = SidePpProfile::build(cb, table, two_side, bench_b, one_side, lone_d);
            let net_atk_drain_a = spk_frac_a + d_eff_a - prof_a.net_eot_frac;
            let net_atk_drain_b = spk_frac_b + d_eff_b - prof_b.net_eot_frac;
            let net_pas_drain_a = spk_frac_a - prof_a.net_eot_frac;
            let net_pas_drain_b = spk_frac_b - prof_b.net_eot_frac;
            let n_atk_turns = d_atk_pp / d_p_deduct;
            let n_pas_turns = d_pas_pp / d_p_deduct;
            let total_d_turns = n_atk_turns + n_pas_turns;

            let immune_lock = (d_max_a == 0 && net_pas_drain_a <= 0.0 && net_pas_drain_b <= 0.0)
                || (d_max_b == 0 && net_pas_drain_b <= 0.0 && net_pas_drain_a <= 0.0);

            let eta = if immune_lock || (net_atk_drain_a <= 0.0 && net_atk_drain_b <= 0.0) {
                1.0
            } else {
                let req_hp_bars = 0.5
                    * n_atk_turns
                    * (net_atk_drain_a.max(0.0) + net_atk_drain_b.max(0.0))
                    + 0.5 * n_pas_turns * (net_pas_drain_a.max(0.0) + net_pas_drain_b.max(0.0));
                let cap_of = |hp_cur: i16, mx: i16, pr: &SidePpProfile, d_eff: f64| -> f64 {
                    let h_spare = ((hp_cur as f64 / mx as f64) - 0.35).max(0.0);
                    let inst_h =
                        pr.pp_inst_heal * ((pr.inst_heal_amt as f64) / (mx as f64)).max(0.25);
                    let rest_h = if !pr.has_berry && d_eff < 0.42 {
                        pr.pp_rest * (0.85 - 2.0 * d_eff.max(0.0)).max(0.0)
                    } else if pr.has_berry {
                        pr.pp_rest * 0.75
                    } else {
                        0.0
                    };
                    h_spare + inst_h + rest_h
                };
                let total_cap =
                    cap_of(hp_a, maxhp_a, &prof_a, d_eff_a) + cap_of(hp_b, maxhp_b, &prof_b, d_eff_b);
                if req_hp_bars <= 1e-4 {
                    1.0
                } else if d_eff_a + spk_frac_a < 0.52 && d_eff_b + spk_frac_b < 0.52 {
                    ((total_cap / req_hp_bars - 1.10) / 0.80).clamp(0.0, 1.0)
                } else {
                    0.0
                }
            };
            if eta > 0.0 {
                let mut cb_0pp = *cb;
                cb_0pp.mons[idx_d].pp = [0; 4];
                cb_0pp.mons[idx_d].set_status(0, 0);
                if net_pas_drain_a < 0.0 && n_pas_turns > 0.0 {
                    let gain_a =
                        (0.5 * n_pas_turns * (-net_pas_drain_a) * (maxhp_a as f64)).round() as i16;
                    cb_0pp.mons[idx_a].hp_pool[0] = (hp_a + gain_a).min(maxhp_a);
                }
                if net_pas_drain_b < 0.0 && n_pas_turns > 0.0 {
                    let gain_b =
                        (0.5 * n_pas_turns * (-net_pas_drain_b) * (maxhp_b as f64)).round() as i16;
                    cb_0pp.mons[idx_b].hp_pool[0] = (hp_b + gain_b).min(maxhp_b);
                }
                let p_ab = eval_seq_with_p1(&cb_0pp, act_a, bench_b, None);
                let p_ba = if p_ab >= 0.999 {
                    p_ab
                } else {
                    eval_seq_with_p1(&cb_0pp, bench_b, act_a, None)
                };
                let p_0pp = p_ab.max(p_ba);
                let p_lock = 0.5 + (p_0pp - 0.5) * (-LAMBDA_SLOW * total_d_turns.min(100.0)).exp();
                let p_seq_before = best_p;
                let p_blend = best_p + eta * (p_lock - best_p).max(0.0);
                if p_blend > best_p {
                    best_p = p_blend;
                    let gain_w = (eta * ((p_lock - p_seq_before) / 0.20)).clamp(0.0, 1.0);
                    if gain_w > lock_override {
                        lock_override = gain_w;
                    }
                }
            }
        }
    }

    ((2.0 * best_p - 1.0).clamp(-1.0, 1.0), lock_override)
}

/// Combined linear-algebra differential added to `compact_eval01` (scaled by `w.scale`).
/// Strictly antisymmetric: `compact_linear_diff(cb) == -compact_linear_diff(swapped(cb))`.
#[inline]
pub fn compact_linear_diff(
    cb: &CompactBattle,
    table: &MatchTable,
    base_diff: f64,
    old_race_margin: f64,
    race_weight: f64,
    scale: f64,
) -> f64 {
    compact_linear_diff_ext(cb, table, base_diff, old_race_margin, race_weight, scale, true)
}

#[inline]
pub fn compact_linear_diff_ext(
    cb: &CompactBattle,
    table: &MatchTable,
    base_diff: f64,
    old_race_margin: f64,
    race_weight: f64,
    scale: f64,
    stall_pp: bool,
) -> f64 {
    let mut delta = compact_perish_margin(cb, table);

    let n0 = cb.sides[0].alive_mask.count_ones();
    let n1 = cb.sides[1].alive_mask.count_ones();
    if n0 == 0 || n1 == 0 {
        return delta;
    }
    let s0 = cb.sides[0].active_idx as usize;
    let s1 = cb.sides[1].active_idx as usize;
    if (cb.sides[0].alive_mask & (1 << s0)) == 0
        || (cb.sides[1].alive_mask & (1 << s1)) == 0
        || race_weight <= 0.0
    {
        return delta;
    }

    if n0 == 1 && n1 == 1 {
        let tr = toeplitz_coupled_race_ext(cb, table, 0, s0, 1, s1, stall_pp);
        let m0 = &cb.mons[s0];
        let m1 = &cb.mons[3 + s1];
        let spd0 = cb.mon_action_speed(table, 0, s0);
        let spd1 = cb.mon_action_speed(table, 1, s1);
        let is_frz = m0.status_raw() == Status::Frz as u8 || m1.status_raw() == Status::Frz as u8;
        let is_rech = ((m0.slot_refs | m1.slot_refs) & 0x80) != 0;
        let has_perish_or_db =
            ((m0.vol_flags | m1.vol_flags) & (vol_bit::PERISH_SONG | vol_bit::DESTINY_BOND)) != 0;
        let pp_scarce = stall_pp
            && (side_pp_scarcity_lambda(cb, table, 0, s0, 1, s1) > 0.0
                || side_pp_scarcity_lambda(cb, table, 1, s1, 0, s0) > 0.0);
        let finite_pp_override = pp_scarce
            && !has_perish_or_db
            && (side_has_usable_heal_ext(cb, table, 0, s0, true)
                || side_has_usable_heal_ext(cb, table, 1, s1, true)
                || side_best_move_low_pp(cb, table, 0, s0, 1, s1)
                || side_best_move_low_pp(cb, table, 1, s1, 0, s0))
            && !side_has_non1d_1v1_wincon(cb, table, 0, s0, 1, s1, tr < -0.15)
            && !side_has_non1d_1v1_wincon(cb, table, 1, s1, 0, s0, tr > 0.15);
        let exact_mechanics_override = finite_pp_override
            || spd0 == spd1
            || m0.status_raw() == Status::Slp as u8
            || m1.status_raw() == Status::Slp as u8
            || is_frz
            || is_rech
            || (m0.vol_flags & vol_bit::CONFUSION) != 0
            || (m1.vol_flags & vol_bit::CONFUSION) != 0
            || old_race_margin == 0.0
            || (tr * old_race_margin > 0.0);

        if exact_mechanics_override {
            if finite_pp_override && !is_frz && scale > 0.0 {
                let p_lin = (0.5 * (1.0 + tr)).clamp(1e-5, 1.0 - 1e-5);
                let logit_tr = (p_lin / (1.0 - p_lin)).ln() / scale;
                delta += logit_tr - base_diff;
            } else if (tr.abs() < 0.99 || is_rech) && !is_frz && scale > 0.0 {
                let p_lin = (0.5 * (1.0 + tr)).clamp(0.005, 0.995);
                let logit_tr =
                    ((p_lin / (1.0 - p_lin)).ln() / scale).clamp(-race_weight, race_weight);
                delta += logit_tr - base_diff;
            } else {
                delta += race_weight * (tr - old_race_margin);
            }
        }
    } else if n0 + n1 == 3
        && ((cb.mons[s0].vol_flags | cb.mons[3 + s1].vol_flags) & vol_bit::PERISH_SONG) == 0
    {
        if stall_pp {
            let need_base_seq = old_race_margin == 0.0;
            let two_sign = if n0 == 2 && n1 == 1 { 1.0 } else { -1.0 };
            let (v_2v1, override_w) = if n0 == 2 && n1 == 1 {
                eval_2v1_margin(cb, table, 0, 1, need_base_seq)
            } else {
                let (v, ow) = eval_2v1_margin(cb, table, 1, 0, need_base_seq);
                (-v, ow)
            };
            let base_term = if old_race_margin == 0.0 {
                0.5 * race_weight * v_2v1
            } else {
                0.0
            };
            if scale > 0.0 && override_w > 0.0 {
                let p_2v1 = (0.5 * (1.0 + v_2v1)).clamp(1e-4, 1.0 - 1e-4);
                let logit_2v1 = ((p_2v1 / (1.0 - p_2v1)).ln() / scale)
                    .clamp(-3.5 * race_weight, 3.5 * race_weight);
                let lift_for_two = two_sign * ((logit_2v1 - base_diff) - base_term);
                if lift_for_two > 0.0 {
                    let w_b = (override_w * 0.96).clamp(0.0, 0.96);
                    delta += w_b * (logit_2v1 - base_diff) + (1.0 - w_b) * base_term;
                } else {
                    delta += base_term;
                }
            } else {
                delta += base_term;
            }
        } else if old_race_margin == 0.0 {
            // Legacy static saddle-point value V_mat(A_0, A_1) over alive pairs (a, d)
            let mut mine = [0usize; 3];
            let mut k0 = 0usize;
            let mut theirs = [0usize; 3];
            let mut k1 = 0usize;
            for slot in 0..3usize {
                if (cb.sides[0].alive_mask & (1 << slot)) != 0 && cb.mons[slot].hp() > 0 {
                    mine[k0] = slot;
                    k0 += 1;
                }
                if (cb.sides[1].alive_mask & (1 << slot)) != 0 && cb.mons[3 + slot].hp() > 0 {
                    theirs[k1] = slot;
                    k1 += 1;
                }
            }
            if k0 > 0 && k1 > 0 {
                let mut mat = [[0.0f64; 3]; 3];
                for (i, &a) in mine[..k0].iter().enumerate() {
                    for (j, &d) in theirs[..k1].iter().enumerate() {
                        mat[i][j] = toeplitz_coupled_race_ext(cb, table, 0, a, 1, d, false);
                    }
                }
                let mut maximin = f64::NEG_INFINITY;
                for row in &mat[..k0] {
                    let mut rmin = f64::INFINITY;
                    for &v in &row[..k1] {
                        if v < rmin {
                            rmin = v;
                        }
                    }
                    if rmin > maximin {
                        maximin = rmin;
                    }
                }
                let mut minimax = f64::INFINITY;
                for j in 0..k1 {
                    let mut cmax = f64::NEG_INFINITY;
                    for row in &mat[..k0] {
                        if row[j] > cmax {
                            cmax = row[j];
                        }
                    }
                    if cmax < minimax {
                        minimax = cmax;
                    }
                }
                let v_mat = 0.5 * (maximin + minimax);
                delta += 0.5 * race_weight * v_mat;
            }
        }
    }

    delta
}

/// Bellman-Markov rollout action selector (zero magic thresholds).
#[inline]
pub fn compact_rollout_linear_pick(
    cb: &CompactBattle,
    table: &MatchTable,
    side: usize,
    cs: &[u8],
) -> Option<u8> {
    compact_rollout_linear_pick_ext(cb, table, side, cs, true)
}

#[inline]
pub fn compact_rollout_linear_pick_ext(
    cb: &CompactBattle,
    table: &MatchTable,
    side: usize,
    cs: &[u8],
    stall_pp: bool,
) -> Option<u8> {
    let att_slot = cb.sides[side].active_idx as usize;
    let att_idx = side * 3 + att_slot;
    let def_side = 1 - side;
    let def_slot = cb.sides[def_side].active_idx as usize;
    let def_idx = def_side * 3 + def_slot;

    if cb.sides[side].request != 1
        || (cb.sides[side].alive_mask & (1 << att_slot)) == 0
        || (cb.sides[def_side].alive_mask & (1 << def_slot)) == 0
    {
        return None;
    }

    let me = &cb.mons[att_idx];
    let foe = &cb.mons[def_idx];
    let maxhp = table.mons[att_idx].maxhp.max(1);
    let foe_maxhp = table.mons[def_idx].maxhp.max(1);

    // 1. Canonical sleeping Markov action when guaranteed asleep this turn (`status_counter >= 2`)
    let is_asleep_this_turn = me.status_raw() == Status::Slp as u8
        && me.status_counter() >= 2
        && !(me.item != 0
            && (table.mons[att_idx].is_mintberry || table.mons[att_idx].is_miracleberry));
    if is_asleep_this_turn {
        if (table.mons[att_idx].role_flags & role_bit::SLEEP_USABLE) == 0 {
            return None;
        }
        for &c in cs {
            if c < 4 && table.move_specs[att_idx][c as usize].special_kind == 10 {
                return Some(c);
            }
        }
        for &c in cs {
            if c < 4 && table.move_specs[att_idx][c as usize].is_sleep_usable {
                return Some(c);
            }
        }
        return None;
    }

    // 2. Protect during active Perish countdown (`perish_turns >= 2`) when trapping
    let me_perish = if (me.vol_flags & vol_bit::PERISH_SONG) != 0 {
        me.perish_turns()
    } else {
        0
    };
    let foe_perish = if (foe.vol_flags & vol_bit::PERISH_SONG) != 0 {
        foe.perish_turns()
    } else {
        0
    };
    if foe_perish >= 2
        && (table.mons[att_idx].role_flags & role_bit::PROTECT) != 0
        && me.stall_counter() == 0
    {
        let me_trapped = (me.vol_flags & vol_bit::TRAPPED_OR_LOCKED) != 0
            || cb.sides[side].alive_mask.count_ones() <= 1;
        let foe_trapped = (foe.vol_flags & vol_bit::TRAPPED_OR_LOCKED) != 0
            || cb.sides[def_side].alive_mask.count_ones() <= 1;
        if foe_trapped && !me_trapped {
            for &c in cs {
                if c < 4 && table.move_specs[att_idx][c as usize].is_protect {
                    return Some(c);
                }
            }
        }
    }

    let me_hp = me.hp();
    let is_frz = me.status_raw() == Status::Frz as u8
        && !(me.item != 0 && table.mons[att_idx].is_miracleberry);
    if is_frz {
        return None;
    }

    let foe_has_perish = (table.mons[def_idx].role_flags & role_bit::PERISH_PHAZE_TRAP) != 0
        && (0..table.mons[def_idx].num_moves as usize)
            .any(|m| foe.pp[m] > 0 && table.move_specs[def_idx][m].special_kind == 19);

    // 3. Finite-PP 1-step Bellman Q-value selection in genuine 1v1 PP-exhaustion matchups
    let n_alive_sum = cb.sides[0].alive_mask.count_ones() + cb.sides[1].alive_mask.count_ones();
    if stall_pp
        && n_alive_sum == 2
        && !foe_has_perish
        && me_perish == 0
        && (side_has_usable_heal_ext(cb, table, 0, cb.sides[0].active_idx as usize, true)
            || side_has_usable_heal_ext(cb, table, 1, cb.sides[1].active_idx as usize, true))
    {
        let mut move_cs = [0u8; 4];
        let mut n_mc = 0usize;
        for &c in cs {
            if c < 4 {
                move_cs[n_mc] = c;
                n_mc += 1;
            }
        }
        if n_mc >= 2 {
            // Check whether finite-PP Q-value disambiguation is active (scarce `<= 4` PP heal, scarce `<= 4` PP
            // breaker with secondary attack alt, or unbreakable `< 25%` full-HP Rest wall with `<= 20` PP)
            let mut has_scarce_heal = false;
            let mut has_scarce_atk = false;
            let mut has_alt_atk = false;
            let mut has_non_brk = false;
            let mut best_eff = -1.0f64;
            let mut greedy_c = move_cs[0];
            for &c in &move_cs[..n_mc] {
                let m_i = c as usize;
                let ms = &table.move_specs[att_idx][m_i];
                if instant_heal_amount(cb, ms, maxhp).is_some() || ms.is_rest {
                    if me.pp[m_i] <= 4 {
                        has_scarce_heal = true;
                    }
                    has_non_brk = true;
                } else {
                    let eff = cb.expected_hit_fraction(
                        table, side, att_slot, def_side, def_slot, m_i, true,
                    );
                    if eff > best_eff {
                        best_eff = eff;
                        greedy_c = c;
                    }
                }
            }
            for &c in &move_cs[..n_mc] {
                let m_i = c as usize;
                let ms = &table.move_specs[att_idx][m_i];
                if instant_heal_amount(cb, ms, maxhp).is_some() || ms.is_rest {
                    continue;
                }
                if c == greedy_c {
                    if me.pp[m_i] <= 4 {
                        has_scarce_atk = true;
                    }
                } else {
                    has_non_brk = true;
                    if !ms.is_status && !ms.is_selfdestruct {
                        has_alt_atk = true;
                    }
                }
            }
            let foe_raw_pp: u32 = (0..table.mons[def_idx].num_moves as usize)
                .map(|m_i| foe.pp[m_i] as u32)
                .sum();
            let foe_full_rest = best_eff < 0.25
                && foe_raw_pp <= 20
                && foe.hp() >= foe_maxhp
                && foe.status_raw() != Status::Slp as u8
                && (table.mons[def_idx].role_flags & role_bit::REST) != 0
                && (table.mons[def_idx].role_flags & role_bit::HALF_HEAL) == 0;

            if (has_scarce_heal && me_hp < maxhp)
                || (has_scarce_atk && has_alt_atk)
                || (foe_full_rest && has_non_brk)
            {
                let me_faster = cb.mon_action_speed(table, side, att_slot)
                    > cb.mon_action_speed(table, def_side, def_slot);
                if !(me_faster && best_eff >= 1.0) {
                    let (s0_slot, s1_slot) = if side == 0 {
                        (att_slot, def_slot)
                    } else {
                        (def_slot, att_slot)
                    };
                    let picked =
                        with_solved_bellman_finite(cb, table, 0, s0_slot, 1, s1_slot, |cached, meta| {
                        let me_sub_up = (me.vol_flags & vol_bit::SUBSTITUTE) != 0;
                        let mut best_q = f64::NEG_INFINITY;
                        let mut best_c = greedy_c;
                        let mut greedy_q = f64::NEG_INFINITY;
                        for &c in &move_cs[..n_mc] {
                            let m_i = c as usize;
                            let ms = &table.move_specs[att_idx][m_i];
                            let is_h = instant_heal_amount(cb, ms, maxhp).is_some() || ms.is_rest;
                            if is_h && (me_hp >= maxhp || (me_sub_up && me_hp * 4 >= maxhp)) {
                                continue;
                            }
                            if !is_h && ms.is_status && !foe_full_rest {
                                continue;
                            }
                            let q = eval_one_step_move_q(cached, meta, cb, table, side, m_i);
                            if c == greedy_c {
                                greedy_q = q;
                            }
                            if q > best_q + 1e-9 {
                                best_q = q;
                                best_c = c;
                            }
                        }
                        if best_c != greedy_c && best_q > greedy_q + 2e-4 {
                            Some(best_c)
                        } else {
                            None
                        }
                    });
                    if picked.is_some() {
                        return picked;
                    }
                }
            }
        }
    }

    // 4. Fast O(1) Bellman stage-game policy lookup for Heal / Rest when awake and strictly below full HP
    let me_sub_up = (me.vol_flags & vol_bit::SUBSTITUTE) != 0;
    if (table.mons[att_idx].role_flags & (role_bit::HALF_HEAL | role_bit::REST)) != 0
        && me_perish == 0
        && me_hp < maxhp
        && !(me_sub_up && me_hp * 4 >= maxhp)
    {
        let b_me = Side1DOps::to_b16(me_hp, maxhp) as usize;
        if b_me < B16 {
            let mut heal_act: Option<u8> = None;
            let mut heal_pp = 0u8;
            let mut heal_is_inst = false;
            for &c in cs {
                if c < 4 {
                    let ms = &table.move_specs[att_idx][c as usize];
                    let is_inst = ms.is_half_heal || (stall_pp && ms.special_kind == 29);
                    if is_inst {
                        heal_act = Some(c);
                        heal_pp = me.pp[c as usize];
                        heal_is_inst = true;
                        break;
                    } else if ms.is_rest && heal_act.is_none() {
                        heal_act = Some(c);
                        heal_pp = me.pp[c as usize];
                    }
                }
            }
            if let Some(h_act) = heal_act {
                // Avoid prematurely burning scarce `<= 8` PP instant heals above 80% HP when safe
                if stall_pp
                    && heal_is_inst
                    && heal_pp <= 8
                    && i32::from(me_hp) * 5 >= i32::from(maxhp) * 4
                    && cb.best_hit_fraction(table, def_side, def_slot, side, att_slot, true) < 0.70
                {
                    return None;
                }
                if !foe_has_perish {
                    let me_faster = cb.mon_action_speed(table, side, att_slot)
                        > cb.mon_action_speed(table, def_side, def_slot);
                    if me_faster
                        && cb.best_hit_fraction(table, side, att_slot, def_side, def_slot, true)
                            >= 1.0
                    {
                        return None;
                    }
                    let b_foe = Side1DOps::to_b16(foe.hp(), foe_maxhp) as usize;
                    let (s0_slot, s1_slot, b0, b1) = if side == 0 {
                        (att_slot, def_slot, b_me, b_foe)
                    } else {
                        (def_slot, att_slot, b_foe, b_me)
                    };
                    let bellman_wants_heal =
                        with_solved_bellman_masks(cb, table, 0, s0_slot, 1, s1_slot, |cached| {
                            if side == 0 {
                                ((cached.heal_mask0[b0] >> b1) & 1) != 0
                            } else {
                                ((cached.heal_mask1[b1] >> b0) & 1) != 0
                            }
                        });
                    if bellman_wants_heal {
                        return Some(h_act);
                    }
                }
            }
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use nc2000_engine::battle::PokemonSet;
    use nc2000_engine::state::Battle;

    fn make_set(species: &str, level: u8, item: &str, moves: &[&str]) -> PokemonSet {
        serde_json::from_value(serde_json::json!({
            "name": species,
            "species": species,
            "item": item,
            "ability": "No Ability",
            "moves": moves,
            "level": level,
            "evs": {"hp": 255, "atk": 255, "def": 255, "spa": 255, "spd": 255, "spe": 255},
            "ivs": {"hp": 30, "atk": 30, "def": 30, "spa": 30, "spd": 30, "spe": 30},
            "happiness": 255,
            "gender": "M"
        }))
        .unwrap()
    }

    #[test]
    fn schur_bellman_svd_is_antisymmetric_and_low_rank() {
        let dex = conformance::load_dex();
        let lax = make_set(
            "Snorlax",
            55,
            "Leftovers",
            &["Body Slam", "Earthquake", "Curse", "Self-Destruct"],
        );
        let skarm = make_set(
            "Skarmory",
            55,
            "Leftovers",
            &["Drill Peck", "Whirlwind", "Rest", "Toxic"],
        );

        let mut b01 =
            Battle::from_fixture(&dex, "1,2,3,4", &[lax.clone()], &[skarm.clone()]).unwrap();
        b01.choose(&dex, 0, "team 1").unwrap();
        b01.choose(&dex, 1, "team 1").unwrap();
        let (t01, cb01) = CompactBattle::from_battle(&b01, &dex);

        let mut b10 = Battle::from_fixture(&dex, "1,2,3,4", &[skarm], &[lax]).unwrap();
        b10.choose(&dex, 0, "team 1").unwrap();
        b10.choose(&dex, 1, "team 1").unwrap();
        let (t10, cb10) = CompactBattle::from_battle(&b10, &dex);

        let v01_tbl = with_solved_bellman(&cb01, &t01, 0, 0, 1, 0, |c| c.v_table);
        let v10_tbl = with_solved_bellman(&cb10, &t10, 0, 0, 1, 0, |c| c.v_table);

        // Check exact antisymmetry across the entire 16x16x3x3 Bellman table
        for s0 in 0..3 {
            for s1 in 0..3 {
                for b0 in 1..=B16 {
                    for b1 in 1..=B16 {
                        let v01 = v01_tbl[s0][s1][b0][b1];
                        let v10 = v10_tbl[s1][s0][b1][b0];
                        assert!(
                            (v01 + v10 - 1.0).abs() < 1e-5,
                            "antisymmetry violated at ({b0},{b1},{s0},{s1}): {v01} + {v10} != 1.0"
                        );
                    }
                }
            }
        }

        let r01 = toeplitz_coupled_race(&cb01, &t01, 0, 0, 1, 0);
        let r10 = toeplitz_coupled_race(&cb10, &t10, 0, 0, 1, 0);
        assert!((r01 + r10).abs() < 1e-5, "antisymmetry violated: {r01} vs {r10}");
        assert!(
            r01 < -0.25,
            "expected Skarmory Rest to be favored over Rest-less Snorlax, got {r01}"
        );
    }

    #[test]
    fn rollout_linear_pick_uses_bellman_policy_and_sleep_talk() {
        let dex = conformance::load_dex();
        let suicune = make_set(
            "Suicune",
            55,
            "Leftovers",
            &["Surf", "Ice Beam", "Rest", "Sleep Talk"],
        );
        let raikou = make_set(
            "Raikou",
            55,
            "Leftovers",
            &["Thunderbolt", "Crunch", "Rest", "Sleep Talk"],
        );
        let mut b = Battle::from_fixture(&dex, "1,2,3,4", &[suicune], &[raikou]).unwrap();
        b.choose(&dex, 0, "team 1").unwrap();
        b.choose(&dex, 1, "team 1").unwrap();
        let (table, mut cb) = CompactBattle::from_battle(&b, &dex);

        // At 100% HP awake, Bellman policy must NEVER choose Rest (`None`)
        let cs = [0u8, 1, 2, 3];
        assert_eq!(compact_rollout_linear_pick(&cb, &table, 0, &cs), None);

        // Put Suicune to sleep with status_counter == 2 (guaranteed asleep this turn) -> Sleep Talk (slot 3)
        cb.mons[0].set_status(Status::Slp as u8, 2);
        assert_eq!(compact_rollout_linear_pick(&cb, &table, 0, &cs), Some(3));

        // At status_counter == 1, Suicune wakes up this turn at 100% HP -> `None` (Attack)
        cb.mons[0].set_status(Status::Slp as u8, 1);
        assert_eq!(compact_rollout_linear_pick(&cb, &table, 0, &cs), None);
    }

    #[test]
    fn canonical_operator_cache_and_toxic_stack_invariants() {
        let dex = conformance::load_dex();
        let suicune_a = make_set(
            "Suicune",
            55,
            "Leftovers",
            &["Surf", "Roar", "Rest", "Toxic"],
        );
        // Permuted move order + different unused status move -> different `static_fp` (L1 miss),
        // but identical 1D Bellman operator (`Surf` + `Rest`, no Sleep Talk) -> L2 hit!
        let suicune_b = make_set(
            "Suicune",
            55,
            "Leftovers",
            &["Rest", "Reflect", "Surf", "Growl"],
        );
        let snorlax = make_set(
            "Snorlax",
            55,
            "Leftovers",
            &["Body Slam", "Earthquake", "Curse", "Rest"],
        );

        let mut ba =
            Battle::from_fixture(&dex, "1,2,3,4", &[suicune_a], &[snorlax.clone()]).unwrap();
        ba.choose(&dex, 0, "team 1").unwrap();
        ba.choose(&dex, 1, "team 1").unwrap();
        let (ta, mut cba) = CompactBattle::from_battle(&ba, &dex);

        let mut bb = Battle::from_fixture(&dex, "1,2,3,4", &[suicune_b], &[snorlax]).unwrap();
        bb.choose(&dex, 0, "team 1").unwrap();
        bb.choose(&dex, 1, "team 1").unwrap();
        let (tb, cbb) = CompactBattle::from_battle(&bb, &dex);

        assert_ne!(ta.mons[0].static_fp, tb.mons[0].static_fp);
        assert_ne!(
            l1_state_key(&cba, &ta, 0, 0, 1, 0),
            l1_state_key(&cbb, &tb, 0, 0, 1, 0)
        );

        let (key_a, hm0_a, hm1_a, vt_a) =
            with_solved_bellman(&cba, &ta, 0, 0, 1, 0, |c| (c.op_key, c.heal_mask0, c.heal_mask1, c.v_table));
        let (key_b, hm0_b, hm1_b) =
            with_solved_bellman(&cbb, &tb, 0, 0, 1, 0, |c| (c.op_key, c.heal_mask0, c.heal_mask1));
        assert_eq!(key_a, key_b);
        assert_eq!(hm0_a, hm0_b);
        assert_eq!(hm1_a, hm1_b);

        // Verify bit-exact 0.0-ULP equivalence against a fresh uncached solve
        let (ops, ms0, ms1, spd_ord) = Side1DOps::build_pair(&cba, &ta, 0, 0, 1, 0);
        let mut fresh = CachedBellman::empty();
        solve_bellman_into(ms0, ms1, spd_ord, &ops, &mut fresh);
        for s0 in 0..3 {
            for s1 in 0..3 {
                for b0 in 0..=B16 {
                    for b1 in 0..=B16 {
                        assert_eq!(
                            vt_a[s0][s1][b0][b1].to_bits(),
                            fresh.v_table[s0][s1][b0][b1].to_bits()
                        );
                    }
                }
            }
        }

        // Gen 2 Toxic-stacking flag (`0x10` in status_counter on Burn/Poison) must produce
        // distinct L1 keys and distinct L2 Bellman operators when counter >= 3.
        cba.mons[0].set_status(Status::Brn as u8, 4);
        let k_brn_plain = l1_state_key(&cba, &ta, 0, 0, 1, 0);
        let op_brn_plain = with_solved_bellman(&cba, &ta, 0, 0, 1, 0, |c| c.op_key);

        cba.mons[0].set_status(Status::Brn as u8, 0x10 | 4);
        let k_brn_tox = l1_state_key(&cba, &ta, 0, 0, 1, 0);
        let op_brn_tox = with_solved_bellman(&cba, &ta, 0, 0, 1, 0, |c| c.op_key);

        assert_ne!(k_brn_plain, k_brn_tox);
        assert_ne!(op_brn_plain, op_brn_tox);
    }

    #[test]
    fn toeplitz_coupled_race_uses_bench_slot_stats_and_moves_not_active() {
        let dex = conformance::load_dex();
        let unown = make_set("Unown", 50, "No Item", &["Hidden Power"]);
        let marowak = make_set(
            "Marowak",
            55,
            "Thick Club",
            &["Earthquake", "Rock Slide", "Fire Blast", "Swords Dance"],
        );
        let jolteon = make_set(
            "Jolteon",
            50,
            "Magnet",
            &["Thunderbolt", "Hidden Power", "Agility", "Baton Pass"],
        );

        let mut b = Battle::from_fixture(
            &dex,
            "1,2,3,4",
            &[unown, marowak],
            &[jolteon],
        )
        .unwrap();
        b.choose(&dex, 0, "team 1,2").unwrap();
        b.choose(&dex, 1, "team 1").unwrap();
        let (table, cb) = CompactBattle::from_battle(&b, &dex);

        // Active is slot 0 (Unown), Bench is slot 1 (Marowak).
        // Race for bench Marowak (side 0, slot 1) vs Jolteon (side 1, slot 0):
        // Marowak is immune to Thunderbolt and OHKOs Jolteon with Earthquake, so Marowak must strongly win!
        let r_unown = toeplitz_coupled_race(&cb, &table, 0, 0, 1, 0);
        let r_marowak_bench = toeplitz_coupled_race(&cb, &table, 0, 1, 1, 0);
        assert!(
            r_unown < 0.0,
            "Active Unown (slot 0) loses to Jolteon, got {r_unown}"
        );
        assert!(
            r_marowak_bench > 0.70,
            "Bench Marowak (slot 1) must use slot 1's own stats/moves/types and crush Jolteon, got {r_marowak_bench}"
        );
        // Verify exact side-swap antisymmetry: race(0,1 vs 1,0) == -race(1,0 vs 0,1).
        let r_swapped = toeplitz_coupled_race(&cb, &table, 1, 0, 0, 1);
        assert!(
            (r_marowak_bench + r_swapped).abs() < 1e-12,
            "Antisymmetry violated: {r_marowak_bench} vs {r_swapped}"
        );
    }
}
