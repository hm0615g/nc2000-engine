//! Omniscient & Best-Response Ceiling Audit (`omniscient_audit`):
//!
//! Quantifies the exact suboptimality gap of the shipped bot (`BlindAgent` @ 400 iterations
//! playing the 2-shell 5-variant Nash equilibrium `data/kodoku-r3/conclusion-equilibrium-deme.json`)
//! against an escalating 7-tier hierarchy of SuperCompute, Rules-Compliant Bayesian Best-Response,
//! Clairvoyant Set-Reader, Clairvoyant 3v3-Pick-Reader, Omniscient God BR (True State + True Live
//! Policy Oracle + Certified `BoundSolver` Endgame), and Psychic Simultaneous-Move Reader (`Jan-Ken` God).
//!
//! Also runs a Certified Exact Endgame Audit (`BoundSolver`) on `1v1` last-mon endgame states
//! reached by the equilibrium bot to measure exact minimax policy loss intervals `[lo, hi]`.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use conformance::fixture::repo_root;
use conformance::load_dex;
use nc2000_bot::belief::{Belief, FallbackPolicy};
use nc2000_bot::blind::BlindSearch;
use nc2000_bot::bounds::{BoundConfig, BoundSolver};
use nc2000_bot::observe::Observer;
use nc2000_bot::preview::{action_index, preview_actions, MetaPool, MetaTeam};
use nc2000_bot::smmcts::{RmConfig, SelRule, SkuctSearch};
use nc2000_bot::teamgen::{to_sets, TeamGen};
use nc2000_bot::{Agent, BlindAgent, ProtocolAgent, SplitMix64};
use nc2000_bot::player::PlayerChannel;
use nc2000_bot::player_state::{LocalPlayer, ObservedPosition};
use nc2000_engine::battle::{Outcome, PokemonSet, SearchChoice};
use nc2000_engine::dex::{toid, Dex};
use nc2000_engine::state::{Battle, RequestState};
use serde_json::{json, Value};

type PreviewSig = Vec<(String, i64, String, bool)>;

fn preview_sig_sets(sets: &[PokemonSet]) -> PreviewSig {
    let mut v: PreviewSig = sets
        .iter()
        .map(|s| {
            let sp = toid(&s.species);
            let lv = s.level as i64;
            let g = s.gender.as_deref().unwrap_or("").to_uppercase();
            let item = !toid(&s.item).is_empty();
            (sp, lv, g, item)
        })
        .collect();
    v.sort();
    v
}

fn legal_triples_sets(sets: &[PokemonSet]) -> Vec<[usize; 3]> {
    let mut out = Vec::with_capacity(60);
    if sets.len() < 3 {
        return out;
    }
    let n = sets.len().min(6);
    for i in 0..n {
        for j in (i + 1)..n {
            for k in (j + 1)..n {
                let lsum = (sets[i].level as u16) + (sets[j].level as u16) + (sets[k].level as u16);
                if lsum <= 155 {
                    out.push([i, j, k]);
                    out.push([j, i, k]);
                    out.push([k, i, j]);
                }
            }
        }
    }
    out
}

#[derive(Clone)]
struct CompiledRoster6 {
    sets: Vec<PokemonSet>,
    legal: Vec<[usize; 3]>,
    base_table: nc2000_engine::compact::MatchTable,
    specs: [nc2000_engine::compact::StaticMonSpec; 6],
    moves: [[nc2000_engine::compact::CompactMoveSpec; 4]; 6],
    mons: [nc2000_engine::compact::CompactMon; 6],
}

impl CompiledRoster6 {
    fn compile(dex: &Dex, sets: &[PokemonSet]) -> Self {
        assert_eq!(sets.len(), 6);
        let legal = legal_triples_sets(sets);
        let ensure_det_gender = |s: &mut PokemonSet| {
            if s.gender.is_none() {
                if let Some(sid) = dex.species.id(&s.species) {
                    let sp = dex.species.get(sid);
                    s.gender = Some(match sp.gender.as_deref() {
                        Some("N") => "N".to_string(),
                        Some("F") => "F".to_string(),
                        _ => "M".to_string(),
                    });
                }
            }
        };
        let mut clean_sets: Vec<PokemonSet> = sets.to_vec();
        for s in &mut clean_sets {
            ensure_det_gender(s);
        }
        let mut dummy50 = clean_sets[0].clone();
        dummy50.level = 50;
        let mut specs = [nc2000_engine::compact::StaticMonSpec::default(); 6];
        let mut moves = [[nc2000_engine::compact::CompactMoveSpec::default(); 4]; 6];
        let mut mons = [nc2000_engine::compact::CompactMon::default(); 6];
        let mut base_table_opt = None;
        for k in 0..3 {
            let i0 = 2 * k;
            let i1 = 2 * k + 1;
            let s0 = [clean_sets[i0].clone(), dummy50.clone(), dummy50.clone()];
            let s1 = [clean_sets[i1].clone(), dummy50.clone(), dummy50.clone()];
            let mut b = Battle::from_fixture(dex, "1,2,3,4", &s0, &s1).unwrap();
            b.set_log_enabled(false);
            b.apply_choices(
                dex,
                [
                    Some(SearchChoice::Team([1, 2, 3])),
                    Some(SearchChoice::Team([1, 2, 3])),
                ],
            )
            .unwrap();
            let (tab, cb) = nc2000_engine::compact::CompactBattle::from_battle(&b, dex);
            specs[i0] = tab.mons[0];
            moves[i0] = tab.move_specs[0];
            mons[i0] = cb.mons[0];
            specs[i1] = tab.mons[3];
            moves[i1] = tab.move_specs[3];
            mons[i1] = cb.mons[3];
            if base_table_opt.is_none() {
                base_table_opt = Some(tab);
            }
        }
        Self {
            sets: clean_sets,
            legal,
            specs,
            moves,
            mons,
            base_table: base_table_opt.unwrap(),
        }
    }
}

#[inline]
fn assemble_3v3_compact(
    r_a: &CompiledRoster6,
    ta: [usize; 3],
    r_b: &CompiledRoster6,
    tb: [usize; 3],
    prng_seed: u64,
) -> (
    nc2000_engine::compact::MatchTable,
    nc2000_engine::compact::CompactBattle,
) {
    use nc2000_engine::compact::{CompactBattle, CompactSide, MatchTable};
    let static_mons = [
        r_a.specs[ta[0]],
        r_a.specs[ta[1]],
        r_a.specs[ta[2]],
        r_b.specs[tb[0]],
        r_b.specs[tb[1]],
        r_b.specs[tb[2]],
    ];
    let move_specs = [
        r_a.moves[ta[0]],
        r_a.moves[ta[1]],
        r_a.moves[ta[2]],
        r_b.moves[tb[0]],
        r_b.moves[tb[1]],
        r_b.moves[tb[2]],
    ];
    let mut fp = 0xcbf29ce484222325u64;
    for m in &static_mons {
        fp = fp
            .wrapping_add(m.static_fp)
            .wrapping_mul(0xf1357aea2e62a9c5);
    }
    let table = MatchTable {
        mons: static_mons,
        move_specs,
        match_fingerprint: fp,
        ..r_a.base_table
    };
    let side_a = CompactSide {
        active_idx: 0,
        party_roster: [ta[0] as u8, ta[1] as u8, ta[2] as u8],
        alive_mask: 0b0111,
        reflect_turns: 0,
        lightscreen_turns: 0,
        safeguard_turns: 0,
        side_flags: 0,
        last_move_selfko: u8::MAX,
        request: 1,
        pad: (fp >> 16) as u8,
    };
    let side_b = CompactSide {
        active_idx: 0,
        party_roster: [tb[0] as u8, tb[1] as u8, tb[2] as u8],
        alive_mask: 0b0111,
        reflect_turns: 0,
        lightscreen_turns: 0,
        safeguard_turns: 0,
        side_flags: 0,
        last_move_selfko: u8::MAX,
        request: 1,
        pad: (fp >> 24) as u8,
    };
    let cb = CompactBattle {
        mons: [
            r_a.mons[ta[0]],
            r_a.mons[ta[1]],
            r_a.mons[ta[2]],
            r_b.mons[tb[0]],
            r_b.mons[tb[1]],
            r_b.mons[tb[2]],
        ],
        sides: [side_a, side_b],
        turn: 1,
        weather_kind: 0,
        weather_turns: 0,
        quick_claw_roll: (((prng_seed >> 32) & 0xFF) < 60) as u8,
        outcome: 0,
        speed_order: 0,
        clause_flags: 5,
        hurt_damage_this_turn: [0, 0],
        hurt_cat_this_turn: [0, 0],
        match_fp_lo: fp as u16,
        prng_seed,
    };
    (table, cb)
}

#[inline]
fn eval_crn_cell_pair_compact(
    r_a: &CompiledRoster6,
    ia: usize,
    ta: [usize; 3],
    r_b: &CompiledRoster6,
    ib: usize,
    tb: [usize; 3],
    s: u32,
    base_seed: u64,
    sig_a_le_b: bool,
    playout: &nc2000_bot::Playout,
    max_turns: u16,
    rules: nc2000_bot::mcts::RolloutRules,
) -> f64 {
    let (c1, c2) = if sig_a_le_b { (ia, ib) } else { (ib, ia) };
    let cell_seed = base_seed
        ^ ((s as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15))
        ^ (((c1 * 64 + c2) as u64).wrapping_mul(0xBF58_476D_1CE4_E5B9));
    let prng = cell_seed | 1;
    let (rng_ab_seed, rng_ba_seed) = if sig_a_le_b {
        (
            cell_seed ^ 0xA5A5_5A5A_F0F0_0F0F,
            cell_seed ^ 0x5A5A_A5A5_0F0F_F0F0,
        )
    } else {
        (
            cell_seed ^ 0x5A5A_A5A5_0F0F_F0F0,
            cell_seed ^ 0xA5A5_5A5A_F0F0_0F0F,
        )
    };
    let (tab_ab, mut cb_ab) = assemble_3v3_compact(r_a, ta, r_b, tb, prng);
    let mut rng_ab = SplitMix64::new(rng_ab_seed);
    let v_ab = nc2000_bot::mcts::compact_playout_value(
        &mut cb_ab, &tab_ab, playout, max_turns, &mut rng_ab, rules,
    );

    let (tab_ba, mut cb_ba) = assemble_3v3_compact(r_b, tb, r_a, ta, prng);
    let mut rng_ba = SplitMix64::new(rng_ba_seed);
    let v_ba = 1.0
        - nc2000_bot::mcts::compact_playout_value(
            &mut cb_ba, &tab_ba, playout, max_turns, &mut rng_ba, rules,
        );

    0.5 * (v_ab + v_ba)
}

fn eval_full_preview_matrix_compact(
    r_a: &CompiledRoster6,
    r_b: &CompiledRoster6,
    n_seeds: u32,
    max_turns: u16,
    base_seed: u64,
) -> Vec<f64> {
    let ka = r_a.legal.len();
    let kb = r_b.legal.len();
    let mut mat = vec![0.5f64; ka * kb];
    if ka == 0 || kb == 0 {
        return mat;
    }
    let ns = n_seeds.max(1);
    let sig_a_le_b = preview_sig_sets(&r_a.sets) <= preview_sig_sets(&r_b.sets);
    let inv_ns = 1.0 / (ns as f64);
    let playout = nc2000_bot::Playout::heavy();
    let rules = nc2000_bot::mcts::RolloutRules::default();
    for (ia, &ta) in r_a.legal.iter().enumerate() {
        for (ib, &tb) in r_b.legal.iter().enumerate() {
            let mut sum = 0.0f64;
            for s in 0..ns {
                sum += eval_crn_cell_pair_compact(
                    r_a, ia, ta, r_b, ib, tb, s, base_seed, sig_a_le_b, &playout, max_turns, rules,
                );
            }
            mat[ia * kb + ib] = sum * inv_ns;
        }
    }
    mat
}

