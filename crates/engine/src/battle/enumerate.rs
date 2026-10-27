//! Exhaustive chance-node enumeration of one decision step (M17e).
//!
//! Replaces the seeded LCG with the Oracle mode of `BattleRng` and walks
//! every outcome-class combination of one `apply_choices` step by
//! recursive DFS over the recorded draw trace: each engine run executes
//! the step to completion under a script prefix (unscripted draws take the
//! first non-empty class); the first draw past the prefix is the branch
//! point, and each of its classes is explored in turn.
//!
//! Damage rolls ("droll" draws, 39 classes) are range-merged: if the
//! complete subtrees under the endpoint rolls of a range coincide
//! (leaf-by-leaf: same state_key, same downstream draw trace), damage's
//! monotonicity in the roll pins every interior roll to the same subtree,
//! so the whole range collapses into the endpoint's leaves at the range's
//! total mass — 2 engine explorations instead of 39 wherever overkill or
//! damage-irrelevance makes rolls indistinguishable (owner-approved
//! merge criterion, 2026-07-21). Non-coinciding ranges bisect.
//!
//! Leaf probabilities are products of per-draw exact rationals
//! (class-count / 2^32) accumulated in f64 — deterministic, no seed noise.

use std::cell::Cell;

use crate::dex::Dex;
use crate::prng::{BattleRng, DamageRollMode, Draw, Oracle};
use crate::state::Battle;

use super::search::SearchChoice;

/// One chance outcome of a decision step. `battle` has advanced to the next
/// request point (or ended); its RNG is a spent Oracle — `reseed` it before
/// any further seeded play.
pub struct ChanceLeaf {
    pub battle: Battle,
    pub prob: f64,
    /// Chance draws consumed along this path.
    pub draws: usize,
}

/// Enumeration result with honest work accounting: `runs` counts actual
/// engine executions (probes + endpoint evaluations included), which is
/// what budgets and benchmarks must use — resolved leaf count is not work.
pub struct StepEnum {
    pub leaves: Vec<ChanceLeaf>,
    pub runs: usize,
    /// Diagnostic observations across scripted engine runs. These are not
    /// unique battle events: probes deliberately replay the same prefix.
    pub damage: DamageEnumStats,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct DamageEnumStats {
    pub exact_draws: usize,
    pub abstract_draws: usize,
    pub offered_classes: usize,
    pub drain_recoil_draws: usize,
    pub multihit_draws: usize,
    pub substitute_draws: usize,
    pub counter_bide_draws: usize,
    pub heal_draws: usize,
}

#[derive(Clone)]
struct LeafRec {
    battle: Battle,
    key: Cell<Option<u128>>,
    prob: f64,
    trace: Vec<Draw>,
}

impl LeafRec {
    #[inline]
    fn key(&self) -> u128 {
        match self.key.get() {
            Some(k) => k,
            None => {
                let k = self.battle.state_key128();
                self.key.set(Some(k));
                k
            }
        }
    }
}

/// One scripted engine execution of a decision step: applies `choices` to a
/// clone of `base` under the Oracle following `script` (draws past the
/// script take the first non-empty class) and returns the advanced battle
/// plus the full draw trace. The lazy bound solver's expansion atom (e-5);
/// the eager enumerator below remains as its test oracle.
pub fn run_scripted(
    dex: &Dex,
    base: &Battle,
    choices: [Option<SearchChoice>; 2],
    script: &[usize],
) -> (Battle, Vec<Draw>) {
    run_scripted_with_damage_mode(dex, base, choices, script, DamageRollMode::Exact)
}

/// Same as `run_scripted`, reusing `out`'s allocations via `clone_from`.
pub fn run_scripted_clone_into(
    dex: &Dex,
    base: &Battle,
    out: &mut Battle,
    choices: [Option<SearchChoice>; 2],
    script: &[usize],
) -> Vec<Draw> {
    out.clone_from(base);
    out.set_log_enabled(false);
    out.prng = BattleRng::enumerating_with_damage_mode(script.to_vec(), DamageRollMode::Exact);
    out.apply_choices(dex, choices).expect("run_scripted: choices must be legal");
    let oracle = out.prng.oracle.take().expect("oracle survives the step");
    let Oracle { trace, .. } = *oracle;
    trace
}

/// Experimental counterpart of `run_scripted`. The production exact path
/// above always fixes `Exact`; callers must opt into damage abstraction.
pub fn run_scripted_with_damage_mode(
    dex: &Dex,
    base: &Battle,
    choices: [Option<SearchChoice>; 2],
    script: &[usize],
    damage_mode: DamageRollMode,
) -> (Battle, Vec<Draw>) {
    let mut b = base.clone();
    b.set_log_enabled(false);
    b.prng = BattleRng::enumerating_with_damage_mode(script.to_vec(), damage_mode);
    b.apply_choices(dex, choices).expect("run_scripted: choices must be legal");
    let oracle = b.prng.oracle.take().expect("oracle survives the step");
    let Oracle { trace, .. } = *oracle;
    (b, trace)
}

struct Ctx<'a> {
    dex: &'a Dex,
    base: &'a Battle,
    choices: [Option<SearchChoice>; 2],
    damage_mode: DamageRollMode,
    cap: usize,
    runs: usize,
    damage: DamageEnumStats,
}

