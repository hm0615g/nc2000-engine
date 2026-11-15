//! World-sampling budget (`MAX_SUBSAMPLED_WORLDS` / `BlindSearch::with_max_worlds`)
//! sensitivity and equal-time duel probe.
//!
//! Strictly single-line JSON stdout (M2M output principle).

use std::collections::HashSet;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use conformance::fixture::repo_root;
use conformance::load_dex;
use nc2000_bot::agent::Agent;
use nc2000_bot::belief::{Belief, FallbackPolicy};
use nc2000_bot::blind::{BlindAgent, BlindSearch, OpenAgent};
use nc2000_bot::observe::Observer;
use nc2000_bot::player_state::{submitted_team, LocalPlayer};
use nc2000_bot::preview::{load_meta_pool, team_sig, MetaPool};
use nc2000_bot::prior::BeliefPrior;
use nc2000_bot::rng::SplitMix64;
use nc2000_bot::runner::{play_game, GameResult};
use nc2000_bot::smmcts::{RmConfig, SelRule};
use nc2000_bot::teamgen::{to_sets, TeamGen};
use nc2000_engine::battle::{Outcome, PokemonSet, SearchChoice};
use nc2000_engine::dex::Dex;
use nc2000_engine::state::Battle;
use nc2000_engine::validate::{validate_team, Learnsets};
use serde_json::{json, Value};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Sensitivity,
    Duel,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum OpponentKind {
    Blind24,
    Open,
}

struct Cli {
    mode: Mode,
    vs: OpponentKind,
    k_list: Vec<usize>,
    k_a: usize,
    k_b: usize,
    iters: u32,
    iters_a: Option<u32>,
    iters_b: Option<u32>,
    collect_games: usize,
    positions: usize,
    pairs: usize,
    threads: usize,
    seed: u64,
    max_turns: u16,
    mutations: usize,
    prior_path: Option<std::path::PathBuf>,
    fallback_only: bool,
    pool_mode: PoolMode,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum PoolMode {
    Offpool,
    BothOffpool,
    Catalog,
}

fn arg_val(args: &[String], key: &str) -> Option<String> {
    args.iter()
        .position(|a| a == key)
        .and_then(|i| args.get(i + 1))
        .cloned()
}

fn has_flag(args: &[String], key: &str) -> bool {
    args.iter().any(|a| a == key)
}

fn parse_cli() -> Cli {
    let args: Vec<String> = std::env::args().collect();
    let mode = match arg_val(&args, "--mode").as_deref() {
        Some("duel") => Mode::Duel,
        _ => Mode::Sensitivity,
    };
    let vs = match arg_val(&args, "--vs").as_deref() {
        Some("open") => OpponentKind::Open,
        _ => OpponentKind::Blind24,
    };
    let k_list = arg_val(&args, "--ks")
        .map(|s| {
            s.split(',')
                .filter_map(|p| p.trim().parse::<usize>().ok())
                .filter(|&k| k >= 1)
                .collect::<Vec<_>>()
        })
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| vec![12, 24, 48, 64, 96]);
    let k_a = arg_val(&args, "--k-a")
        .and_then(|s| s.parse().ok())
        .unwrap_or(48);
    let k_b = arg_val(&args, "--k-b")
        .and_then(|s| s.parse().ok())
        .unwrap_or(24);
    let iters = arg_val(&args, "--iters")
        .and_then(|s| s.parse().ok())
        .unwrap_or(27_000);
    let iters_a = arg_val(&args, "--iters-a").and_then(|s| s.parse().ok());
    let iters_b = arg_val(&args, "--iters-b").and_then(|s| s.parse().ok());
    let collect_games = arg_val(&args, "--collect-games")
        .and_then(|s| s.parse().ok())
        .unwrap_or(24);
    let positions = arg_val(&args, "--positions")
        .and_then(|s| s.parse().ok())
        .unwrap_or(64);
    let pairs = arg_val(&args, "--pairs")
        .and_then(|s| s.parse().ok())
        .unwrap_or(64);
    let default_threads = std::thread::available_parallelism()
        .map_or(1, usize::from)
        .min(16);
    let threads = arg_val(&args, "--threads")
        .and_then(|s| s.parse().ok())
        .unwrap_or(default_threads)
        .max(1);
    let seed = arg_val(&args, "--seed")
        .and_then(|s| s.parse().ok())
        .unwrap_or(0x574F_524C_445F_0001);
    let max_turns = arg_val(&args, "--max-turns")
        .and_then(|s| s.parse().ok())
        .unwrap_or(200);
    let mutations = arg_val(&args, "--mutations")
        .and_then(|s| s.parse().ok())
        .unwrap_or(48);
    let prior_path = arg_val(&args, "--prior").map(std::path::PathBuf::from);
    let fallback_only = !has_flag(&args, "--all-positions");
    let pool_mode = match arg_val(&args, "--pool").as_deref() {
        Some("both-offpool") => PoolMode::BothOffpool,
        Some("catalog") => PoolMode::Catalog,
        _ => {
            if has_flag(&args, "--catalog") {
                PoolMode::Catalog
            } else {
                PoolMode::Offpool
            }
        }
    };