// Corrected experiments initialize every selected lineup through the engine;
// copied compact templates can carry entry effects on unseen bench members.
fn native_preview_matrix(dex:&Dex,a:&CompiledRoster6,b:&CompiledRoster6,ns:u32,turns:u16,seed:u64)->Vec<f64> {
    let mut matrix=vec![0.;a.legal.len()*b.legal.len()];
    let template=Battle::from_fixture(dex,"1,2,3,4",&a.sets,&b.sets).unwrap();
    for (ia,ta) in a.legal.iter().enumerate() {for (ib,tb) in b.legal.iter().enumerate() {
        let mut value=0.;for k in 0..ns {
            let mut root=template.clone();root.set_log_enabled(false);
            let pair_seed=seed ^ u64::from(k).wrapping_mul(0x9E3779B97F4A7C15) ^ ((ia*64+ib) as u64).wrapping_mul(0xBF58476D1CE4E5B9);
            root.reseed(pair_seed);
            root.apply_choices(dex,[Some(SearchChoice::Team(ta.map(|i|i as u8+1))),Some(SearchChoice::Team(tb.map(|i|i as u8+1)))]).unwrap();
            let (table,mut state)=nc2000_engine::compact::CompactBattle::from_battle(&root,dex);
            value+=nc2000_bot::mcts::compact_playout_value(&mut state,&table,&nc2000_bot::Playout::heavy(),turns,
                &mut SplitMix64::new(pair_seed ^ 0xA5A55A5AF0F00F0F),nc2000_bot::mcts::RolloutRules::default());
        }matrix[ia*b.legal.len()+ib]=value/ns as f64;
    }}matrix
}

fn model_search(position:Option<&ObservedPosition>,model:&Battle,dex:&Dex,cfg:RmConfig,side:usize,seed:u64)->BlindSearch {
    match position { Some(p)=>BlindSearch::from_observation(p,dex,cfg,side,seed),None=>BlindSearch::new(model,dex,cfg,side,seed) }
}

fn solve_bayesian_shell_preview_nash(
    mats_km: &[&[f64]],
    ka: usize,
    kb: usize,
    alpha: &[f64],
    beta: &[f64],
    sweeps: usize,
) -> (Vec<Vec<f64>>, Vec<Vec<f64>>) {
    let n_u = alpha.len();
    let n_v = beta.len();
    let mut a_norm = alpha.to_vec();
    let a_sum: f64 = a_norm.iter().sum::<f64>().max(1e-12);
    for x in &mut a_norm {
        *x /= a_sum;
    }
    let mut b_norm = beta.to_vec();
    let b_sum: f64 = b_norm.iter().sum::<f64>().max(1e-12);
    for x in &mut b_norm {
        *x /= b_sum;
    }

    let mut reg_p = vec![vec![0.0f64; ka]; n_u];
    let mut reg_q = vec![vec![0.0f64; kb]; n_v];
    let mut p_cum = vec![vec![0.0f64; ka]; n_u];
    let mut q_cum = vec![vec![0.0f64; kb]; n_v];
    let mut p_cur = vec![vec![1.0 / (ka as f64); ka]; n_u];
    let mut q_cur = vec![vec![1.0 / (kb as f64); kb]; n_v];
    let unif_p = 1.0 / (ka as f64);
    let unif_q = 1.0 / (kb as f64);

    let n_iters = sweeps.max(400);
    for t in 1..=n_iters {
        for k in 0..n_u {
            let sp: f64 = reg_p[k].iter().sum();
            if sp > 1e-12 {
                for ia in 0..ka {
                    p_cur[k][ia] = reg_p[k][ia] / sp;
                }
            } else {
                p_cur[k].fill(unif_p);
            }
        }
        for m in 0..n_v {
            let sq: f64 = reg_q[m].iter().sum();
            if sq > 1e-12 {
                for ib in 0..kb {
                    q_cur[m][ib] = reg_q[m][ib] / sq;
                }
            } else {
                q_cur[m].fill(unif_q);
            }
        }
        for k in 0..n_u {
            if a_norm[k] <= 1e-12 {
                continue;
            }
            let mut u_p = vec![0.0f64; ka];
            let mut v_p = 0.0f64;
            for ia in 0..ka {
                let mut val_ia = 0.0f64;
                for m in 0..n_v {
                    let mat = mats_km[k * n_v + m];
                    let row_slice = &mat[ia * kb..(ia + 1) * kb];
                    let qm = &q_cur[m];
                    let mut dot = 0.0f64;
                    for ib in 0..kb {
                        dot += row_slice[ib] * qm[ib];
                    }
                    val_ia += b_norm[m] * dot;
                }
                u_p[ia] = val_ia;
                v_p += p_cur[k][ia] * val_ia;
            }
            for ia in 0..ka {
                reg_p[k][ia] = (reg_p[k][ia] + (u_p[ia] - v_p)).max(0.0);
            }
        }
        for m in 0..n_v {
            if b_norm[m] <= 1e-12 {
                continue;
            }
            let mut u_q = vec![0.0f64; kb];
            let mut v_q = 0.0f64;
            for ib in 0..kb {
                let mut loss_ib = 0.0f64;
                for k in 0..n_u {
                    let mat = mats_km[k * n_v + m];
                    let pk = &p_cur[k];
                    let mut dot = 0.0f64;
                    for ia in 0..ka {
                        dot += pk[ia] * mat[ia * kb + ib];
                    }
                    loss_ib += a_norm[k] * dot;
                }
                let val_ib = 1.0 - loss_ib;
                u_q[ib] = val_ib;
                v_q += q_cur[m][ib] * val_ib;
            }
            for ib in 0..kb {
                reg_q[m][ib] = (reg_q[m][ib] + (u_q[ib] - v_q)).max(0.0);
            }
        }
        let w_t = t as f64;
        for k in 0..n_u {
            for ia in 0..ka {
                p_cum[k][ia] += w_t * p_cur[k][ia];
            }
        }
        for m in 0..n_v {
            for ib in 0..kb {
                q_cum[m][ib] += w_t * q_cur[m][ib];
            }
        }
    }
    for k in 0..n_u {
        let s: f64 = p_cum[k].iter().sum();
        if s > 1e-12 {
            for x in &mut p_cum[k] {
                *x /= s;
            }
        } else {
            p_cum[k].fill(unif_p);
        }
    }
    for m in 0..n_v {
        let s: f64 = q_cum[m].iter().sum();
        if s > 1e-12 {
            for x in &mut q_cum[m] {
                *x /= s;
            }
        } else {
            q_cum[m].fill(unif_q);
        }
    }
    (p_cum, q_cum)
}

fn expand_to_canonical_60(legal: &[[usize; 3]], strat: &[f64]) -> Vec<f64> {
    let actions = preview_actions();
    let mut out = vec![0.0f64; 60];
    for (t0, &p) in legal.iter().zip(strat.iter()) {
        if p <= 0.0 {
            continue;
        }
        let triple_1based = [t0[0] as u8 + 1, t0[1] as u8 + 1, t0[2] as u8 + 1];
        if let Some(idx60) = action_index(&actions, triple_1based) {
            out[idx60] += p;
        }
    }
    let sum: f64 = out.iter().sum();
    if sum > 0.0 {
        for x in &mut out {
            *x /= sum;
        }
    }
    out
}

fn one_hot_canonical_60(choice: SearchChoice) -> Option<Vec<f64>> {
    let SearchChoice::Team(t) = choice else {
        return None;
    };
    let actions = preview_actions();
    let idx60 = action_index(&actions, t)?;
    let mut v = vec![0.0f64; 60];
    v[idx60] = 1.0;
    Some(v)
}

#[derive(Clone)]
struct PrecomputedMatchupOracle {
    mat_red_vs_blue: Vec<f64>,
    ka: usize,
    kb: usize,
    red_legal: Vec<[usize; 3]>,
    blue_legal: Vec<[usize; 3]>,
    blue_own_pol60: Vec<f64>,
    blue_opp_pol60: Vec<f64>,
    red_nash_pol60: Vec<f64>,
    red_opp_shell_pol60: Vec<f64>,
    red_bayesian_br_pol60: Vec<f64>,
    red_sheet_clairvoyant_pol60: Vec<f64>,
}

impl PrecomputedMatchupOracle {
    fn pick_clairvoyant_counter(
        &self,
        blue_choice: SearchChoice,
        legal_red_choices: &[SearchChoice],
    ) -> SearchChoice {
        if let SearchChoice::Team(t_b) = blue_choice {
            let b0 = (t_b[0] as usize).saturating_sub(1);
            let b1 = (t_b[1] as usize).saturating_sub(1);
            let b2 = (t_b[2] as usize).saturating_sub(1);
            if let Some(ib) = self.blue_legal.iter().position(|&[l, x, y]| {
                l == b0 && ((x == b1 && y == b2) || (x == b2 && y == b1))
            }) {
                let mut best_ia = 0usize;
                let mut best_val = f64::NEG_INFINITY;
                for ia in 0..self.ka {
                    let v = self.mat_red_vs_blue[ia * self.kb + ib];
                    if v > best_val {
                        best_val = v;
                        best_ia = ia;
                    }
                }
                let [r0, r1, r2] = self.red_legal[best_ia];
                let target = SearchChoice::Team([r0 as u8 + 1, r1 as u8 + 1, r2 as u8 + 1]);
                if legal_red_choices.contains(&target) {
                    return target;
                }
            }
        }
        legal_red_choices[0]
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AttackerTier {
    T0BaselineBlind400,
    T1aRootPruningOnly,
    T1bAdaptiveMixOnly,
    T1cBlind400AdaptiveMix,
    T1SuperCompute3200Mix,
    T2aRulesSuperSearch3x,
    T2RulesBayesianOracleBr,
    T3ClairvoyantOpenSheet,
    T4Clairvoyant3v3FullInfo,
    T5OmniscientGodBr,
    T6PsychicMoveReaderGod,
}

impl AttackerTier {
    fn id(self) -> &'static str {
        match self {
            Self::T0BaselineBlind400 => "T0_Baseline_Blind",
            Self::T1aRootPruningOnly => "T1a_RootPruning_Only",
            Self::T1bAdaptiveMixOnly => "T1b_AdaptiveMix_Only",
            Self::T1cBlind400AdaptiveMix => "T1c_AdaptiveMix_And_Pruning",
            Self::T1SuperCompute3200Mix => "T1d_SuperCompute_8x_Mix",
            Self::T2aRulesSuperSearch3x => "T2a_Rules_SuperSearch_3x",
            Self::T2RulesBayesianOracleBr => "T2_Rules_Bayesian_Oracle_BR",
            Self::T3ClairvoyantOpenSheet => "T3_Clairvoyant_OpenSheet",
            Self::T4Clairvoyant3v3FullInfo => "T4_Clairvoyant_3v3_FullInfo",
            Self::T5OmniscientGodBr => "T5_Omniscient_God_BR",
            Self::T6PsychicMoveReaderGod => "T6_Psychic_MoveReader_God",
        }
    }

    fn description(self) -> &'static str {
        match self {
            Self::T0BaselineBlind400 => "Shipped Bot (BlindAgent 1x iters, c=0.4 + Bayesian Shell-Nash Preview)",
            Self::T1aRootPruningOnly => "Rules-Compliant 1x Compute + Root Pruning Only (prune_dominated before UCB)",
            Self::T1bAdaptiveMixOnly => "Rules-Compliant 1x Compute + Bayesian Adaptive Mixture Only (2/5 probe reserve)",
            Self::T1cBlind400AdaptiveMix => "Rules-Compliant 1x Compute + Bayesian Adaptive Mixture (1/5 reserve) + Root Pruning",
            Self::T1SuperCompute3200Mix => "Rules-Compliant 8x Compute + Root Mixing + Pruning",
            Self::T2aRulesSuperSearch3x => "Rules-Compliant 3x Pure Search Oracle (3x Battle Iters + Bayesian BR Preview)",
            Self::T2RulesBayesianOracleBr => "Rules-Compliant Coupled Bayesian Policy-Oracle BR (Public Info Only + Determinized 1x Policy Oracle + BoundSolver + Coupled UCB-BR)",
            Self::T3ClairvoyantOpenSheet => "Cheating Set-Sheet Reader (Knows Exact Variant/Moves/Items + Variant-BR Preview + OpenAgent)",
            Self::T4Clairvoyant3v3FullInfo => "Cheating 3v3-Pick & Lead Reader (Counter-Picks Blue's Exact Turn-0 3v3 Selection + Pinned 3-Mon Bench)",
            Self::T5OmniscientGodBr => "Omniscient God BR (Full State + Live-Policy Oracle + Certified BoundSolver Endgame + UCB-BR Search)",
            Self::T6PsychicMoveReaderGod => "Psychic Move-Reader God (Reads Blue's Exact Realized Action on Every Simultaneous Turn + BoundSolver)",
        }
    }