impl Ctx<'_> {
    fn run(&mut self, script: &[usize]) -> Option<(Battle, Vec<Draw>)> {
        if self.runs >= self.cap {
            return None;
        }
        self.runs += 1;
        let result = run_scripted_with_damage_mode(
            self.dex,
            self.base,
            self.choices,
            script,
            self.damage_mode,
        );
        for draw in &result.1 {
            match draw.label {
                "dmgvar" => self.damage.exact_draws += 1,
                "dmgvar-drain-recoil" => {
                    self.damage.exact_draws += 1;
                    self.damage.drain_recoil_draws += 1;
                }
                "dmgvar-multihit" => {
                    self.damage.exact_draws += 1;
                    self.damage.multihit_draws += 1;
                }
                "dmgvar-substitute" => {
                    self.damage.exact_draws += 1;
                    self.damage.substitute_draws += 1;
                }
                "dmgvar-counter-bide" => {
                    self.damage.exact_draws += 1;
                    self.damage.counter_bide_draws += 1;
                }
                "dmgvar-heal" => {
                    self.damage.exact_draws += 1;
                    self.damage.heal_draws += 1;
                }
                "dmgabs" => self.damage.abstract_draws += 1,
                _ => continue,
            }
            self.damage.offered_classes += draw.counts.len();
        }
        Some(result)
    }
}

fn leaf(b: Battle, trace: Vec<Draw>) -> LeafRec {
    let prob = trace.iter().map(Draw::prob).product();
    LeafRec {
        battle: b,
        key: Cell::new(None),
        prob,
        trace,
    }
}

/// All leaves of the subtree whose draw choices are fixed by `script`.
fn subtree(ctx: &mut Ctx, script: &mut Vec<usize>) -> Option<Vec<LeafRec>> {
    let (b, trace) = ctx.run(script)?;
    subtree_from_run(ctx, script, b, trace)
}

fn subtree_from_run(
    ctx: &mut Ctx,
    script: &mut Vec<usize>,
    b: Battle,
    trace: Vec<Draw>,
) -> Option<Vec<LeafRec>> {
    if trace.len() <= script.len() {
        return Some(vec![leaf(b, trace)]);
    }
    let p = script.len();
    let d = &trace[p];
    let mut out = Vec::new();
    if d.label.starts_with("dmgvar") {
        let chosen = d.chosen;
        let last = d.counts.len() - 1;
        let mut cache: [Option<Vec<LeafRec>>; 40] = std::array::from_fn(|_| None);
        script.push(chosen);
        let known = subtree_from_run(ctx, script, b, trace)?;
        script.pop();
        cache[chosen] = Some(known);
        droll_ranges(ctx, script, p, 0, last, &mut cache, &mut out)?;
    } else if d.counts.len() <= 64 {
        let chosen = d.chosen;
        let mut mask = 0u64;
        for (c, &cnt) in d.counts.iter().enumerate() {
            if cnt != 0 {
                mask |= 1u64 << c;
            }
        }
        let mut known = Some((b, trace));
        while mask != 0 {
            let c = mask.trailing_zeros() as usize;
            mask &= mask - 1;
            script.push(c);
            let mut leaves = if c == chosen {
                let (b, trace) = known.take().unwrap();
                subtree_from_run(ctx, script, b, trace)?
            } else {
                subtree(ctx, script)?
            };
            out.append(&mut leaves);
            script.pop();
        }
    } else {
        let counts = d.counts.clone();
        let chosen = d.chosen;
        let mut known = Some((b, trace));
        for (c, &cnt) in counts.iter().enumerate() {
            if cnt == 0 {
                continue;
            }
            script.push(c);
            let mut leaves = if c == chosen {
                let (b, trace) = known.take().unwrap();
                subtree_from_run(ctx, script, b, trace)?
            } else {
                subtree(ctx, script)?
            };
            out.append(&mut leaves);
            script.pop();
        }
    }
    Some(out)
}