    Cli {
        mode,
        vs,
        k_list,
        k_a,
        k_b,
        iters,
        iters_a,
        iters_b,
        collect_games,
        positions,
        pairs,
        threads,
        seed,
        max_turns,
        mutations,
        prior_path,
        fallback_only,
        pool_mode,
    }
}

fn production_cfg(iters: u32) -> RmConfig {
    let profile: Value =
        serde_json::from_str(include_str!("../../../data/search-profiles.json")).unwrap();
    RmConfig {
        iterations: iters,
        rule: SelRule::Ucb,
        c: profile["blind"]["c"].as_f64().unwrap(),
        ..Default::default()
    }
}

fn load_legal_rentals(root: &Path, dex: &Dex, ls: &Learnsets) -> Vec<Vec<PokemonSet>> {
    let text = std::fs::read_to_string(root.join("data/community-rentals-v0/teams.json")).unwrap();
    let v: Value = serde_json::from_str(&text).unwrap();
    let mut out = Vec::new();
    for t in v["teams"].as_array().unwrap() {
        let Some(sets_val) = t["sets"].as_array() else {
            continue;
        };
        let team_json = Value::Array(sets_val.clone()).to_string();
        if validate_team(dex, ls, &team_json)["ok"] != Value::Bool(true) {
            continue;
        }
        if let Ok(sets) = to_sets(sets_val) {
            out.push(sets);
        }
    }
    out
}

fn load_offpool_teams(
    root: &Path,
    dex: &Dex,
    prior_pool: &MetaPool,
    mutation_count: usize,
    seed: u64,
) -> Vec<Vec<PokemonSet>> {
    let ls_text = std::fs::read_to_string(root.join("data/learnsets-gen2.json")).unwrap();
    let ls = Learnsets::from_json(&ls_text).unwrap();
    let pool_text = std::fs::read_to_string(root.join("data/belief-pool-v3/belief-pool.json")).unwrap();
    let gen = TeamGen::new(dex, &ls_text, &pool_text).unwrap();

    let prior_sigs: HashSet<_> = prior_pool
        .teams
        .iter()
        .map(|t| team_sig(dex, &t.sets))
        .collect();
    let mut seen_sigs = prior_sigs;
    let mut out = Vec::new();

    let v2 = load_meta_pool(&root.join("data/team-pool-v2/team-pool.json"));
    for t in &v2.teams {
        let sig = team_sig(dex, &t.sets);
        if seen_sigs.insert(sig) {
            out.push(t.sets.clone());
        }
    }

    for sets in load_legal_rentals(root, dex, &ls) {
        let sig = team_sig(dex, &sets);
        if seen_sigs.insert(sig) {
            out.push(sets);
        }
    }

    let mut rng = SplitMix64::new(seed ^ 0x517C_C1B7_2722_0A95);
    for m_idx in 0..mutation_count {
        let seed_idx = (m_idx * 17) % prior_pool.teams.len();
        let mut cur = gen.canonize(dex, &gen.team_json(seed_idx)).unwrap();
        let n_steps = 1 + (m_idx % 4);
        let mut accepted = None;
        for step in 0..24usize {
            let Some(prop) = gen.propose_valid(dex, &cur, &mut rng, 64) else {
                break;
            };
            cur = prop.team;
            if step + 1 >= n_steps {
                if let Ok(sets) = to_sets(&cur) {
                    let sig = team_sig(dex, &sets);
                    if seen_sigs.insert(sig) {
                        accepted = Some(sets);
                        break;
                    }
                }
            }
        }
        if let Some(sets) = accepted {
            out.push(sets);
        }
    }

    out
}