    fn rules_compliant(self) -> bool {
        matches!(
            self,
            Self::T0BaselineBlind400
                | Self::T1aRootPruningOnly
                | Self::T1bAdaptiveMixOnly
                | Self::T1cBlind400AdaptiveMix
                | Self::T1SuperCompute3200Mix
                | Self::T2aRulesSuperSearch3x
                | Self::T2RulesBayesianOracleBr
        )
    }
}

fn count_alive_mons(battle: &Battle, side: usize) -> usize {
    battle.sides[side]
        .party
        .iter()
        .filter(|&&slot| {
            let p = &battle.sides[side].roster[slot as usize];
            p.hp > 0 && !p.fainted
        })
        .count()
}

/// Run UCB1-guided Best-Response search on `bs` against a distribution of opponent replies `replies`:
/// 1. First runs `warmup_iters` of standard `step_one` to populate continuation subtrees & `EndgameRootStats`.
/// 2. Then runs `br_iters` of UCB1-selected `own_a` forced against `replies[t % replies.len()]`.
/// 3. Selects the action with maximum forced-probe mean reward among actions with sufficient visits.
fn run_ucb_best_response_search(
    bs: &mut BlindSearch,
    dex: &Dex,
    bel: &Belief,
    obs: &Observer,
    side: usize,
    replies: &[SearchChoice],
    warmup_iters: u32,
    br_iters: u32,
) -> SearchChoice {
    let actions = bs.actions().to_vec();
    let n_own = actions.len();
    if n_own == 1 {
        return actions[0];
    }
    bs.prune_dominated();
    let dom = bs.dominated().to_vec();
    let any_safe = dom.iter().any(|&d| !d);
    let eligible: Vec<usize> = (0..n_own).filter(|&i| !any_safe || !dom[i]).collect();
    if eligible.len() == 1 {
        return actions[eligible[0]];
    }

    for _ in 0..warmup_iters {
        bs.step_one(dex, bel, obs);
    }
    if replies.is_empty() || br_iters == 0 {
        return bs.best().unwrap_or(actions[eligible[0]]);
    }

    // Track dedicated BR statistics `(br_n[a], br_w[a])` against `replies`
    let mut br_n = vec![0u32; n_own];
    let mut br_w = vec![0.0f64; n_own];

    for it in 0..(br_iters as usize) {
        // Ensure every eligible action is tried at least twice before UCB1
        let own_a = if it < eligible.len() * 2 {
            eligible[it % eligible.len()]
        } else {
            let tot = (it as f64).ln();
            let mut best_a = eligible[0];
            let mut best_ucb = f64::NEG_INFINITY;
            for &a in &eligible {
                let na = (br_n[a].max(1)) as f64;
                let mean = br_w[a] / na;
                let ucb = mean + 0.45 * (tot / na).sqrt();
                if ucb > best_ucb {
                    best_ucb = ucb;
                    best_a = a;
                }
            }
            best_a
        };
        let rep = replies[it % replies.len()];
        let r_side0 = bs.step_forced_joint(dex, bel, obs, own_a, rep);
        let r_my = if side == 0 { r_side0 } else { 1.0 - r_side0 };
        br_n[own_a] += 1;
        br_w[own_a] += r_my;
    }

    // Choose the eligible action with highest empirical mean `br_w[a] / br_n[a]` among well-visited contenders (`br_n[a] >= max_n / 3`)
    let max_n = eligible.iter().map(|&a| br_n[a]).max().unwrap_or(1);
    let min_n = (max_n / 3).max(2);
    let best_idx = eligible
        .into_iter()
        .filter(|&a| br_n[a] >= min_n)
        .max_by(|&a, &b| {
            let ma = br_w[a] / (br_n[a].max(1) as f64);
            let mb = br_w[b] / (br_n[b].max(1) as f64);
            ma.total_cmp(&mb).then_with(|| br_n[a].cmp(&br_n[b]))
        })
        .unwrap_or(0);
    actions[best_idx]
}