/// Two subtrees coincide iff they have the same leaves in DFS order: equal
/// state and equal downstream chance structure past the droll at `p`.
fn coincide(a: &[LeafRec], b: &[LeafRec], p: usize) -> bool {
    a.len() == b.len()
        && a.iter().zip(b).all(|(x, y)| {
            x.trace.len() == y.trace.len()
                && x.trace[p + 1..].iter().zip(&y.trace[p + 1..]).all(|(dx, dy)| {
                    dx.label == dy.label && dx.chosen == dy.chosen && dx.counts == dy.counts
                })
                && x.key() == y.key()
        })
}

#[allow(clippy::too_many_arguments)]
fn droll_ranges(
    ctx: &mut Ctx,
    script: &mut Vec<usize>,
    p: usize,
    a: usize,
    b: usize,
    cache: &mut [Option<Vec<LeafRec>>; 40],
    out: &mut Vec<LeafRec>,
) -> Option<()> {
    if cache[a].is_none() {
        script.push(a);
        cache[a] = Some(subtree(ctx, script)?);
        script.pop();
    }
    if a == b {
        let mut sa = cache[a].take().unwrap();
        out.append(&mut sa);
        return Some(());
    }
    if cache[b].is_none() {
        script.push(b);
        cache[b] = Some(subtree(ctx, script)?);
        script.pop();
    }
    if coincide(cache[a].as_ref().unwrap(), cache[b].as_ref().unwrap(), p) {
        // Merge [a, b]: reweight position p from class a's mass to the
        // range's total mass (per-class counts differ by ±1, so divide the
        // old factor out exactly).
        let mut sa = cache[a].take().unwrap();
        cache[b] = None;
        let scale = if let Some(first) = sa.first() {
            let counts = &first.trace[p].counts;
            let range_mass: u64 = counts[a..=b].iter().sum();
            range_mass as f64 / counts[a] as f64
        } else {
            1.0
        };
        for l in &mut sa {
            l.prob *= scale;
        }
        out.append(&mut sa);
        return Some(());
    }
    let m = (a + b) / 2;
    droll_ranges(ctx, script, p, a, m, cache, out)?;
    droll_ranges(ctx, script, p, m + 1, b, cache, out)
}

/// Enumerate every chance outcome of applying `choices` (legal, from
/// `legal_choices`) to `base`, which must sit at a decision point. Returns
/// `None` if more than `cap` engine runs would be needed (e.g. Metronome).
///
/// Panics if the choices are rejected — feeding legal choices is the
/// caller's contract.
pub fn enumerate_step(
    dex: &Dex,
    base: &Battle,
    choices: [Option<SearchChoice>; 2],
    cap: usize,
) -> Option<StepEnum> {
    enumerate_step_with_damage_mode(dex, base, choices, cap, DamageRollMode::Exact)
}

/// Enumerate one decision step while quotienting damage rolls according to
/// `damage_mode`. This is approximate unless `damage_mode == Exact`.
pub fn enumerate_step_with_damage_mode(
    dex: &Dex,
    base: &Battle,
    choices: [Option<SearchChoice>; 2],
    cap: usize,
    damage_mode: DamageRollMode,
) -> Option<StepEnum> {
    let clean_base = if base.log_enabled || !base.log.is_empty() {
        let mut b = base.clone();
        b.set_log_enabled(false);
        Some(b)
    } else {
        None
    };
    let base_ref = clean_base.as_ref().unwrap_or(base);
    let mut ctx = Ctx {
        dex,
        base: base_ref,
        choices,
        damage_mode,
        cap,
        runs: 0,
        damage: DamageEnumStats::default(),
    };
    let mut script = Vec::new();
    let recs = subtree(&mut ctx, &mut script)?;
    Some(StepEnum {
        leaves: recs
            .into_iter()
            .map(|l| ChanceLeaf {
                battle: l.battle,
                prob: l.prob,
                draws: l.trace.len(),
            })
            .collect(),
        runs: ctx.runs,
        damage: ctx.damage,
    })
}