struct MaxWorldsBlindAgent {
    cfg: RmConfig,
    pool: Arc<MetaPool>,
    prior: Option<Arc<BeliefPrior>>,
    max_worlds: usize,
    rng: SplitMix64,
    local: Option<LocalPlayer>,
    observer: Option<Observer>,
    belief: Option<Belief>,
    side: usize,
    last_turn: u16,
    think_ns: u64,
    moves: u64,
}

impl MaxWorldsBlindAgent {
    fn new(
        cfg: RmConfig,
        pool: Arc<MetaPool>,
        prior: Option<Arc<BeliefPrior>>,
        max_worlds: usize,
        seed: u64,
    ) -> Self {
        Self {
            cfg,
            pool,
            prior,
            max_worlds: max_worlds.max(1),
            rng: SplitMix64::new(seed),
            local: None,
            observer: None,
            belief: None,
            side: usize::MAX,
            last_turn: u16::MAX,
            think_ns: 0,
            moves: 0,
        }
    }
}

impl Agent for MaxWorldsBlindAgent {
    fn name(&self) -> String {
        format!(
            "blind-w{}:{}:{}",
            self.max_worlds, self.cfg.iterations, self.cfg.c
        )
    }

    fn requires_player_log(&self) -> bool {
        true
    }

    fn choose(
        &mut self,
        battle: &Battle,
        dex: &Dex,
        side: usize,
        choices: &[SearchChoice],
    ) -> SearchChoice {
        let is_preview = matches!(choices[0], SearchChoice::Team(_));
        let stale = match &self.local {
            None => true,
            Some(lp) => {
                self.side != side
                    || is_preview
                    || battle.turn < self.last_turn
                    || !lp.is_compatible(battle)
            }
        };
        if stale {
            let own_sets = submitted_team(battle, dex, side);
            let (lp, obs) = LocalPlayer::new_with_own_sets(battle, dex, side, own_sets)
                .expect("player observation failed");
            let mut b = Belief::with_fallback_policy(
                dex,
                &self.pool,
                &obs,
                FallbackPolicy::Bayesian,
            );
            if let Some(p) = &self.prior {
                b.set_prior(p.clone());
            }
            self.local = Some(lp);
            self.observer = Some(obs);
            self.belief = Some(b);
            self.side = side;
        }
        let lp = self.local.as_mut().unwrap();
        let obs = self.observer.as_mut().unwrap();
        let belief = self.belief.as_mut().unwrap();
        self.last_turn = battle.turn;
        lp.observe(battle, dex, obs).expect("observe failed");
        belief.sync(dex, obs);
        if choices.len() == 1 {
            return choices[0];
        }
        let pos = lp
            .position(dex, belief, obs, &mut self.rng)
            .expect("position reconstruction failed");
        let opp_prior = belief.has_preview_prior(obs);
        let search_seed = self.rng.next();
        let t0 = Instant::now();
        let mut bs = BlindSearch::from_observation(&pos, dex, self.cfg.clone(), side, search_seed)
            .with_max_worlds(self.max_worlds);
        bs.prune_preview_aliases(dex);
        if opp_prior {
            bs.set_opponent_preview_prior(true);
        }
        for _ in 0..self.cfg.iterations {
            bs.step_one(dex, belief, obs);
            if bs.should_early_stop() {
                break;
            }
        }
        let picked = bs.best().expect("non-empty choice list");
        self.think_ns += t0.elapsed().as_nanos() as u64;
        self.moves += 1;
        picked
    }
}

struct Snapshot {
    battle: Battle,
    own_sets: Vec<PokemonSet>,
    side: usize,
    is_fallback: bool,
    fallback_cands: usize,
    alive_cands: usize,
    pos_seed: u64,
    seed_a: u64,
    seed_b: u64,
}