#[allow(clippy::too_many_arguments)]
fn play_omniscient_duel_game(
    dex: &Dex,
    battle_seed: &str,
    blue_sets: &[PokemonSet],
    red_sets: &[PokemonSet],
    blue_is_p1: bool,
    blue_seed: u64,
    red_seed: u64,
    blue_prior: Arc<MetaPool>,
    red_prior: Arc<MetaPool>,
    oracle: &PrecomputedMatchupOracle,
    tier: AttackerTier,
    base_iters: u32,
    same_budget: bool,
    k_oracle: usize,
    max_turns: u16,
    corrected: bool,
    compiled: &Value,
) -> (f64, u16, usize, nc2000_bot::agent::RootSearchStats) {
    let (side_blue, side_red) = if blue_is_p1 { (0, 1) } else { (1, 0) };
    let (t1, t2) = if blue_is_p1 {
        (blue_sets, red_sets)
    } else {
        (red_sets, blue_sets)
    };
    let mut battle = Battle::from_fixture(dex, battle_seed, t1, t2).unwrap();
    battle.set_log_enabled(true);

    let base_cfg = RmConfig {
        iterations: base_iters,
        rule: SelRule::Ucb,
        c: 0.4,
        ..Default::default()
    };

    let mut blue_agent = BlindAgent::new(base_cfg.clone(), blue_prior.clone(), None, blue_seed)
        .with_preview_iterations(base_iters.max(240))
        .with_own_preview_policy(oracle.blue_own_pol60.clone());
    if !corrected {blue_agent=blue_agent.with_opponent_pick_policy(oracle.blue_opp_pol60.clone(),0.10);}

    // Match the actual live worker: protocol reconstruction, unchanged prior,
    // u32 search seed, compiled preview selected by seed modulo 64.
    let mut blue_protocol = corrected.then(|| {
        let mut agent=ProtocolAgent::new(dex,side_blue,(*blue_prior).clone(),base_cfg.clone(),blue_seed as u32 as u64);
        agent.set_own_team(blue_sets.to_vec());agent
    });
    let mut blue_channel=PlayerChannel::new(side_blue);
    let mut red_rng = SplitMix64::new(red_seed);
    let mut red_std_agent: Option<BlindAgent> = match tier {
        AttackerTier::T0BaselineBlind400 => Some(
            BlindAgent::new(base_cfg.clone(), red_prior.clone(), None, red_seed)
                .with_preview_iterations(base_iters.max(240))
                .with_own_preview_policy(oracle.red_nash_pol60.clone())
                .with_opponent_pick_policy(oracle.red_opp_shell_pol60.clone(), 0.10),
        ),
        AttackerTier::T1aRootPruningOnly => Some(
            BlindAgent::new(base_cfg.clone(), red_prior.clone(), None, red_seed)
                .with_preview_iterations(base_iters.max(240))
                .with_own_preview_policy(oracle.red_nash_pol60.clone())
                .with_opponent_pick_policy(oracle.red_opp_shell_pol60.clone(), 0.10)
                .with_root_pruning(),
        ),
        AttackerTier::T1bAdaptiveMixOnly => Some(
            BlindAgent::new(base_cfg.clone(), red_prior.clone(), None, red_seed)
                .with_preview_iterations(base_iters.max(240))
                .with_own_preview_policy(oracle.red_nash_pol60.clone())
                .with_opponent_pick_policy(oracle.red_opp_shell_pol60.clone(), 0.10)
                .with_adaptive_mixture((base_iters * 2 / 5).max(40)),
        ),
        AttackerTier::T1cBlind400AdaptiveMix => Some(
            BlindAgent::new(base_cfg.clone(), red_prior.clone(), None, red_seed)
                .with_preview_iterations(base_iters.max(240))
                .with_own_preview_policy(oracle.red_nash_pol60.clone())
                .with_opponent_pick_policy(oracle.red_opp_shell_pol60.clone(), 0.10)
                .with_adaptive_mixture((base_iters / 5).max(40))
                .with_root_pruning(),
        ),
        AttackerTier::T1SuperCompute3200Mix => {
            let super_iters = base_iters * 8;
            let super_cfg = RmConfig {
                iterations: super_iters,
                rule: SelRule::Ucb,
                c: 0.4,
                ..Default::default()
            };
            Some(
                BlindAgent::new(super_cfg, red_prior.clone(), None, red_seed)
                    .with_preview_iterations(super_iters)
                    .with_own_preview_policy(oracle.red_nash_pol60.clone())
                    .with_opponent_pick_policy(oracle.red_opp_shell_pol60.clone(), 0.10)
                    .with_adaptive_mixture((super_iters / 8).max(64))
                    .with_root_pruning(),
            )
        }
        AttackerTier::T2aRulesSuperSearch3x => {
            let super_iters = base_iters * 3;
            let super_cfg = RmConfig {
                iterations: super_iters,
                rule: SelRule::Ucb,
                c: 0.4,
                ..Default::default()
            };
            Some(
                BlindAgent::new(super_cfg, red_prior.clone(), None, red_seed)
                    .with_preview_iterations(super_iters)
                    .with_own_preview_policy(oracle.red_bayesian_br_pol60.clone())
                    .with_opponent_pick_policy(oracle.red_opp_shell_pol60.clone(), 0.10)
                    .with_root_pruning(),
            )
        }
        _ => None,
    };

    let mut red_local: Option<LocalPlayer> = None;
    let mut blue_public_local: Option<LocalPlayer> = None;
    let mut red_obs: Option<Observer> = None;
    let mut red_belief: Option<Belief> = None;
    // Publicly observable shadow tracker of what Blue knows about Red (100% public info: Red's own preview + Red's public moves/items)
    let mut blue_public_obs: Option<Observer> = None;
    let mut blue_public_bel: Option<Belief> = None;
    let mut blue_pick_one_hot60: Option<Vec<f64>> = None;
    let mut solver_interventions = 0usize;

    loop {
        if let Some(outcome) = battle.outcome() {
            let p1_pts = match outcome {
                Outcome::P1Win => 1.0,
                Outcome::P2Win => 0.0,
                Outcome::Tie => 0.5,
            };
            let blue_pts = if blue_is_p1 { p1_pts } else { 1.0 - p1_pts };
            let r_stats = red_std_agent
                .as_ref()
                .and_then(|a| a.root_search_stats())
                .unwrap_or_default();
            return (blue_pts, battle.turn, solver_interventions, r_stats);
        }
        if battle.turn >= max_turns && !matches!(battle.request_state, RequestState::TeamPreview) {
            let r_stats = red_std_agent
                .as_ref()
                .and_then(|a| a.root_search_stats())
                .unwrap_or_default();
            return (0.5, battle.turn, solver_interventions, r_stats);
        }

        let choices_blue = battle.legal_choices(dex, side_blue);
        let choices_red = battle.legal_choices(dex, side_red);

        // 1. Blue always chooses its action via its standard `BlindAgent`
        let blue_choice = if choices_blue.is_empty() {
            None
        } else {
            Some(if let Some(agent)=blue_protocol.as_mut() {
                let frame=blue_channel.frame(&mut battle,dex).unwrap();
                for line in frame.lines {agent.push_line(dex,&line);}
                assert!(agent.on_request(dex,&frame.request.to_string()).unwrap());
                let input=if matches!(battle.request_state,RequestState::TeamPreview) {
                    let key=nc2000_bot::compiled_preview::key(agent.battle().unwrap(),side_blue).unwrap();
                    let choices=compiled["policies"][key].as_array().unwrap();
                    choices[blue_seed as u32 as usize % choices.len()].as_str().unwrap().to_string()
                } else {agent.step(dex,base_iters).unwrap();agent.best(dex).unwrap()};
                nc2000_bot::fork::find_choice(dex,&choices_blue,&input).expect("product emitted illegal action")
            } else {blue_agent.choose(&battle, dex, side_blue, &choices_blue)})
        };

        // 2. Red chooses its action according to `AttackerTier`
        let red_choice = if choices_red.is_empty() {
            None
        } else if let Some(ref mut std_red) = red_std_agent {
            Some(std_red.choose(&battle, dex, side_red, &choices_red))
        } else {
            let is_preview = matches!(choices_red[0], SearchChoice::Team(_));
            if red_obs.is_none() || is_preview {
                let obs = if corrected && tier==AttackerTier::T2RulesBayesianOracleBr {
                    let (local,obs)=LocalPlayer::new(&battle,dex,side_red).unwrap();red_local=Some(local);obs
                } else {Observer::new(&battle,side_red)};
                let bel = match tier {
                    AttackerTier::T2RulesBayesianOracleBr => {
                        Belief::with_fallback_policy(
                            dex,
                            &red_prior,
                            &obs,
                            FallbackPolicy::default(),
                        )
                    }
                    AttackerTier::T3ClairvoyantOpenSheet
                    | AttackerTier::T4Clairvoyant3v3FullInfo
                    | AttackerTier::T5OmniscientGodBr
                    | AttackerTier::T6PsychicMoveReaderGod => {
                        Belief::pinned_from_battle(&battle, &obs)
                    }
                    _ => unreachable!(),
                };
                red_obs = Some(obs);
                red_belief = Some(bel);

                let b_obs = if corrected && tier==AttackerTier::T2RulesBayesianOracleBr {
                    let (local,obs)=LocalPlayer::new(&battle,dex,side_blue).unwrap();blue_public_local=Some(local);obs
                } else {Observer::new(&battle,side_blue)};
                let b_bel = Belief::with_fallback_policy(
                    dex,
                    &blue_prior,
                    &b_obs,
                    FallbackPolicy::default(),
                );
                blue_public_obs = Some(b_obs);
                blue_public_bel = Some(b_bel);
            }

            let obs = red_obs.as_mut().unwrap();
            let bel = red_belief.as_mut().unwrap();
            if let Some(local)=red_local.as_mut() {local.observe(&battle,dex,obs).unwrap();}
            else {obs.observe(&battle,dex);}
            bel.sync(dex, obs);
            let public_position=red_local.as_ref().map(|local|local.position(dex,bel,obs,&mut red_rng).unwrap());
            if let (Some(b_obs), Some(b_bel)) = (blue_public_obs.as_mut(), blue_public_bel.as_mut()) {
                if let Some(local)=blue_public_local.as_mut() {
                    local.observe(&battle,dex,b_obs).unwrap();local.condition_belief(b_bel,b_obs);
                } else {b_obs.observe(&battle,dex);}
                b_bel.sync(dex, b_obs);
                if !is_preview && !corrected {
                    b_bel.condition_canonical_pick_policy(
                        &oracle.blue_opp_pol60,
                        0.10,
                        b_obs,
                    );
                }
            }

            if is_preview {
                if let Some(bc) = blue_choice {
                    blue_pick_one_hot60 = one_hot_canonical_60(bc);
                }
                let picked = match tier {
                    AttackerTier::T2RulesBayesianOracleBr => {
                        draw_canonical_60(&choices_red, &oracle.red_bayesian_br_pol60, &mut red_rng)
                    }
                    AttackerTier::T3ClairvoyantOpenSheet => draw_canonical_60(
                        &choices_red,
                        &oracle.red_sheet_clairvoyant_pol60,
                        &mut red_rng,
                    ),
                    AttackerTier::T4Clairvoyant3v3FullInfo
                    | AttackerTier::T5OmniscientGodBr
                    | AttackerTier::T6PsychicMoveReaderGod => {
                        oracle.pick_clairvoyant_counter(blue_choice.unwrap(), &choices_red)
                    }
                    _ => unreachable!(),
                };
                Some(picked)
            } else {
                match tier {
                    AttackerTier::T2RulesBayesianOracleBr => {
                        bel.condition_canonical_pick_policy(
                            &oracle.red_opp_shell_pol60,
                            0.10,
                            obs,
                        );
                    }
                    AttackerTier::T3ClairvoyantOpenSheet => {
                        bel.condition_canonical_pick_policy(&oracle.blue_own_pol60, 0.05, obs);
                    }
                    AttackerTier::T4Clairvoyant3v3FullInfo
                    | AttackerTier::T5OmniscientGodBr
                    | AttackerTier::T6PsychicMoveReaderGod => {
                        if let Some(ref oh60) = blue_pick_one_hot60 {
                            bel.condition_canonical_pick_policy(oh60, 0.0, obs);
                        }
                    }
                    _ => unreachable!(),
                }

                if choices_red.len() == 1 {
                    Some(choices_red[0])
                } else {
                    let chosen = match tier {
                        AttackerTier::T3ClairvoyantOpenSheet
                        | AttackerTier::T4Clairvoyant3v3FullInfo => {
                            let mut cfg = base_cfg.clone();
                            cfg.iterations = if same_budget { base_iters } else { base_iters * 2 };
                            let mut bs = BlindSearch::new(
                                &battle,
                                dex,
                                cfg.clone(),
                                side_red,
                                red_rng.next(),
                            );
                            bs.prune_dominated();
                            for _ in 0..cfg.iterations {
                                bs.step_one(dex, bel, obs);
                                if bs.should_early_stop() {
                                    break;
                                }
                            }
                            bs.best().unwrap_or(choices_red[0])
                        }
                        AttackerTier::T2RulesBayesianOracleBr => {
                            // Rules-compliant Coupled Bayesian Policy-Oracle Best Response:
                            // 1. Sample `k_oracle` determinized worlds `det_b` from Red's public posterior `bel`.
                            // 2. In each world `det_b`, query Blue's 1x `BlindSearch` using only Blue's publicly
                            //    observable belief `blue_public_bel` & `blue_public_obs` tracked from Turn 0.
                            // 3. Evaluate Red's actions coupled to each `(det_b, rep_k)` world (including `BoundSolver` in 1v1).
                            let mut coupled_worlds: Vec<(Battle, SearchChoice, Vec<SearchChoice>, Option<ObservedPosition>)> =
                                Vec::with_capacity(k_oracle);
                            if let (Some(b_obs), Some(b_bel)) =
                                (blue_public_obs.as_ref(), blue_public_bel.as_ref())
                            {
                                for _ in 0..k_oracle {
                                    let sampled=public_position.as_ref().map(|p|p.sample(dex,bel,obs,&mut red_rng));
                                    let mut det_b=if let Some(p)=&sampled {p.model().clone()} else {
                                        let cand=bel.sample(&mut red_rng);
                                        bel.determinize_with(dex,&battle,obs,cand,&mut red_rng)
                                    };
                                    det_b.set_log_enabled(false);
                                    let det_blue_choices = det_b.legal_choices(dex, side_blue);
                                    if !det_blue_choices.is_empty() {
                                        let rep = if det_blue_choices.len() == 1 {
                                            det_blue_choices[0]
                                        } else {
                                            let mut b_search = model_search(sampled.as_ref(),
                                                &det_b,
                                                dex,
                                                base_cfg.clone(),
                                                side_blue,
                                                red_rng.next(),
                                            );
                                            for _ in 0..base_cfg.iterations {
                                                b_search.step_one(dex, b_bel, b_obs);
                                                if b_search.should_early_stop() {
                                                    break;
                                                }
                                            }
                                            b_search.best().unwrap_or(det_blue_choices[0])
                                        };
                                        coupled_worlds.push((det_b, rep, det_blue_choices, sampled));
                                    }
                                }
                            }

                            let alive_red = count_alive_mons(public_position.as_ref().map(|p|p.model()).unwrap_or(&battle), side_red);
                            let alive_blue = count_alive_mons(public_position.as_ref().map(|p|p.model()).unwrap_or(&battle), side_blue);
                            let mut solver_pick: Option<SearchChoice> = None;
                            if alive_red == 1 && alive_blue == 1 && !coupled_worlds.is_empty() {
                                let mut ev_by_a = vec![0.0f64; choices_red.len()];
                                let mut solved_w = 0usize;
                                for (det_b, rep_k, det_b_choices, _) in &coupled_worlds {
                                    if let Some(b_idx) =
                                        det_b_choices.iter().position(|&c| c == *rep_k)
                                    {
                                        let mut solver = BoundSolver::new(
                                            dex,
                                            BoundConfig {
                                                time_budget: Some(Duration::from_millis(80)),
                                                work_budget: 25_000,
                                                node_budget: 60_000,
                                                cell_cap: 2048,
                                                resource_closure_nodes: Some(6000),
                                                eps: 0.02,
                                                ..Default::default()
                                            },
                                        );
                                        let _ = solver.solve(det_b, None);
                                        if let Some(mat) = solver.root_payoffs(det_b) {
                                            if mat.value.width() <= 0.10
                                                && mat.actions[side_red].len() == choices_red.len()
                                                && mat.actions[side_blue].len()
                                                    == det_b_choices.len()
                                            {
                                                solved_w += 1;
                                                let cols = mat.actions[1].len();
                                                for a_idx in 0..choices_red.len() {
                                                    let cell = if side_red == 0 {
                                                        mat.cells[a_idx * cols + b_idx].mid()
                                                    } else {
                                                        1.0 - mat.cells[b_idx * cols + a_idx].mid()
                                                    };
                                                    ev_by_a[a_idx] += cell;
                                                }
                                            }
                                        }
                                    }
                                }
                                if solved_w == coupled_worlds.len() {
                                    solver_interventions += 1;
                                    let best_a = (0..choices_red.len())
                                        .max_by(|&a, &b| ev_by_a[a].total_cmp(&ev_by_a[b]))
                                        .unwrap_or(0);
                                    solver_pick = Some(choices_red[best_a]);
                                }
                            }

                            if let Some(sp) = solver_pick {
                                sp
                            } else if coupled_worlds.is_empty() {
                                let mut bs = model_search(public_position.as_ref(),
                                    public_position.as_ref().map(|p|p.model()).unwrap_or(&battle),
                                    dex,
                                    base_cfg.clone(),
                                    side_red,
                                    red_rng.next(),
                                );
                                for _ in 0..(base_iters * 3) {
                                    bs.step_one(dex, bel, obs);
                                    if bs.should_early_stop() {
                                        break;
                                    }
                                }
                                bs.best().unwrap_or(choices_red[0])
                            } else {
                                // Coupled multi-world UCB-BR: each sampled world `det_b` has its own
                                // `BlindSearch` + `Pinned` belief paired with `rep_k`, while `(br_n, br_w)`
                                // are shared across the `k_oracle` worlds!
                                let (w_total, br_total) = if same_budget {
                                    let w = (base_iters / 4).max(1);
                                    (w, base_iters.saturating_sub(w).max(1))
                                } else {
                                    (base_iters, base_iters * 2)
                                };
                                let k_w = coupled_worlds.len();
                                let w_per_world = ((w_total as usize) / k_w).max(1) as u32;
                                let mut world_searches: Vec<(BlindSearch, Belief, SearchChoice)> =
                                    Vec::with_capacity(k_w);
                                for (det_b, rep_k, _, sampled) in &coupled_worlds {
                                    let det_bel = Belief::pinned_from_battle(det_b, obs);
                                    let mut w_bs = model_search(sampled.as_ref(),
                                        det_b,
                                        dex,
                                        base_cfg.clone(),
                                        side_red,
                                        red_rng.next(),
                                    );
                                    w_bs.prune_dominated();
                                    for _ in 0..w_per_world {
                                        w_bs.step_one(dex, &det_bel, obs);
                                    }
                                    world_searches.push((w_bs, det_bel, *rep_k));
                                }
                                let actions = world_searches[0].0.actions().to_vec();
                                let n_own = actions.len();
                                let dom = world_searches[0].0.dominated().to_vec();
                                let any_safe = dom.iter().any(|&d| !d);
                                let eligible: Vec<usize> =
                                    (0..n_own).filter(|&i| !any_safe || !dom[i]).collect();
                                if eligible.len() == 1 {
                                    actions[eligible[0]]
                                } else {
                                    let mut br_n = vec![0u32; n_own];
                                    let mut br_w = vec![0.0f64; n_own];
                                    for it in 0..(br_total as usize) {
                                        let own_a = if it < eligible.len() * 2 * k_w {
                                            eligible[(it / k_w) % eligible.len()]
                                        } else {
                                            let tot = (it as f64).ln();
                                            let mut best_a = eligible[0];
                                            let mut best_ucb = f64::NEG_INFINITY;
                                            for &a in &eligible {
                                                let na = (br_n[a].max(1)) as f64;
                                                let mean = br_w[a] / na;
                                                let ucb = mean + 0.40 * (tot / na).sqrt();
                                                if ucb > best_ucb {
                                                    best_ucb = ucb;
                                                    best_a = a;
                                                }
                                            }
                                            best_a
                                        };
                                        let (ref mut w_bs, ref det_bel, rep_k) =
                                            world_searches[it % k_w];
                                        let r_side0 =
                                            w_bs.step_forced_joint(dex, det_bel, obs, own_a, rep_k);
                                        let r_my = if side_red == 0 {
                                            r_side0
                                        } else {
                                            1.0 - r_side0
                                        };
                                        br_n[own_a] += 1;
                                        br_w[own_a] += r_my;
                                    }
                                    let max_n =
                                        eligible.iter().map(|&a| br_n[a]).max().unwrap_or(1);
                                    let min_n = (max_n / 3).max(2);
                                    let best_idx = eligible
                                        .into_iter()
                                        .filter(|&a| br_n[a] >= min_n)
                                        .max_by(|&a, &b| {
                                            let ma = br_w[a] / (br_n[a].max(1) as f64);
                                            let mb = br_w[b] / (br_n[b].max(1) as f64);
                                            ma.total_cmp(&mb).then_with(|| br_n[a].cmp(&br_n[b]))
                                        })
                                        .unwrap_or(0);
                                    actions[best_idx]
                                }
                            }
                        }
                        AttackerTier::T5OmniscientGodBr => {
                            // Full-Info + True Live-Policy Oracle (`blue_agent.belief()` & `blue_agent.observer()`)
                            // + Certified `1v1` Endgame `BoundSolver`
                            let mut sampled_replies = Vec::with_capacity(k_oracle);
                            let mut reply_dist = vec![0.0f64; choices_blue.len()];
                            if !choices_blue.is_empty() {
                                if choices_blue.len() == 1 {
                                    sampled_replies.push(choices_blue[0]);
                                    reply_dist[0] = 1.0;
                                } else if let (Some(blue_bel), Some(blue_obs)) =
                                    (blue_agent.belief(), blue_agent.observer())
                                {
                                    for _ in 0..k_oracle {
                                        let mut b_search = BlindSearch::new(
                                            &battle,
                                            dex,
                                            base_cfg.clone(),
                                            side_blue,
                                            red_rng.next(),
                                        );
                                        for _ in 0..base_cfg.iterations {
                                            b_search.step_one(dex, blue_bel, blue_obs);
                                            if b_search.should_early_stop() {
                                                break;
                                            }
                                        }
                                        let rep = b_search.best().unwrap_or(choices_blue[0]);
                                        sampled_replies.push(rep);
                                        if let Some(pos) =
                                            choices_blue.iter().position(|&c| c == rep)
                                        {
                                            reply_dist[pos] += 1.0 / (k_oracle as f64);
                                        }
                                    }
                                }
                            }

                            let alive_red = count_alive_mons(&battle, side_red);
                            let alive_blue = count_alive_mons(&battle, side_blue);
                            let mut solver_pick: Option<SearchChoice> = None;
                            if alive_red == 1 && alive_blue == 1 && !choices_blue.is_empty() {
                                let mut solver = BoundSolver::new(
                                    dex,
                                    BoundConfig {
                                        time_budget: Some(Duration::from_millis(80)),
                                        work_budget: 25_000,
                                        node_budget: 60_000,
                                        cell_cap: 2048,
                                        resource_closure_nodes: Some(6000),
                                        eps: 0.02,
                                        ..Default::default()
                                    },
                                );
                                let _ = solver.solve(&battle, None);
                                if let Some(mat) = solver.root_payoffs(&battle) {
                                    if mat.value.width() <= 0.10
                                        && mat.actions[side_red].len() == choices_red.len()
                                        && mat.actions[side_blue].len() == choices_blue.len()
                                    {
                                        solver_interventions += 1;
                                        let cols = mat.actions[1].len();
                                        let mut best_a = 0usize;
                                        let mut best_ev = f64::NEG_INFINITY;
                                        for a_idx in 0..choices_red.len() {
                                            let mut ev = 0.0f64;
                                            for (b_idx, &prob) in reply_dist.iter().enumerate() {
                                                if prob <= 0.0 {
                                                    continue;
                                                }
                                                let cell = if side_red == 0 {
                                                    mat.cells[a_idx * cols + b_idx].mid()
                                                } else {
                                                    1.0 - mat.cells[b_idx * cols + a_idx].mid()
                                                };
                                                ev += prob * cell;
                                            }
                                            if ev > best_ev {
                                                best_ev = ev;
                                                best_a = a_idx;
                                            }
                                        }
                                        solver_pick = Some(choices_red[best_a]);
                                    }
                                }
                            }

                            if let Some(sp) = solver_pick {
                                sp
                            } else {
                                let mut bs = BlindSearch::new(
                                    &battle,
                                    dex,
                                    base_cfg.clone(),
                                    side_red,
                                    red_rng.next(),
                                );
                                let (w_it, br_it) = if same_budget {
                                    let w = (base_iters / 4).max(1);
                                    (w, base_iters.saturating_sub(w).max(1))
                                } else {
                                    (base_iters * 2, base_iters * 6)
                                };
                                run_ucb_best_response_search(
                                    &mut bs,
                                    dex,
                                    bel,
                                    obs,
                                    side_red,
                                    &sampled_replies,
                                    w_it,
                                    br_it,
                                )
                            }
                        }
                        AttackerTier::T6PsychicMoveReaderGod => {
                            // Psychic Simultaneous-Move Reader: peeks at `blue_choice` (`b_actual`)!
                            let alive_red = count_alive_mons(&battle, side_red);
                            let alive_blue = count_alive_mons(&battle, side_blue);
                            let mut solver_pick: Option<SearchChoice> = None;
                            if let Some(b_actual) = blue_choice {
                                if alive_red == 1 && alive_blue == 1 && !choices_blue.is_empty() {
                                    if let Some(b_idx) =
                                        choices_blue.iter().position(|&c| c == b_actual)
                                    {
                                        let mut solver = BoundSolver::new(
                                            dex,
                                            BoundConfig {
                                                time_budget: Some(Duration::from_millis(80)),
                                                work_budget: 25_000,
                                                node_budget: 60_000,
                                                cell_cap: 2048,
                                                resource_closure_nodes: Some(6000),
                                                eps: 0.02,
                                                ..Default::default()
                                            },
                                        );
                                        let _ = solver.solve(&battle, None);
                                        if let Some(mat) = solver.root_payoffs(&battle) {
                                            if mat.value.width() <= 0.10
                                                && mat.actions[side_red].len() == choices_red.len()
                                                && mat.actions[side_blue].len()
                                                    == choices_blue.len()
                                            {
                                                solver_interventions += 1;
                                                let cols = mat.actions[1].len();
                                                let mut best_a = 0usize;
                                                let mut best_ev = f64::NEG_INFINITY;
                                                for a_idx in 0..choices_red.len() {
                                                    let cell = if side_red == 0 {
                                                        mat.cells[a_idx * cols + b_idx].mid()
                                                    } else {
                                                        1.0 - mat.cells[b_idx * cols + a_idx].mid()
                                                    };
                                                    if cell > best_ev {
                                                        best_ev = cell;
                                                        best_a = a_idx;
                                                    }
                                                }
                                                solver_pick = Some(choices_red[best_a]);
                                            }
                                        }
                                    }
                                }
                            }

                            if let Some(sp) = solver_pick {
                                sp
                            } else {
                                let mut bs = BlindSearch::new(
                                    &battle,
                                    dex,
                                    base_cfg.clone(),
                                    side_red,
                                    red_rng.next(),
                                );
                                let rep_slice = blue_choice.map(|c| vec![c]).unwrap_or_default();
                                let (w_it, br_it) = if same_budget {
                                    let w = (base_iters / 4).max(1);
                                    (w, base_iters.saturating_sub(w).max(1))
                                } else {
                                    (base_iters * 2, base_iters * 6)
                                };
                                run_ucb_best_response_search(
                                    &mut bs,
                                    dex,
                                    bel,
                                    obs,
                                    side_red,
                                    &rep_slice,
                                    w_it,
                                    br_it,
                                )
                            }
                        }
                        _ => unreachable!(),
                    };
                    Some(chosen)
                }
            }
        };

        let pair_choices = if blue_is_p1 {
            [blue_choice, red_choice]
        } else {
            [red_choice, blue_choice]
        };
        battle.apply_choices(dex, pair_choices).unwrap();
    }
}