fn collect_snapshots(
    cli: &Cli,
    dex: &Arc<Dex>,
    prior: &Arc<MetaPool>,
    belief_prior: Option<&Arc<BeliefPrior>>,
    teams: &[Vec<PokemonSet>],
) -> Vec<Snapshot> {
    let fast_cfg = production_cfg(2_000);
    let mut out = Vec::new();
    let mut rng = SplitMix64::new(cli.seed ^ 0xA5A5_1234_9876_5432);

    for g_idx in 0..cli.collect_games {
        let opp_sets = &teams[g_idx % teams.len()];
        let own_sets = &prior.teams[(g_idx * 19 + 3) % prior.teams.len()].sets;
        let bseed = rng.battle_seed();
        let Ok(mut battle) = Battle::from_fixture(dex, &bseed, own_sets, opp_sets) else {
            continue;
        };
        battle.set_log_enabled(true);
        let mut blind = BlindAgent::new_with_fallback_policy(
            fast_cfg.clone(),
            prior.clone(),
            None,
            rng.next(),
            FallbackPolicy::Bayesian,
        );
        if let Some(bp) = belief_prior {
            blind.set_belief_prior(bp.clone());
        }
        let mut open = OpenAgent::new(fast_cfg.clone(), None, rng.next());

        let (mut lp, mut obs) = LocalPlayer::new_with_own_sets(&battle, dex, 0, own_sets.clone())
            .expect("lp init failed");
        let mut belief =
            Belief::with_fallback_policy(dex, prior, &obs, FallbackPolicy::Bayesian);
        if let Some(bp) = belief_prior {
            belief.set_prior(bp.clone());
        }

        for _ in 0..cli.max_turns {
            if battle.ended {
                break;
            }
            let cs0 = battle.legal_choices(dex, 0);
            let cs1 = battle.legal_choices(dex, 1);
            if !cs0.is_empty() && !matches!(cs0[0], SearchChoice::Team(_)) && cs0.len() >= 2 {
                if lp.observe(&battle, dex, &mut obs).is_ok() {
                    belief.sync(dex, &obs);
                    let is_fb = belief.is_fallback();
                    if !cli.fallback_only || is_fb {
                        out.push(Snapshot {
                            battle: battle.clone(),
                            own_sets: own_sets.clone(),
                            side: 0,
                            is_fallback: is_fb,
                            fallback_cands: belief.fallback_cand_count(),
                            alive_cands: belief.candidate_count(),
                            pos_seed: rng.next(),
                            seed_a: rng.next(),
                            seed_b: rng.next(),
                        });
                    }
                }
            }
            let p0 = if cs0.is_empty() {
                None
            } else {
                Some(blind.choose(&battle, dex, 0, &cs0))
            };
            let p1 = if cs1.is_empty() {
                None
            } else {
                Some(open.choose(&battle, dex, 1, &cs1))
            };
            if p0.is_none() && p1.is_none() {
                break;
            }
            if battle.apply_choices(dex, [p0, p1]).is_err() {
                break;
            }
        }
        if out.len() >= cli.positions * 3 {
            break;
        }
    }

    // Evenly subsample up to cli.positions
    if out.len() <= cli.positions {
        return out;
    }
    let step = out.len() as f64 / cli.positions as f64;
    (0..cli.positions)
        .map(|i| out.swap_remove(((i as f64 * step) as usize).min(out.len() - 1)))
        .collect()
}

#[derive(Clone)]
struct EvalRun {
    best: SearchChoice,
    top1_share: f64,
    q_gap: f64,
    wall_ms: f64,
    nodes: usize,
}

fn eval_snapshot(
    snap: &Snapshot,
    dex: &Dex,
    prior: &MetaPool,
    belief_prior: Option<&Arc<BeliefPrior>>,
    cfg: &RmConfig,
    max_worlds: usize,
    seed: u64,
) -> EvalRun {
    let (lp, obs) =
        LocalPlayer::new_with_own_sets(&snap.battle, dex, snap.side, snap.own_sets.clone())
            .expect("lp init failed");
    let mut belief = Belief::with_fallback_policy(dex, prior, &obs, FallbackPolicy::Bayesian);
    if let Some(bp) = belief_prior {
        belief.set_prior(bp.clone());
    }
    belief.sync(dex, &obs);
    let mut pos_rng = SplitMix64::new(snap.pos_seed);
    let pos = lp
        .position(dex, &mut belief, &obs, &mut pos_rng)
        .expect("position failed");

    let t0 = Instant::now();
    let mut bs = BlindSearch::from_observation(&pos, dex, cfg.clone(), snap.side, seed)
        .with_max_worlds(max_worlds);
    bs.prune_preview_aliases(dex);
    if belief.has_preview_prior(&obs) {
        bs.set_opponent_preview_prior(true);
    }
    for _ in 0..cfg.iterations {
        bs.step_one(dex, &belief, &obs);
        if bs.should_early_stop() {
            break;
        }
    }
    let wall_ms = t0.elapsed().as_secs_f64() * 1000.0;
    let best = bs.best().unwrap();
    let visits = bs.visits();
    let means = bs.means();
    let total_v: u32 = visits.iter().sum();
    let best_idx = bs.actions().iter().position(|&c| c == best).unwrap_or(0);
    let top1_share = if total_v > 0 {
        visits[best_idx] as f64 / total_v as f64
    } else {
        0.0
    };
    let mut sorted_q: Vec<f64> = visits
        .iter()
        .zip(means.iter())
        .filter(|(&v, _)| v > 0)
        .map(|(_, &q)| q)
        .collect();
    sorted_q.sort_by(|a, b| b.partial_cmp(a).unwrap_or(std::cmp::Ordering::Equal));
    let q_gap = if sorted_q.len() >= 2 {
        sorted_q[0] - sorted_q[1]
    } else {
        0.0
    };
    EvalRun {
        best,
        top1_share,
        q_gap,
        wall_ms,
        nodes: bs.node_count(),
    }
}