fn draw_canonical_60(
    legal_choices: &[SearchChoice],
    pol60: &[f64],
    rng: &mut SplitMix64,
) -> SearchChoice {
    let actions = preview_actions();
    let mut weights = vec![0.0f64; legal_choices.len()];
    let mut sum = 0.0f64;
    for (i, &c) in legal_choices.iter().enumerate() {
        if let SearchChoice::Team(t) = c {
            if let Some(idx60) = action_index(&actions, t) {
                let w = pol60.get(idx60).copied().unwrap_or(0.0);
                weights[i] = w;
                sum += w;
            }
        }
    }
    if sum <= 1e-12 {
        return legal_choices[rng.below(legal_choices.len())];
    }
    let u = rng.next_f64() * sum;
    let mut acc = 0.0f64;
    for (i, &w) in weights.iter().enumerate() {
        acc += w;
        if u <= acc && w > 0.0 {
            return legal_choices[i];
        }
    }
    legal_choices[0]
}

fn main() {
    let t_start = Instant::now();
    let dex = load_dex();
    let root = repo_root();
    let ls_text = std::fs::read_to_string(root.join("data/learnsets-gen2.json")).unwrap();
    let pool_text = std::fs::read_to_string(root.join("data/meta-pool-v0/meta-pool.json")).unwrap();
    let gen = TeamGen::new(&dex, &ls_text, &pool_text).unwrap();

    let args: Vec<String> = std::env::args().collect();
    let find_flag = |k: &str| -> Option<String> {
        args.windows(2).find(|w| w[0] == k).map(|w| w[1].clone())
    };
    let corrected=args.iter().any(|a|a=="--corrected-product");
    let compiled:Value=serde_json::from_slice(&std::fs::read(root.join("data/compiled-preview-v1.json")).unwrap()).unwrap();
    if corrected {assert_eq!(compiled["build"],nc2000_bot::compiled_preview::build_fingerprint(),"stale corrected preview artifact");}
    let base_iters: u32 = find_flag("--iters")
        .and_then(|v| v.parse().ok())
        .unwrap_or(400);
    let num_pairs: usize = find_flag("--pairs")
        .and_then(|v| v.parse().ok())
        .unwrap_or(24);
    let endgame_states_target: usize = find_flag("--endgame-states")
        .and_then(|v| v.parse().ok())
        .unwrap_or(24);
    let same_budget: bool = args.iter().any(|a| a == "--same-budget");
    let k_oracle: usize = find_flag("--k-oracle")
        .and_then(|v| v.parse().ok())
        .unwrap_or(if same_budget { 2 } else { 6 });
    let max_turns: u16 = find_flag("--max-turns").and_then(|s|s.parse().ok()).unwrap_or(250);
    let seed: u64 = find_flag("--seed").and_then(|s|s.parse().ok()).unwrap_or(if corrected {2026110307} else {20261102});
    let threads: usize = find_flag("--threads").and_then(|s|s.parse().ok()).unwrap_or_else(||std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(8));
    let out_path = PathBuf::from(find_flag("--out").unwrap_or_else(|| {
        "data/dominance-audit-v13-verified/omniscient-audit-report.json".to_string()
    }));

    println!("==========================================================================================");
    println!(
        "=== OMNISCIENT & BEST-RESPONSE CEILING AUDIT (iters={}, pairs={} [{}g/tier], threads={}) ===",
        base_iters,
        num_pairs,
        num_pairs * 2,
        threads
    );
    println!("==========================================================================================");

    // 1. Load the 5-team Equilibrium `data/kodoku-r3/conclusion-equilibrium-deme.json`
    let eq_txt =
        std::fs::read_to_string(root.join("data/kodoku-r3/conclusion-equilibrium-deme.json"))
            .expect("read conclusion-equilibrium-deme.json");
    let eq_v: Value = serde_json::from_str(&eq_txt).unwrap();
    let mut pi_ids: Vec<String> = Vec::new();
    let mut pi_weights: Vec<f64> = Vec::new();
    let mut pi_sets: Vec<Vec<PokemonSet>> = Vec::new();
    let mut pi_rosters: Vec<CompiledRoster6> = Vec::new();
    let mut rng_norm = SplitMix64::new(0xC0DE_BA5E_2026_1102);

    for t in eq_v["teams"].as_array().unwrap() {
        let id = t["id"].as_str().unwrap().to_string();
        let w = t["weight"].as_f64().unwrap();
        let mut raw = t["sets"].as_array().unwrap().clone();
        if !corrected {gen.optimize_team_dominance(&dex, &mut raw, &mut rng_norm);}
        let canon = gen.canonize(&dex, &raw).unwrap();
        let sets = to_sets(&canon).unwrap();
        let roster = CompiledRoster6::compile(&dex, &sets);
        pi_ids.push(id);
        pi_weights.push(w);
        pi_sets.push(sets);
        pi_rosters.push(roster);
    }
    let n_pi = pi_sets.len();

    let bp_text =
        std::fs::read_to_string(root.join("data/belief-pool-v3/belief-pool.json")).unwrap();
    let raw_mp: MetaPool = serde_json::from_str(&bp_text).unwrap();
    let mut pi_sigs: BTreeSet<PreviewSig> = BTreeSet::new();
    for s in &pi_sets {
        pi_sigs.insert(preview_sig_sets(s));
    }
    let product_prior=Arc::new(raw_mp.clone());
    let mut clean_teams: Vec<MetaTeam> = raw_mp
        .teams
        .into_iter()
        .filter(|t| !pi_sigs.contains(&preview_sig_sets(&t.sets)))
        .collect();
    for k in 0..n_pi {
        clean_teams.push(MetaTeam {
            id: format!("pi-star-{}", pi_ids[k]),
            sets: pi_sets[k].clone(),
            weight: Some(pi_weights[k]),
        });
    }
    let pi_star_mp = Arc::new(MetaPool { teams: clean_teams });

    // 2. Precompute anti-symmetric 5x5 pairwise 3v3 preview matrices (4 CRN seeds per cell)
    //    so that `M(b, a)[ib, ia] == 1.0 - M(a, b)[ia, ib]` identically!
    println!("[Step 1] Precomputing anti-symmetric 5x5 equilibrium preview matrices & Bayesian Shell Nash / BR tables...");
    let mut pair_mats: Vec<Vec<f64>> = vec![vec![]; n_pi * n_pi];
    for r_i in 0..n_pi {
        for b_j in r_i..n_pi {
            let p_seed = seed
                ^ 0x3C6E_F372_FE94_F82A_u64
                ^ (((r_i * 31 + b_j) as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15));
            let raw = if corrected {native_preview_matrix(&dex,&pi_rosters[r_i],&pi_rosters[b_j],4,max_turns,p_seed)}
            else {eval_full_preview_matrix_compact(
                &pi_rosters[r_i],
                &pi_rosters[b_j],
                4,
                max_turns,
                p_seed,
            )};
            let ka = pi_rosters[r_i].legal.len();
            let kb = pi_rosters[b_j].legal.len();
            if r_i == b_j {
                let mut sym = vec![0.5f64; ka * kb];
                for ia in 0..ka {
                    for ib in 0..kb {
                        sym[ia * kb + ib] =
                            0.5 * (raw[ia * kb + ib] + (1.0 - raw[ib * ka + ia]));
                    }
                }
                pair_mats[r_i * n_pi + b_j] = sym;
            } else {
                let mut trans = vec![0.5f64; kb * ka];
                for ia in 0..ka {
                    for ib in 0..kb {
                        trans[ib * ka + ia] = 1.0 - raw[ia * kb + ib];
                    }
                }
                pair_mats[r_i * n_pi + b_j] = raw;
                pair_mats[b_j * n_pi + r_i] = trans;
            }
        }
    }

    let mut shell_groups: BTreeMap<PreviewSig, Vec<usize>> = BTreeMap::new();
    for k in 0..n_pi {
        shell_groups
            .entry(preview_sig_sets(&pi_sets[k]))
            .or_default()
            .push(k);
    }

    let mut oracles: Vec<PrecomputedMatchupOracle> = Vec::with_capacity(n_pi * n_pi);
    for _ in 0..(n_pi * n_pi) {
        oracles.push(PrecomputedMatchupOracle {
            mat_red_vs_blue: vec![],
            ka: 0,
            kb: 0,
            red_legal: vec![],
            blue_legal: vec![],
            blue_own_pol60: vec![],
            blue_opp_pol60: vec![],
            red_nash_pol60: vec![],
            red_opp_shell_pol60: vec![],
            red_bayesian_br_pol60: vec![],
            red_sheet_clairvoyant_pol60: vec![],
        });
    }

    for (_sig_r, r_members) in &shell_groups {
        for (_sig_b, b_members) in &shell_groups {
            let n_u = r_members.len();
            let n_v = b_members.len();
            let ka = pi_rosters[r_members[0]].legal.len();
            let kb = pi_rosters[b_members[0]].legal.len();
            let alpha: Vec<f64> = r_members.iter().map(|&i| pi_weights[i]).collect();
            let beta: Vec<f64> = b_members.iter().map(|&j| pi_weights[j]).collect();
            let a_sum: f64 = alpha.iter().sum::<f64>().max(1e-12);
            let b_sum: f64 = beta.iter().sum::<f64>().max(1e-12);

            let mut mats_refs: Vec<&[f64]> = Vec::with_capacity(n_u * n_v);
            for &r_i in r_members {
                for &b_j in b_members {
                    mats_refs.push(pair_mats[r_i * n_pi + b_j].as_slice());
                }
            }
            let (p_strats, mut q_strats) =
                solve_bayesian_shell_preview_nash(&mats_refs, ka, kb, &alpha, &beta, 600);
            if corrected {
                for (v,&b_j) in b_members.iter().enumerate() {
                    let root=Battle::from_fixture(&dex,"1,2,3,4",&pi_sets[b_j],&pi_sets[r_members[0]]).unwrap();
                    let key=nc2000_bot::compiled_preview::key(&root,0).unwrap();
                    let choices=compiled["policies"][&key].as_array().expect("missing regenerated Nash preview");
                    let mut p=vec![0.;kb];
                    for choice in choices {
                        let numbers:Vec<usize>=choice.as_str().unwrap().strip_prefix("team ").unwrap().split(',').map(|x|x.trim().parse::<usize>().unwrap()-1).collect();
                        let slot=pi_rosters[b_j].legal.iter().position(|t|t[0]==numbers[0] && ((t[1]==numbers[1]&&t[2]==numbers[2])||(t[1]==numbers[2]&&t[2]==numbers[1]))).unwrap();
                        p[slot]+=1./choices.len() as f64;
                    }
                    q_strats[v]=p;
                }
            }

            let mut r_shell_avg = vec![0.0f64; ka];
            for (u_k, &r_i) in r_members.iter().enumerate() {
                let w_u = pi_weights[r_i] / a_sum;
                for ia in 0..ka {
                    r_shell_avg[ia] += w_u * p_strats[u_k][ia];
                }
            }
            let mut b_shell_avg = vec![0.0f64; kb];
            for (v_m, &b_j) in b_members.iter().enumerate() {
                let w_v = pi_weights[b_j] / b_sum;
                for ib in 0..kb {
                    b_shell_avg[ib] += w_v * q_strats[v_m][ib];
                }
            }

            let r_shell_pol60 =
                expand_to_canonical_60(&pi_rosters[r_members[0]].legal, &r_shell_avg);
            let b_shell_pol60 =
                expand_to_canonical_60(&pi_rosters[b_members[0]].legal, &b_shell_avg);

            for (u_k, &r_i) in r_members.iter().enumerate() {
                let mut best_bayes_ia = 0usize;
                let mut best_bayes_ev = f64::NEG_INFINITY;
                for ia in 0..ka {
                    let mut ev = 0.0f64;
                    for (v_m, &b_j) in b_members.iter().enumerate() {
                        let w_v = pi_weights[b_j] / b_sum;
                        let mat = &pair_mats[r_i * n_pi + b_j];
                        let mut dot = 0.0f64;
                        for ib in 0..kb {
                            dot += mat[ia * kb + ib] * q_strats[v_m][ib];
                        }
                        ev += w_v * dot;
                    }
                    if ev > best_bayes_ev {
                        best_bayes_ev = ev;
                        best_bayes_ia = ia;
                    }
                }
                let mut bayes_br_vec = vec![0.0f64; ka];
                bayes_br_vec[best_bayes_ia] = 1.0;
                let red_bayesian_br_pol60 =
                    expand_to_canonical_60(&pi_rosters[r_i].legal, &bayes_br_vec);

                for (v_m, &b_j) in b_members.iter().enumerate() {
                    let mat = &pair_mats[r_i * n_pi + b_j];
                    let mut best_sheet_ia = 0usize;
                    let mut best_sheet_ev = f64::NEG_INFINITY;
                    for ia in 0..ka {
                        let mut dot = 0.0f64;
                        for ib in 0..kb {
                            dot += mat[ia * kb + ib] * q_strats[v_m][ib];
                        }
                        if dot > best_sheet_ev {
                            best_sheet_ev = dot;
                            best_sheet_ia = ia;
                        }
                    }
                    let mut sheet_br_vec = vec![0.0f64; ka];
                    sheet_br_vec[best_sheet_ia] = 1.0;
                    let red_sheet_clairvoyant_pol60 =
                        expand_to_canonical_60(&pi_rosters[r_i].legal, &sheet_br_vec);

                    oracles[r_i * n_pi + b_j] = PrecomputedMatchupOracle {
                        mat_red_vs_blue: mat.clone(),
                        ka,
                        kb,
                        red_legal: pi_rosters[r_i].legal.clone(),
                        blue_legal: pi_rosters[b_j].legal.clone(),
                        blue_own_pol60: expand_to_canonical_60(
                            &pi_rosters[b_j].legal,
                            &q_strats[v_m],
                        ),
                        blue_opp_pol60: r_shell_pol60.clone(),
                        red_nash_pol60: expand_to_canonical_60(
                            &pi_rosters[r_i].legal,
                            &p_strats[u_k],
                        ),
                        red_opp_shell_pol60: b_shell_pol60.clone(),
                        red_bayesian_br_pol60: red_bayesian_br_pol60.clone(),
                        red_sheet_clairvoyant_pol60,
                    };
                }
            }
        }
    }

    // 3. Build QMC CRN-paired matchup jobs `(red_idx, blue_idx, battle_seed, blue_is_p1)`
    struct DuelJob {
        red_idx: usize,
        blue_idx: usize,
        battle_seed: String,
        blue_seed: u64,
        red_seed: u64,
        blue_is_p1: bool,
    }

    let mut cum_w = Vec::with_capacity(n_pi);
    let mut acc = 0.0f64;
    for &w in &pi_weights {
        acc += w;
        cum_w.push(acc);
    }
    let w_tot = acc.max(1e-12);
    let phi1 = 0.618_033_988_749_894_9_f64;
    let phi2 = 0.381_966_011_250_105_1_f64;
    let mut jobs: Vec<DuelJob> = Vec::with_capacity(num_pairs * 2);
    let mut matchup_rng=SplitMix64::new(seed ^ 0xD6E8_FEB8_6659_FD93);
    for m in 0..num_pairs {
        // Independent mixture draws in the corrected experiment. The old
        // complementary QMC coordinates correlated the two hidden types.
        let (u_b,u_r)=if corrected {(matchup_rng.next_f64()*w_tot,matchup_rng.next_f64()*w_tot)}
            else {((((m as f64)+0.5)*phi1).rem_euclid(1.0)*w_tot,(((m as f64)+0.5)*phi2).rem_euclid(1.0)*w_tot)};
        let blue_idx = cum_w
            .iter()
            .position(|&cw| cw >= u_b)
            .unwrap_or(n_pi - 1);
        let red_idx = cum_w
            .iter()
            .position(|&cw| cw >= u_r)
            .unwrap_or(n_pi - 1);
        let mut rng_j = SplitMix64::new(
            seed ^ ((m as u64 + 1).wrapping_mul(0x9E37_79B9_7F4A_7C15))
                ^ (((blue_idx * 7 + red_idx) as u64).wrapping_mul(0xBF58_476D_1CE4_E5B9)),
        );
        let bseed = rng_j.battle_seed();
        let s_a = rng_j.next();
        let s_b = rng_j.next();
        // Exact CRN Mirror Pair:
        // Game 0: P1 = Blue(team_A, seed_A), P2 = Red(team_B, seed_B)
        // Game 1: P1 = Red(team_A, seed_A),  P2 = Blue(team_B, seed_B)
        // When Red == Blue (`T0_Baseline_Blind400`), Game 0 and Game 1 are bit-for-bit identical battles
        // with roles swapped, guaranteeing `Red_WR == 50.00% (±0.00%)`!
        jobs.push(DuelJob {
            red_idx,
            blue_idx,
            battle_seed: bseed.clone(),
            blue_seed: s_a,
            red_seed: s_b,
            blue_is_p1: true,
        });
        jobs.push(DuelJob {
            red_idx: blue_idx,
            blue_idx: red_idx,
            battle_seed: bseed,
            blue_seed: s_b,
            red_seed: s_a,
            blue_is_p1: false,
        });
    }

    let all_tiers = [
        AttackerTier::T0BaselineBlind400,
        AttackerTier::T1aRootPruningOnly,
        AttackerTier::T1bAdaptiveMixOnly,
        AttackerTier::T1cBlind400AdaptiveMix,
        AttackerTier::T1SuperCompute3200Mix,
        AttackerTier::T2aRulesSuperSearch3x,
        AttackerTier::T2RulesBayesianOracleBr,
        AttackerTier::T3ClairvoyantOpenSheet,
        AttackerTier::T4Clairvoyant3v3FullInfo,
        AttackerTier::T5OmniscientGodBr,
        AttackerTier::T6PsychicMoveReaderGod,
    ];
    let tiers: Vec<AttackerTier> = if let Some(filter_str) = find_flag("--tiers") {
        let wanted: Vec<&str> = filter_str.split(',').map(|s| s.trim()).collect();
        all_tiers
            .iter()
            .copied()
            .filter(|t| wanted.iter().any(|&w| t.id().contains(w)))
            .collect()
    } else if same_budget {
        vec![
            AttackerTier::T1aRootPruningOnly,
            AttackerTier::T1bAdaptiveMixOnly,
            AttackerTier::T1cBlind400AdaptiveMix,
            AttackerTier::T2RulesBayesianOracleBr,
            AttackerTier::T5OmniscientGodBr,
            AttackerTier::T6PsychicMoveReaderGod,
        ]
    } else {
        all_tiers.to_vec()
    };

    println!(
        "\n[Step 2] Running Full-Game Head-to-Head Escalating Omniscient Duels ({} games/tier, same_budget={}, k_oracle={})...",
        jobs.len(),
        same_budget,
        k_oracle
    );
    let mut tier_reports: Vec<Value> = Vec::new();
    for &tier in &tiers {
        let t_tier = Instant::now();
        let cursor = AtomicUsize::new(0);
        let mut game_outs: Vec<(usize, f64, u16, usize, nc2000_bot::agent::RootSearchStats)> =
            Vec::with_capacity(jobs.len());
        std::thread::scope(|scope| {
            let mut handles = Vec::new();
            for _ in 0..threads.min(jobs.len()).max(1) {
                let product_prior_ref=&product_prior;
                let compiled_ref=&compiled;
                let (jobs_ref, cursor_ref, pi_sets_ref, oracles_ref, mp_ref, dex_ref) = (
                    &jobs,
                    &cursor,
                    &pi_sets,
                    &oracles,
                    &pi_star_mp,
                    &dex,
                );
                handles.push(scope.spawn(move || {
                    let mut loc = Vec::new();
                    loop {
                        let k = cursor_ref.fetch_add(1, Ordering::Relaxed);
                        if k >= jobs_ref.len() {
                            break;
                        }
                        let job = &jobs_ref[k];
                        let oracle = &oracles_ref[job.red_idx * n_pi + job.blue_idx];
                        let (blue_pts, turns, solv_hits, r_stats) = play_omniscient_duel_game(
                            dex_ref,
                            &job.battle_seed,
                            &pi_sets_ref[job.blue_idx],
                            &pi_sets_ref[job.red_idx],
                            job.blue_is_p1,
                            job.blue_seed,
                            job.red_seed,
                            if corrected {product_prior_ref.clone()} else {mp_ref.clone()},
                            mp_ref.clone(),
                            oracle,
                            tier,
                            base_iters,
                            same_budget,
                            k_oracle,
                            max_turns,
                            corrected,
                            compiled_ref,
                        );
                        eprintln!("[duel] {} game {}/{} blue={} turns={}",tier.id(),k+1,jobs_ref.len(),blue_pts,turns);
                        loc.push((k, blue_pts, turns, solv_hits, r_stats));
                    }
                    loc
                }));
            }
            for h in handles {
                game_outs.extend(h.join().unwrap());
            }
        });
        game_outs.sort_by_key(|x| x.0);

        let n_g = game_outs.len() as f64;
        let blue_sum: f64 = game_outs.iter().map(|x| x.1).sum();
        let blue_wr = blue_sum / n_g;
        let red_wr = 1.0 - blue_wr;
        let red_gain_over_50 = red_wr - 0.50;
        let avg_turns: f64 = game_outs.iter().map(|x| x.2 as f64).sum::<f64>() / n_g;
        let total_solver_hits: usize = game_outs.iter().map(|x| x.3).sum();
        let tot_searches: u64 = game_outs.iter().map(|x| x.4.searches).sum();
        let tot_probed: u64 = game_outs.iter().map(|x| x.4.probed_searches).sum();
        let tot_mixed: u64 = game_outs.iter().map(|x| x.4.mixed_searches).sum();

        let mut pair_red_scores = Vec::with_capacity(num_pairs);
        for p in 0..num_pairs {
            let r0 = 1.0 - game_outs[2 * p].1;
            let r1 = 1.0 - game_outs[2 * p + 1].1;
            pair_red_scores.push(0.5 * (r0 + r1));
        }
        let var_pair: f64 = pair_red_scores
            .iter()
            .map(|&x| (x - red_wr).powi(2))
            .sum::<f64>()
            / ((num_pairs.saturating_sub(1).max(1)) as f64);
        let se_red = (var_pair / (num_pairs as f64)).sqrt();

        println!(
            "  {:28} | RulesOK={:5} | Red_WR={:6.2}% (±{:4.2}%) | Gain={:+6.2}% | Turns={:4.1} | Mix={}/{}/{} | Solv={:2} ({:.1}s)",
            tier.id(),
            tier.rules_compliant(),
            red_wr * 100.0,
            se_red * 100.0,
            red_gain_over_50 * 100.0,
            avg_turns,
            tot_mixed,
            tot_probed,
            tot_searches,
            total_solver_hits,
            t_tier.elapsed().as_secs_f64()
        );

        tier_reports.push(json!({
            "game_records":game_outs.iter().map(|(k,score,turns,solver,_)|json!({"index":k,"blue_score":score,"turns":turns,"solver_interventions":solver,
                "blue_team":pi_ids[jobs[*k].blue_idx],"red_team":pi_ids[jobs[*k].red_idx],"blue_is_p1":jobs[*k].blue_is_p1,
                "battle_seed":jobs[*k].battle_seed,"blue_seed":jobs[*k].blue_seed,"red_seed":jobs[*k].red_seed})).collect::<Vec<_>>(),
            "tier": tier.id(),
            "description": tier.description(),
            "rules_compliant": tier.rules_compliant(),
            "games": game_outs.len(),
            "red_wr": red_wr,
            "blue_wr": blue_wr,
            "se_red_wr": se_red,
            "paired_red_scores":pair_red_scores,
            "red_wr_95pct_normal_approx":[(red_wr-1.96*se_red).max(0.),(red_wr+1.96*se_red).min(1.)],
            "red_wr_onesided95_hoeffding_upper":(red_wr+(20.0_f64.ln()/(2.*num_pairs as f64)).sqrt()).min(1.),
            "turn_capped_or_natural_ties":game_outs.iter().filter(|x|x.1==0.5).count(),
            "gain_over_shipped_bot": red_gain_over_50,
            "avg_turns": avg_turns,
            "root_searches": tot_searches,
            "probed_searches": tot_probed,
            "mixed_searches": tot_mixed,
            "solver_interventions": total_solver_hits,
            "elapsed_secs": t_tier.elapsed().as_secs_f64(),
        }));
    }

    // 4. Step 3: Certified Exact Endgame Suboptimality Audit (`BoundSolver`) on `1v1` last-mon states!
    println!(
        "\n[Step 3] Collecting {} 1v1 last-mon endgame decision states & running Certified BoundSolver...",
        endgame_states_target
    );
    let mut collected_endgames: Vec<(Battle, usize, usize, usize)> = Vec::new();
    let mut eg_rng = SplitMix64::new(seed ^ 0xE4D6_A3E5_2026_1102);
    let mut seen_state_keys: BTreeSet<u128> = BTreeSet::new();

    for game_idx in 0..160 {
        if collected_endgames.len() >= endgame_states_target {
            break;
        }
        let b_idx = game_idx % n_pi;
        let r_idx = (game_idx * 2 + 1) % n_pi;
        let bseed = eg_rng.battle_seed();
        let mut b = Battle::from_fixture(&dex, &bseed, &pi_sets[b_idx], &pi_sets[r_idx]).unwrap();
        b.set_log_enabled(true);
        let oracle = &oracles[r_idx * n_pi + b_idx];
        let cfg = RmConfig {
            iterations: base_iters,
            rule: SelRule::Ucb,
            c: 0.4,
            ..Default::default()
        };
        let mut ag0 = BlindAgent::new(cfg.clone(), pi_star_mp.clone(), None, eg_rng.next())
            .with_own_preview_policy(oracle.blue_own_pol60.clone())
            .with_opponent_pick_policy(oracle.blue_opp_pol60.clone(), 0.10);
        let mut ag1 = BlindAgent::new(cfg.clone(), pi_star_mp.clone(), None, eg_rng.next())
            .with_own_preview_policy(oracle.red_nash_pol60.clone())
            .with_opponent_pick_policy(oracle.red_opp_shell_pol60.clone(), 0.10);

        let mut grab_this_game = 0usize;
        while b.outcome().is_none() && b.turn < max_turns {
            let c0 = b.legal_choices(&dex, 0);
            let c1 = b.legal_choices(&dex, 1);
            if !matches!(b.request_state, RequestState::TeamPreview)
                && c0.len() >= 2
                && c1.len() >= 2
            {
                let a0 = count_alive_mons(&b, 0);
                let a1 = count_alive_mons(&b, 1);
                if a0 == 1 && a1 == 1 && grab_this_game < 1 {
                    let sk = b.state_key128();
                    if seen_state_keys.insert(sk) {
                        collected_endgames.push((b.clone(), 0, a0, a1));
                        grab_this_game += 1;
                        if collected_endgames.len() >= endgame_states_target {
                            break;
                        }
                    }
                }
            }
            let ch0 = if c0.is_empty() {
                None
            } else {
                Some(ag0.choose(&b, &dex, 0, &c0))
            };
            let ch1 = if c1.is_empty() {
                None
            } else {
                Some(ag1.choose(&b, &dex, 1, &c1))
            };
            b.apply_choices(&dex, [ch0, ch1]).unwrap();
        }
    }

    let eg_cursor = AtomicUsize::new(0);
    let mut eg_records: Vec<Value> = Vec::with_capacity(collected_endgames.len());
    std::thread::scope(|scope| {
        let mut handles = Vec::new();
        for _ in 0..threads.min(collected_endgames.len()).max(1) {
            let (eg_ref, cursor_ref, dex_ref, mp_ref) =
                (&collected_endgames, &eg_cursor, &dex, &pi_star_mp);
            handles.push(scope.spawn(move || {
                let mut loc = Vec::new();
                loop {
                    let idx = cursor_ref.fetch_add(1, Ordering::Relaxed);
                    if idx >= eg_ref.len() {
                        break;
                    }
                    let (ref b_orig, side, a0, a1) = eg_ref[idx];
                    let mut battle = b_orig.clone();
                    let choices = battle.legal_choices(dex_ref, side);

                    // 1. Evaluate policies first so we can pass them to `solve_policy_losses`!
                    let k_seeds = 4usize;
                    let mut blind400_pol = vec![0.0f64; choices.len()];
                    let mut blind400_mix_pol = vec![0.0f64; choices.len()];
                    let mut skuct3200_pol = vec![0.0f64; choices.len()];
                    for s_i in 0..k_seeds {
                        let s_seed = 0x900D_BEEF_0000_0000_u64
                            ^ ((idx as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15))
                            ^ (s_i as u64);
                        let cfg400 = RmConfig {
                            iterations: base_iters,
                            rule: SelRule::Ucb,
                            c: 0.4,
                            ..Default::default()
                        };
                        let mut ba = BlindAgent::new(cfg400.clone(), mp_ref.clone(), None, s_seed);
                        let p_ba = ba.root_policy(&battle, dex_ref, side, &choices);
                        for (j, &v) in p_ba.iter().enumerate() {
                            blind400_pol[j] += v / (k_seeds as f64);
                        }

                        let mut ba_mix =
                            BlindAgent::new(cfg400, mp_ref.clone(), None, s_seed ^ 0x5555)
                                .with_adaptive_mixture((base_iters / 4).max(40));
                        let p_mix = ba_mix.root_policy(&battle, dex_ref, side, &choices);
                        for (j, &v) in p_mix.iter().enumerate() {
                            blind400_mix_pol[j] += v / (k_seeds as f64);
                        }

                        let cfg3200 = RmConfig {
                            iterations: base_iters * 8,
                            rule: SelRule::Ucb,
                            c: 0.4,
                            ..Default::default()
                        };
                        let mut sk = SkuctSearch::new(&battle, dex_ref, cfg3200, s_seed ^ 0xAAAA);
                        sk.step(dex_ref, base_iters * 8);
                        if let Some(best_c) = sk.best(side) {
                            if let Some(pos) = choices.iter().position(|&c| c == best_c) {
                                skuct3200_pol[pos] += 1.0 / (k_seeds as f64);
                            }
                        }
                    }

                    // 2. Run `BoundSolver::solve_policy_losses` with directed refinement (`600ms` budget)
                    let mut solver = BoundSolver::new(
                        dex_ref,
                        BoundConfig {
                            time_budget: Some(Duration::from_millis(600)),
                            work_budget: 150_000,
                            node_budget: 120_000,
                            trial_depth: 24,
                            cell_cap: 4096,
                            resource_closure_nodes: Some(9000),
                            eps: 0.005,
                            ..Default::default()
                        },
                    );
                    let targets = vec![
                        (side, blind400_pol.clone()),
                        (side, blind400_mix_pol.clone()),
                        (side, skuct3200_pol.clone()),
                    ];
                    let rep = solver.solve_policy_losses(&battle, &targets, 0.005);
                    let Some(mat) = solver.root_payoffs(&battle) else {
                        continue;
                    };
                    if mat.actions[side].len() != choices.len() {
                        continue;
                    }

                    let loss_blind400 = mat.policy_loss(side, &blind400_pol);
                    let loss_blind400_mix = mat.policy_loss(side, &blind400_mix_pol);
                    let loss_skuct3200 = mat.policy_loss(side, &skuct3200_pol);

                    loc.push((
                        idx,
                        json!({
                            "idx": idx,
                            "turn": battle.turn,
                            "alive": [a0, a1],
                            "value_bounds": [mat.value.lo, mat.value.hi],
                            "value_width": mat.value.width(),
                            "solver_runs": rep.runs,
                            "solver_stop": format!("{:?}", rep.stop),
                            "blind400_loss_lo": loss_blind400.lo,
                            "blind400_loss_hi": loss_blind400.hi,
                            "blind400_loss_mid": loss_blind400.mid(),
                            "blind400_mix_loss_lo": loss_blind400_mix.lo,
                            "blind400_mix_loss_hi": loss_blind400_mix.hi,
                            "blind400_mix_loss_mid": loss_blind400_mix.mid(),
                            "skuct3200_loss_lo": loss_skuct3200.lo,
                            "skuct3200_loss_hi": loss_skuct3200.hi,
                            "skuct3200_loss_mid": loss_skuct3200.mid(),
                        }),
                    ));
                }
                loc
            }));
        }
        let mut indexed = Vec::new();
        for h in handles {
            indexed.extend(h.join().unwrap());
        }
        indexed.sort_by_key(|x| x.0);
        for (_, v) in indexed {
            eg_records.push(v);
        }
    });

    let n_eg = eg_records.len().max(1) as f64;
    let avg_width: f64 = eg_records
        .iter()
        .filter_map(|r| r["value_width"].as_f64())
        .sum::<f64>()
        / n_eg;
    let avg_blind400_lo: f64 = eg_records
        .iter()
        .filter_map(|r| r["blind400_loss_lo"].as_f64())
        .sum::<f64>()
        / n_eg;
    let avg_blind400_hi: f64 = eg_records
        .iter()
        .filter_map(|r| r["blind400_loss_hi"].as_f64())
        .sum::<f64>()
        / n_eg;
    let avg_blind400_mid: f64 = eg_records
        .iter()
        .filter_map(|r| r["blind400_loss_mid"].as_f64())
        .sum::<f64>()
        / n_eg;
    let avg_mix_lo: f64 = eg_records
        .iter()
        .filter_map(|r| r["blind400_mix_loss_lo"].as_f64())
        .sum::<f64>()
        / n_eg;
    let avg_mix_mid: f64 = eg_records
        .iter()
        .filter_map(|r| r["blind400_mix_loss_mid"].as_f64())
        .sum::<f64>()
        / n_eg;
    let avg_skuct3200_lo: f64 = eg_records
        .iter()
        .filter_map(|r| r["skuct3200_loss_lo"].as_f64())
        .sum::<f64>()
        / n_eg;
    let avg_skuct3200_mid: f64 = eg_records
        .iter()
        .filter_map(|r| r["skuct3200_loss_mid"].as_f64())
        .sum::<f64>()
        / n_eg;

    // Also compute stats on tightly-resolved states (`value_width <= 0.10`)
    let tight_recs: Vec<&Value> = eg_records
        .iter()
        .filter(|r| r["value_width"].as_f64().unwrap_or(1.0) <= 0.10)
        .collect();
    let n_tight = tight_recs.len().max(1) as f64;
    let tight_blind400_mid: f64 = tight_recs
        .iter()
        .filter_map(|r| r["blind400_loss_mid"].as_f64())
        .sum::<f64>()
        / n_tight;
    let tight_mix_mid: f64 = tight_recs
        .iter()
        .filter_map(|r| r["blind400_mix_loss_mid"].as_f64())
        .sum::<f64>()
        / n_tight;
    let tight_skuct3200_mid: f64 = tight_recs
        .iter()
        .filter_map(|r| r["skuct3200_loss_mid"].as_f64())
        .sum::<f64>()
        / n_tight;

    println!(
        "  [Endgame BoundSolver Summary ({} 1v1 states, {} tightly closed <=0.10)] AvgWidth={:.4} | Blind400_Loss=[{:.2}%, {:.2}%] (TightMid={:.2}%) | Blind400+Mix_TightMid={:.2}% | Skuct3200_TightMid={:.2}%",
        eg_records.len(),
        tight_recs.len(),
        avg_width,
        avg_blind400_lo * 100.0,
        avg_blind400_hi * 100.0,
        tight_blind400_mid * 100.0,
        tight_mix_mid * 100.0,
        tight_skuct3200_mid * 100.0
    );

    if let Some(parent) = out_path.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    let report = json!({
        "corrected_product":corrected,"seed":seed,"max_turns":max_turns,"k_oracle":k_oracle,"same_budget":same_budget,
        "build":nc2000_bot::compiled_preview::build_fingerprint(),
        "blue_preview":"regenerated compiled choices[u32(seed)%64] when corrected_product=true",
        "blue_api":"ProtocolAgent with trusted PlayerChannel when corrected_product=true",
        "matchup_sampling":if corrected {"independent weighted draws per mirrored block"}else{"historical complementary QMC"},
        "blue_prior":"unmodified frozen production prior when corrected_product=true",
        "elapsed_secs": t_start.elapsed().as_secs_f64(),
        "base_iters": base_iters,
        "pairs_per_tier": num_pairs,
        "games_per_tier": num_pairs * 2,
        "duel_tiers": tier_reports,
        "endgame_bound_solver_summary": {
            "states_evaluated": eg_records.len(),
            "tight_states_count": tight_recs.len(),
            "avg_root_value_width": avg_width,
            "blind400_certified_loss_lo": avg_blind400_lo,
            "blind400_certified_loss_hi": avg_blind400_hi,
            "blind400_certified_loss_mid": avg_blind400_mid,
            "blind400_tight_loss_mid": tight_blind400_mid,
            "blind400_adaptive_mix_loss_lo": avg_mix_lo,
            "blind400_adaptive_mix_loss_mid": avg_mix_mid,
            "blind400_adaptive_mix_tight_loss_mid": tight_mix_mid,
            "skuct3200_full_info_loss_lo": avg_skuct3200_lo,
            "skuct3200_full_info_loss_mid": avg_skuct3200_mid,
            "skuct3200_full_info_tight_loss_mid": tight_skuct3200_mid,
            "records": eg_records,
        }
    });
    std::fs::write(&out_path, serde_json::to_string_pretty(&report).unwrap()).unwrap();
    println!(
        "\nSaved full omniscient audit report to {}",
        out_path.display()
    );
}