fn percentile(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let idx = ((sorted.len() - 1) as f64 * p).round() as usize;
    sorted[idx.min(sorted.len() - 1)]
}

fn mode_sensitivity(cli: &Cli) -> Value {
    let t_all = Instant::now();
    let root = repo_root();
    let dex = Arc::new(load_dex());
    let prior = Arc::new(load_meta_pool(
        &root.join("data/belief-pool-v3/belief-pool.json"),
    ));
    let belief_prior = cli
        .prior_path
        .as_deref()
        .map(|p| Arc::new(BeliefPrior::load(p)));
    let teams = if cli.pool_mode == PoolMode::Catalog {
        prior.teams.iter().map(|t| t.sets.clone()).collect()
    } else {
        load_offpool_teams(&root, &dex, &prior, cli.mutations, cli.seed)
    };
    let snaps = collect_snapshots(
        cli,
        &dex,
        &prior,
        belief_prior.as_ref(),
        &teams,
    );
    let n_snaps = snaps.len();
    let fb_count = snaps.iter().filter(|s| s.is_fallback).count();
    let mean_fb_cands = if n_snaps > 0 {
        snaps.iter().map(|s| s.fallback_cands).sum::<usize>() as f64 / n_snaps as f64
    } else {
        0.0
    };
    let mean_alive = if n_snaps > 0 {
        snaps.iter().map(|s| s.alive_cands).sum::<usize>() as f64 / n_snaps as f64
    } else {
        0.0
    };

    let cfg = production_cfg(cli.iters);
    let next = AtomicUsize::new(0);
    let base_runs = Mutex::new(vec![None::<EvalRun>; n_snaps]);
    let seed_runs = Mutex::new(vec![None::<EvalRun>; n_snaps]);
    let mut ks_sorted = cli.k_list.clone();
    if !ks_sorted.contains(&24) {
        ks_sorted.push(24);
    }
    ks_sorted.sort_unstable();
    ks_sorted.dedup();
    let k_runs: Vec<Mutex<Vec<Option<EvalRun>>>> = ks_sorted
        .iter()
        .map(|_| Mutex::new(vec![None::<EvalRun>; n_snaps]))
        .collect();

    std::thread::scope(|s| {
        for _ in 0..cli.threads {
            s.spawn(|| loop {
                let idx = next.fetch_add(1, Ordering::Relaxed);
                if idx >= n_snaps {
                    break;
                }
                let snap = &snaps[idx];
                let r_base =
                    eval_snapshot(snap, &dex, &prior, belief_prior.as_ref(), &cfg, 24, snap.seed_a);
                let r_seed =
                    eval_snapshot(snap, &dex, &prior, belief_prior.as_ref(), &cfg, 24, snap.seed_b);
                {
                    base_runs.lock().unwrap()[idx] = Some(r_base);
                    seed_runs.lock().unwrap()[idx] = Some(r_seed);
                }
                for (ki, &k) in ks_sorted.iter().enumerate() {
                    if k == 24 {
                        continue;
                    }
                    let r = eval_snapshot(
                        snap,
                        &dex,
                        &prior,
                        belief_prior.as_ref(),
                        &cfg,
                        k,
                        snap.seed_a,
                    );
                    k_runs[ki].lock().unwrap()[idx] = Some(r);
                }
            });
        }
    });

    let base_vec: Vec<EvalRun> = base_runs
        .into_inner()
        .unwrap()
        .into_iter()
        .map(Option::unwrap)
        .collect();
    let seed_vec: Vec<EvalRun> = seed_runs
        .into_inner()
        .unwrap()
        .into_iter()
        .map(Option::unwrap)
        .collect();

    let summarize = |runs: &[EvalRun], ref_runs: &[EvalRun]| -> Value {
        let n = runs.len().max(1);
        let mut ms_list: Vec<f64> = runs.iter().map(|r| r.wall_ms).collect();
        ms_list.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let ms_mean = ms_list.iter().sum::<f64>() / n as f64;
        let ms_p95 = percentile(&ms_list, 0.95);
        let ref_ms_mean = ref_runs.iter().map(|r| r.wall_ms).sum::<f64>() / n as f64;
        let nodes_mean = runs.iter().map(|r| r.nodes).sum::<usize>() as f64 / n as f64;
        let share_mean = runs.iter().map(|r| r.top1_share).sum::<f64>() / n as f64;
        let q_gap_mean = runs.iter().map(|r| r.q_gap).sum::<f64>() / n as f64;

        let flips = runs
            .iter()
            .zip(ref_runs.iter())
            .filter(|(a, b)| a.best != b.best)
            .count();
        let mut low_n = 0usize;
        let mut low_flips = 0usize;
        let mut mid_n = 0usize;
        let mut mid_flips = 0usize;
        let mut hi_n = 0usize;
        let mut hi_flips = 0usize;
        for (a, b) in runs.iter().zip(ref_runs.iter()) {
            let flipped = a.best != b.best;
            if b.top1_share < 0.50 {
                low_n += 1;
                if flipped {
                    low_flips += 1;
                }
            } else if b.top1_share < 0.80 {
                mid_n += 1;
                if flipped {
                    mid_flips += 1;
                }
            } else {
                hi_n += 1;
                if flipped {
                    hi_flips += 1;
                }
            }
        }
        let r3 = |x: f64| (x * 1e3).round() / 1e3;
        let r4 = |x: f64| (x * 1e4).round() / 1e4;
        json!({
            "wall_ms_mean": r3(ms_mean),
            "wall_ms_p95": r3(ms_p95),
            "ms_ratio_vs_k24": r4(ms_mean / ref_ms_mean.max(1e-9)),
            "nodes_mean": nodes_mean.round() as u64,
            "top1_share_mean": r4(share_mean),
            "q_gap_mean": r4(q_gap_mean),
            "flip_rate_vs_k24": r4(flips as f64 / n as f64),
            "flip_low_share_lt50": [low_flips, low_n],
            "flip_mid_share_50_80": [mid_flips, mid_n],
            "flip_hi_share_ge80": [hi_flips, hi_n],
        })
    };

    let mut arms = serde_json::Map::new();
    arms.insert(
        "k24_seed_noise".to_string(),
        summarize(&seed_vec, &base_vec),
    );
    for (ki, &k) in ks_sorted.iter().enumerate() {
        let label = format!("k{k}");
        if k == 24 {
            arms.insert(label, summarize(&base_vec, &base_vec));
        } else {
            let vec_k: Vec<EvalRun> = k_runs[ki]
                .lock()
                .unwrap()
                .iter_mut()
                .map(|o| o.take().unwrap())
                .collect();
            arms.insert(label, summarize(&vec_k, &base_vec));
        }
    }

    json!({
        "mode": "sensitivity",
        "iters": cli.iters,
        "pool": match cli.pool_mode { PoolMode::Catalog => "catalog", PoolMode::BothOffpool => "both-offpool", PoolMode::Offpool => "offpool" },
        "snapshots": n_snaps,
        "fallback_snapshots": fb_count,
        "mean_fallback_cands": (mean_fb_cands * 100.0).round() / 100.0,
        "mean_alive_cands": (mean_alive * 100.0).round() / 100.0,
        "arms": arms,
        "wall_s": (t_all.elapsed().as_secs_f64() * 100.0).round() / 100.0,
    })
}

fn elo_diff(win_rate: f64) -> f64 {
    let clamped = win_rate.clamp(1e-4, 1.0 - 1e-4);
    -400.0 * (1.0 / clamped - 1.0).log10()
}

fn mode_duel(cli: &Cli) -> Value {
    let t_all = Instant::now();
    let root = repo_root();
    let dex = Arc::new(load_dex());
    let prior = Arc::new(load_meta_pool(
        &root.join("data/belief-pool-v3/belief-pool.json"),
    ));
    let belief_prior = cli
        .prior_path
        .as_deref()
        .map(|p| Arc::new(BeliefPrior::load(p)));
    let teams = if cli.pool_mode == PoolMode::Catalog {
        prior.teams.iter().map(|t| t.sets.clone()).collect::<Vec<_>>()
    } else {
        load_offpool_teams(&root, &dex, &prior, cli.mutations, cli.seed)
    };

    let iters_a = cli.iters_a.unwrap_or(cli.iters);
    let iters_b = cli.iters_b.unwrap_or(cli.iters);
    let cfg_a = production_cfg(iters_a);
    let cfg_b = production_cfg(iters_b);

    let mut pairs_spec: Vec<(Vec<PokemonSet>, Vec<PokemonSet>)> = Vec::with_capacity(cli.pairs);
    for i in 0..cli.pairs {
        match cli.pool_mode {
            PoolMode::Catalog => {
                let a = &prior.teams[(i * 17) % prior.teams.len()];
                let b = &prior.teams[(i * 31 + 11) % prior.teams.len()];
                pairs_spec.push((a.sets.clone(), b.sets.clone()));
            }
            PoolMode::BothOffpool => {
                let a = &teams[i % teams.len()];
                let b = &teams[(i * 13 + 7) % teams.len()];
                pairs_spec.push((a.clone(), b.clone()));
            }
            PoolMode::Offpool => {
                let opp = &teams[i % teams.len()];
                let pilot = &prior.teams[(i * 19 + 3) % prior.teams.len()];
                pairs_spec.push((pilot.sets.clone(), opp.clone()));
            }
        }
    }

    struct PairStats {
        pair_score_avg: f64,
        score_sum: f64,
        scored: usize,
        capped: usize,
        invalids: usize,
        ns_a: u64,
        moves_a: u64,
        ns_b: u64,
        moves_b: u64,
    }

    let next = AtomicUsize::new(0);
    let out = Mutex::new(Vec::<PairStats>::with_capacity(cli.pairs));

    std::thread::scope(|s| {
        for _ in 0..cli.threads {
            s.spawn(|| {
                let mut local = Vec::new();
                loop {
                    let pi = next.fetch_add(1, Ordering::Relaxed);
                    if pi >= pairs_spec.len() {
                        break;
                    }
                    let (t1, t2) = &pairs_spec[pi];
                    let mut rng = SplitMix64::new(cli.seed ^ ((pi as u64 + 1) * 0x9FB2_1C65));
                    let bseed = rng.battle_seed();
                    let seed_a0 = rng.next();
                    let seed_b0 = rng.next();
                    let seed_a1 = rng.next();
                    let seed_b1 = rng.next();

                    let mut score_sum = 0.0f64;
                    let mut scored = 0usize;
                    let mut capped = 0usize;
                    let mut invalids = 0usize;
                    let mut ns_a = 0u64;
                    let mut moves_a = 0u64;
                    let mut ns_b = 0u64;
                    let mut moves_b = 0u64;

                    for (a_is_p1, sa, sb) in [(true, seed_a0, seed_b0), (false, seed_a1, seed_b1)] {
                        let Ok(mut battle) = Battle::from_fixture(&dex, &bseed, t1, t2) else {
                            invalids += 1;
                            continue;
                        };
                        battle.set_log_enabled(true);
                        let mut agent_a = MaxWorldsBlindAgent::new(
                            cfg_a.clone(),
                            prior.clone(),
                            belief_prior.clone(),
                            cli.k_a,
                            sa,
                        );
                        let res = match cli.vs {
                            OpponentKind::Blind24 => {
                                let mut agent_b = MaxWorldsBlindAgent::new(
                                    cfg_b.clone(),
                                    prior.clone(),
                                    belief_prior.clone(),
                                    cli.k_b,
                                    sb,
                                );
                                let r = if a_is_p1 {
                                    play_game(
                                        &dex,
                                        &mut battle,
                                        &mut [&mut agent_a, &mut agent_b],
                                        cli.max_turns,
                                    )
                                } else {
                                    play_game(
                                        &dex,
                                        &mut battle,
                                        &mut [&mut agent_b, &mut agent_a],
                                        cli.max_turns,
                                    )
                                };
                                ns_b += agent_b.think_ns;
                                moves_b += agent_b.moves;
                                r
                            }
                            OpponentKind::Open => {
                                let mut agent_b = OpenAgent::new(cfg_b.clone(), None, sb);
                                let t_b0 = Instant::now();
                                let r = if a_is_p1 {
                                    play_game(
                                        &dex,
                                        &mut battle,
                                        &mut [&mut agent_a, &mut agent_b],
                                        cli.max_turns,
                                    )
                                } else {
                                    play_game(
                                        &dex,
                                        &mut battle,
                                        &mut [&mut agent_b, &mut agent_a],
                                        cli.max_turns,
                                    )
                                };
                                let total_ns = t_b0.elapsed().as_nanos() as u64;
                                ns_b += total_ns.saturating_sub(agent_a.think_ns);
                                moves_b += agent_a.moves;
                                r
                            }
                        };
                        ns_a += agent_a.think_ns;
                        moves_a += agent_a.moves;
                        match res {
                            Ok(GameResult::Outcome(o)) => {
                                let p1_s = match o {
                                    Outcome::P1Win => 1.0,
                                    Outcome::P2Win => 0.0,
                                    Outcome::Tie => 0.5,
                                };
                                score_sum += if a_is_p1 { p1_s } else { 1.0 - p1_s };
                                scored += 1;
                            }
                            Ok(GameResult::TurnCapped) => {
                                capped += 1;
                            }
                            Err(_) => {
                                invalids += 1;
                            }
                        }
                    }
                    if scored > 0 {
                        local.push(PairStats {
                            pair_score_avg: score_sum / scored as f64,
                            score_sum,
                            scored,
                            capped,
                            invalids,
                            ns_a,
                            moves_a,
                            ns_b,
                            moves_b,
                        });
                    } else {
                        local.push(PairStats {
                            pair_score_avg: 0.5,
                            score_sum: 0.0,
                            scored: 0,
                            capped,
                            invalids,
                            ns_a,
                            moves_a,
                            ns_b,
                            moves_b,
                        });
                    }
                }
                out.lock().unwrap().extend(local);
            });
        }
    });

    let rows = out.into_inner().unwrap();
    let score_sum: f64 = rows.iter().map(|r| r.score_sum).sum();
    let scored: usize = rows.iter().map(|r| r.scored).sum();
    let capped: usize = rows.iter().map(|r| r.capped).sum();
    let invalids: usize = rows.iter().map(|r| r.invalids).sum();
    let ns_a: u64 = rows.iter().map(|r| r.ns_a).sum();
    let moves_a: u64 = rows.iter().map(|r| r.moves_a).sum();
    let ns_b: u64 = rows.iter().map(|r| r.ns_b).sum();
    let moves_b: u64 = rows.iter().map(|r| r.moves_b).sum();

    let pair_avgs: Vec<f64> = rows
        .iter()
        .filter(|r| r.scored > 0)
        .map(|r| r.pair_score_avg)
        .collect();
    let n_p = pair_avgs.len();
    let win_rate = if scored > 0 {
        score_sum / scored as f64
    } else {
        0.5
    };
    let ci95 = if n_p >= 2 {
        let mean = pair_avgs.iter().sum::<f64>() / n_p as f64;
        let var = pair_avgs
            .iter()
            .map(|&x| (x - mean) * (x - mean))
            .sum::<f64>()
            / (n_p - 1) as f64;
        1.96 * (var / n_p as f64).sqrt()
    } else {
        0.0
    };

    let ms_a = ns_a as f64 / 1e6 / moves_a.max(1) as f64;
    let ms_b = ns_b as f64 / 1e6 / moves_b.max(1) as f64;

    json!({
        "mode": "duel",
        "vs": match cli.vs { OpponentKind::Blind24 => format!("blind-w{}", cli.k_b), OpponentKind::Open => "open".into() },
        "pool": match cli.pool_mode { PoolMode::Catalog => "catalog", PoolMode::BothOffpool => "both-offpool", PoolMode::Offpool => "offpool" },
        "k_a": cli.k_a,
        "iters_a": iters_a,
        "k_b": if cli.vs == OpponentKind::Blind24 { Some(cli.k_b) } else { None },
        "iters_b": iters_b,
        "pairs": cli.pairs,
        "games_scored": scored,
        "capped": capped,
        "invalids": invalids,
        "score_a": (win_rate * 1e4).round() / 1e4,
        "ci95_pair": (ci95 * 1e4).round() / 1e4,
        "elo_diff": (elo_diff(win_rate) * 10.0).round() / 10.0,
        "ms_per_move_a": (ms_a * 100.0).round() / 100.0,
        "ms_per_move_b": (ms_b * 100.0).round() / 100.0,
        "ms_ratio": ((ms_a / ms_b.max(1e-6)) * 1000.0).round() / 1000.0,
        "wall_s": (t_all.elapsed().as_secs_f64() * 100.0).round() / 100.0,
    })
}

fn main() {
    let cli = parse_cli();
    let out = match cli.mode {
        Mode::Sensitivity => mode_sensitivity(&cli),
        Mode::Duel => mode_duel(&cli),
    };
    println!("{out}");
}
