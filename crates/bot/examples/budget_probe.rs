//! Search-budget vs playing-strength diagnostic and duel harness.
//!
//! Subcommands (all emit single-line JSON on stdout per M2M convention):
//!   budget_probe collect-positions --out GAMES.json [--pairs 16] [--iters 81000] [--preview-iters 243000] [--seed 20261017003] [--threads 10]
//!   budget_probe convergence --games GAMES.json --out CONV.jsonl [--samples 200] [--checkpoints 3000,9000,27000,81000,243000,729000] [--threads 10] [--seed 1]
//!   budget_probe counterfactual --games GAMES.json --conv CONV.jsonl --pair 81000,243000 --out CF.jsonl [--trials 24] [--cont-iters 27000] [--threads 10] [--seed 1]
//!   budget_probe duel --iters-a 243000 --iters-b 81000 --out DUEL.jsonl [--pairs 256] [--pair-start 0] [--preview-iters 243000] [--preview-cache PREV.json] [--seed 20261017003] [--threads 10]

use std::collections::{BTreeMap, HashMap};
use std::fs::File;
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use conformance::fixture::repo_root;
use conformance::load_dex;
use nc2000_bot::belief::FallbackPolicy;
use nc2000_bot::player_state::{submitted_team, LocalPlayer};
use nc2000_bot::preview::{action_index, load_meta_pool, preview_actions, MetaPool};
use nc2000_bot::runner::{play_game, GameResult};
use nc2000_bot::smmcts::{EndgameRegime, RmConfig, SelRule};
use nc2000_bot::{Agent, Belief, BlindAgent, BlindSearch, Observer, SplitMix64};
use nc2000_engine::battle::{Outcome, PokemonSet, SearchChoice};
use nc2000_engine::dex::Dex;
use nc2000_engine::state::Battle;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Clone, Deserialize)]
struct WeightedCatalog {
    teams: Vec<WeightedTeam>,
}

#[derive(Clone, Deserialize)]
struct WeightedTeam {
    id: String,
    sets: Vec<PokemonSet>,
    #[serde(rename = "drawWeight")]
    draw_weight: Option<f64>,
}

#[derive(Clone, Serialize, Deserialize)]
struct SchedulePair {
    pair_idx: usize,
    team_a_idx: usize,
    team_b_idx: usize,
    team_a_id: String,
    team_b_id: String,
    battle_seed: String,
    preview_seed_a: u64,
    preview_seed_b: u64,
    seat_seed_p1: u64,
    seat_seed_p2: u64,
    preview_a: [u8; 3],
    preview_b: [u8; 3],
}

#[derive(Clone, Serialize, Deserialize)]
struct RecordedStep {
    choices: [Option<String>; 2],
    searched: [bool; 2],
    rng_after: [u64; 2],
}

#[derive(Clone, Serialize, Deserialize)]
struct RecordedGame {
    game_idx: usize,
    pair_idx: usize,
    a_is_p1: bool,
    team_p1_idx: usize,
    team_p2_idx: usize,
    team_p1_id: String,
    team_p2_id: String,
    battle_seed: String,
    preview_p1: [u8; 3],
    preview_p2: [u8; 3],
    seat_seed_p1: u64,
    seat_seed_p2: u64,
    turns: u16,
    p1_score: f64,
    steps: Vec<RecordedStep>,
}

#[derive(Clone, Serialize, Deserialize)]
struct RecordedGamesBundle {
    schema: String,
    seed: u64,
    collect_iters: u32,
    preview_iters: u32,
    games: Vec<RecordedGame>,
}

/// Bit-exact replayable blind player mirroring `BlindAgent`'s lifecycle so
/// prefix turns advance `LocalPlayer`/`Observer`/`Belief` without searching.
struct ReplayableBlindPlayer {
    side: usize,
    cfg: RmConfig,
    rng: SplitMix64,
    local: LocalPlayer,
    observer: Observer,
    belief: Belief,
    preview_triple: [u8; 3],
}

impl ReplayableBlindPlayer {
    fn new(
        battle: &Battle,
        dex: &Dex,
        pool: &MetaPool,
        side: usize,
        cfg: RmConfig,
        seed: u64,
        preview_triple: [u8; 3],
    ) -> Self {
        let own_sets = submitted_team(battle, dex, side);
        let (local, observer) =
            LocalPlayer::new_with_own_sets(battle, dex, side, own_sets).expect("local player");
        let belief = Belief::with_fallback_policy(dex, pool, &observer, FallbackPolicy::default());
        Self {
            side,
            cfg,
            rng: SplitMix64::new(seed),
            local,
            observer,
            belief,
            preview_triple,
        }
    }

    fn observe_sync(&mut self, battle: &Battle, dex: &Dex) {
        self.local
            .observe(battle, dex, &mut self.observer)
            .expect("observe failed");
        self.belief.sync(dex, &self.observer);
    }

    fn build_search(
        &mut self,
        battle: &Battle,
        dex: &Dex,
        choices: &[SearchChoice],
    ) -> BlindSearch {
        let pos = self
            .local
            .position(dex, &mut self.belief, &self.observer, &mut self.rng)
            .expect("position reconstruction failed");
        let has_prior = self.belief.has_preview_prior(&self.observer);
        let mut bs =
            BlindSearch::from_observation(&pos, dex, self.cfg.clone(), self.side, self.rng.0);
        bs.prune_preview_aliases(dex);
        if has_prior {
            bs.set_opponent_preview_prior(true);
        }
        debug_assert_eq!(bs.actions(), choices);
        bs
    }

    fn advance_recorded(&mut self, battle: &Battle, dex: &Dex, rng_after: u64) {
        self.observe_sync(battle, dex);
        self.rng = SplitMix64::new(rng_after);
    }

    fn choose(&mut self, battle: &Battle, dex: &Dex, choices: &[SearchChoice]) -> SearchChoice {
        self.observe_sync(battle, dex);
        if choices.len() == 1 {
            return choices[0];
        }
        if matches!(choices[0], SearchChoice::Team(_)) {
            let _ = self.rng.next_f64();
            return SearchChoice::Team(self.preview_triple);
        }
        let mut bs = self.build_search(battle, dex, choices);
        bs.step(dex, &self.belief, &self.observer, self.cfg.iterations);
        let pick = bs.best().expect("non-empty choice");
        self.rng = SplitMix64::new(bs.search_rng_state());
        pick
    }
}

fn flag(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1).cloned())
}

fn default_threads() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
        .min(10)
}

fn blind_cfg(iters: u32) -> RmConfig {
    RmConfig {
        iterations: iters,
        rule: SelRule::Ucb,
        c: 0.4,
        ..Default::default()
    }
}

fn load_catalog(root: &Path) -> WeightedCatalog {
    let text = std::fs::read_to_string(root.join("data/team-pool-v2/team-pool.json"))
        .expect("read team-pool-v2");
    serde_json::from_str(&text).expect("parse team-pool-v2")
}

fn sample_weighted(weights: &[f64], rng: &mut SplitMix64) -> usize {
    let total: f64 = weights.iter().sum();
    let mut r = rng.next_f64() * total;
    for (i, &w) in weights.iter().enumerate() {
        if r < w {
            return i;
        }
        r -= w;
    }
    weights.len() - 1
}

fn one_hot_preview_policy(triple: [u8; 3]) -> Vec<f64> {
    let acts = preview_actions();
    let idx = action_index(&acts, triple).expect("canonical preview action");
    let mut pol = vec![0.0; acts.len()];
    pol[idx] = 1.0;
    pol
}

fn compute_preview_choice(
    dex: &Dex,
    pool: &Arc<MetaPool>,
    own_sets: &[PokemonSet],
    opp_sets: &[PokemonSet],
    battle_seed: &str,
    side: usize,
    preview_seed: u64,
    preview_iters: u32,
) -> [u8; 3] {
    let mut b = Battle::from_fixture(
        dex,
        battle_seed,
        if side == 0 { own_sets } else { opp_sets },
        if side == 0 { opp_sets } else { own_sets },
    )
    .expect("preview battle");
    b.set_log_enabled(true);
    let choices = b.legal_choices(dex, side);
    let mut agent = BlindAgent::new(blind_cfg(preview_iters), pool.clone(), None, preview_seed);
    match agent.choose(&b, dex, side, &choices) {
        SearchChoice::Team(t) => t,
        other => panic!("expected team choice, got {other:?}"),
    }
}

fn build_schedules(
    dex: &Dex,
    catalog: &WeightedCatalog,
    pool: &Arc<MetaPool>,
    pairs: usize,
    seed: u64,
    preview_iters: u32,
    threads: usize,
    preview_cache_path: Option<&Path>,
) -> Vec<SchedulePair> {
    let weights: Vec<f64> = catalog
        .teams
        .iter()
        .map(|t| t.draw_weight.unwrap_or(1.0))
        .collect();
    let mut rng = SplitMix64::new(seed);
    let mut raw: Vec<SchedulePair> = Vec::with_capacity(pairs);
    for pair_idx in 0..pairs {
        let ta = sample_weighted(&weights, &mut rng);
        let tb = sample_weighted(&weights, &mut rng);
        let battle_seed = rng.battle_seed();
        let preview_seed_a = rng.next();
        let preview_seed_b = rng.next();
        let seat_seed_p1 = rng.next();
        let seat_seed_p2 = rng.next();
        raw.push(SchedulePair {
            pair_idx,
            team_a_idx: ta,
            team_b_idx: tb,
            team_a_id: catalog.teams[ta].id.clone(),
            team_b_id: catalog.teams[tb].id.clone(),
            battle_seed,
            preview_seed_a,
            preview_seed_b,
            seat_seed_p1,
            seat_seed_p2,
            preview_a: [0; 3],
            preview_b: [0; 3],
        });
    }

    let mut cached: HashMap<(usize, u32), ([u8; 3], [u8; 3])> = HashMap::new();
    if let Some(path) = preview_cache_path {
        if path.exists() {
            if let Ok(txt) = std::fs::read_to_string(path) {
                if let Ok(v) = serde_json::from_str::<Value>(&txt) {
                    if v["seed"].as_u64() == Some(seed)
                        && v["preview_iters"].as_u64() == Some(u64::from(preview_iters))
                    {
                        if let Some(arr) = v["entries"].as_array() {
                            for e in arr {
                                if let (Some(idx), Some(pa), Some(pb)) = (
                                    e["pair_idx"].as_u64(),
                                    serde_json::from_value::<[u8; 3]>(e["preview_a"].clone()).ok(),
                                    serde_json::from_value::<[u8; 3]>(e["preview_b"].clone()).ok(),
                                ) {
                                    cached.insert((idx as usize, preview_iters), (pa, pb));
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    let missing: Vec<usize> = (0..pairs)
        .filter(|i| !cached.contains_key(&(*i, preview_iters)))
        .collect();
    if !missing.is_empty() {
        let cursor = AtomicUsize::new(0);
        let results: Mutex<Vec<(usize, [u8; 3], [u8; 3])>> = Mutex::new(Vec::new());
        std::thread::scope(|scope| {
            for _ in 0..threads.min(missing.len()).max(1) {
                let (raw, missing, cursor, results) = (&raw, &missing, &cursor, &results);
                scope.spawn(move || loop {
                    let k = cursor.fetch_add(1, Ordering::Relaxed);
                    if k >= missing.len() {
                        break;
                    }
                    let idx = missing[k];
                    let s = &raw[idx];
                    let ta = &catalog.teams[s.team_a_idx].sets;
                    let tb = &catalog.teams[s.team_b_idx].sets;
                    let pa = compute_preview_choice(
                        dex,
                        pool,
                        ta,
                        tb,
                        &s.battle_seed,
                        0,
                        s.preview_seed_a,
                        preview_iters,
                    );
                    let pb = compute_preview_choice(
                        dex,
                        pool,
                        tb,
                        ta,
                        &s.battle_seed,
                        1,
                        s.preview_seed_b,
                        preview_iters,
                    );
                    results.lock().unwrap().push((idx, pa, pb));
                });
            }
        });
        for (idx, pa, pb) in results.into_inner().unwrap() {
            cached.insert((idx, preview_iters), (pa, pb));
        }
        if let Some(path) = preview_cache_path {
            if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
                std::fs::create_dir_all(parent).unwrap();
            }
            let mut entries: Vec<Value> = Vec::with_capacity(pairs);
            for i in 0..pairs {
                let (pa, pb) = cached[&(i, preview_iters)];
                entries.push(json!({
                    "pair_idx": i,
                    "preview_a": pa,
                    "preview_b": pb,
                }));
            }
            let payload = json!({
                "schema": "nc2000-preview-cache-v1",
                "seed": seed,
                "preview_iters": preview_iters,
                "entries": entries,
            });
            std::fs::write(path, serde_json::to_vec(&payload).unwrap()).unwrap();
        }
    }

    for s in &mut raw {
        let (pa, pb) = cached[&(s.pair_idx, preview_iters)];
        s.preview_a = pa;
        s.preview_b = pb;
    }
    raw
}

fn outcome_p1_score(res: GameResult) -> f64 {
    match res {
        GameResult::Outcome(Outcome::P1Win) => 1.0,
        GameResult::Outcome(Outcome::P2Win) => 0.0,
        GameResult::Outcome(Outcome::Tie) | GameResult::TurnCapped => 0.5,
    }
}

struct TimedBlindWrapper {
    inner: BlindAgent,
    searched_battle_ns: Vec<u64>,
}

impl TimedBlindWrapper {
    fn new(inner: BlindAgent) -> Self {
        Self {
            inner,
            searched_battle_ns: Vec::new(),
        }
    }
}

impl Agent for TimedBlindWrapper {
    fn name(&self) -> String {
        self.inner.name()
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
        let is_searched_battle =
            choices.len() > 1 && !matches!(choices.first(), Some(SearchChoice::Team(_)));
        let t0 = Instant::now();
        let c = self.inner.choose(battle, dex, side, choices);
        if is_searched_battle {
            self.searched_battle_ns.push(t0.elapsed().as_nanos() as u64);
        }
        c
    }
}

fn bootstrap_ci(pair_scores: &[f64], seed: u64) -> (f64, f64, f64) {
    let n = pair_scores.len();
    let mean = pair_scores.iter().sum::<f64>() / n as f64;
    if n < 2 {
        return (mean, mean, mean);
    }
    let mut rng = SplitMix64::new(seed ^ 0xB007_57AA_1234_5678);
    let mut means = Vec::with_capacity(4000);
    for _ in 0..4000 {
        let mut sum = 0.0;
        for _ in 0..n {
            sum += pair_scores[rng.below(n)];
        }
        means.push(sum / n as f64);
    }
    means.sort_by(|a, b| a.total_cmp(b));
    (mean, means[100], means[3899])
}

fn percentile_ms(samples: &[u64], pct: usize) -> f64 {
    if samples.is_empty() {
        return 0.0;
    }
    let mut sorted = samples.to_vec();
    sorted.sort_unstable();
    let idx = (sorted.len() * pct)
        .div_ceil(100)
        .saturating_sub(1)
        .min(sorted.len() - 1);
    sorted[idx] as f64 / 1e6
}

fn elo_diff(p: f64) -> f64 {
    let pc = p.clamp(1e-4, 1.0 - 1e-4);
    400.0 * (pc / (1.0 - pc)).log10()
}

fn find_choice(dex: &Dex, legal: &[SearchChoice], input: &str) -> SearchChoice {
    *legal
        .iter()
        .find(|&&c| c.to_input(dex) == input)
        .unwrap_or_else(|| panic!("choice `{input}` not in legal choices"))
}

fn replay_to_step(
    dex: &Dex,
    catalog: &WeightedCatalog,
    pool: &MetaPool,
    game: &RecordedGame,
    target_step: usize,
    iters: u32,
) -> (Battle, [ReplayableBlindPlayer; 2]) {
    let mut battle = Battle::from_fixture(
        dex,
        &game.battle_seed,
        &catalog.teams[game.team_p1_idx].sets,
        &catalog.teams[game.team_p2_idx].sets,
    )
    .unwrap();
    battle.set_log_enabled(true);
    let mut players = [
        ReplayableBlindPlayer::new(
            &battle,
            dex,
            pool,
            0,
            blind_cfg(iters),
            game.seat_seed_p1,
            game.preview_p1,
        ),
        ReplayableBlindPlayer::new(
            &battle,
            dex,
            pool,
            1,
            blind_cfg(iters),
            game.seat_seed_p2,
            game.preview_p2,
        ),
    ];
    for step in &game.steps[..target_step] {
        let c0 = battle.legal_choices(dex, 0);
        let c1 = battle.legal_choices(dex, 1);
        players[0].advance_recorded(&battle, dex, step.rng_after[0]);
        players[1].advance_recorded(&battle, dex, step.rng_after[1]);
        let ch0 = step.choices[0].as_deref().map(|s| find_choice(dex, &c0, s));
        let ch1 = step.choices[1].as_deref().map(|s| find_choice(dex, &c1, s));
        battle.apply_choices(dex, [ch0, ch1]).unwrap();
    }
    players[0].observe_sync(&battle, dex);
    players[1].observe_sync(&battle, dex);
    (battle, players)
}

fn cmd_collect_positions(args: &[String]) {
    let out_path = PathBuf::from(flag(args, "--out").expect("--out PATH"));
    let pairs: usize = flag(args, "--pairs").map_or(16, |v| v.parse().unwrap());
    let collect_iters: u32 = flag(args, "--iters").map_or(81_000, |v| v.parse().unwrap());
    let preview_iters: u32 = flag(args, "--preview-iters").map_or(243_000, |v| v.parse().unwrap());
    let seed: u64 = flag(args, "--seed").map_or(20261017003, |v| v.parse().unwrap());
    let threads: usize = flag(args, "--threads").map_or_else(default_threads, |v| v.parse().unwrap());
    let preview_cache = flag(args, "--preview-cache").map(PathBuf::from);

    let root = repo_root();
    let dex = load_dex();
    let catalog = load_catalog(&root);
    let pool = Arc::new(load_meta_pool(&root.join("data/belief-pool-v3/belief-pool.json")));
    let t0 = Instant::now();
    let schedules = build_schedules(
        &dex,
        &catalog,
        &pool,
        pairs,
        seed,
        preview_iters,
        threads,
        preview_cache.as_deref(),
    );

    let cursor = AtomicUsize::new(0);
    let games_out: Mutex<Vec<RecordedGame>> = Mutex::new(Vec::new());
    let total_games = pairs;
    std::thread::scope(|scope| {
        for _ in 0..threads.min(total_games).max(1) {
            let (dex, catalog, pool, schedules, cursor, games_out) =
                (&dex, &catalog, &pool, &schedules, &cursor, &games_out);
            scope.spawn(move || loop {
                let game_idx = cursor.fetch_add(1, Ordering::Relaxed);
                if game_idx >= total_games {
                    break;
                }
                let sched = &schedules[game_idx];
                let a_is_p1 = true;
                let (t1_idx, t2_idx, p1_prev, p2_prev) = (
                    sched.team_a_idx,
                    sched.team_b_idx,
                    sched.preview_a,
                    sched.preview_b,
                );
                let mut battle = Battle::from_fixture(
                    dex,
                    &sched.battle_seed,
                    &catalog.teams[t1_idx].sets,
                    &catalog.teams[t2_idx].sets,
                )
                .unwrap();
                battle.set_log_enabled(true);
                let mut p1 = ReplayableBlindPlayer::new(
                    &battle,
                    dex,
                    pool,
                    0,
                    blind_cfg(collect_iters),
                    sched.seat_seed_p1,
                    p1_prev,
                );
                let mut p2 = ReplayableBlindPlayer::new(
                    &battle,
                    dex,
                    pool,
                    1,
                    blind_cfg(collect_iters),
                    sched.seat_seed_p2,
                    p2_prev,
                );
                let mut steps = Vec::new();
                while battle.outcome().is_none() && battle.turn < 500 {
                    let c0 = battle.legal_choices(dex, 0);
                    let c1 = battle.legal_choices(dex, 1);
                    if c0.is_empty() && c1.is_empty() {
                        break;
                    }
                    let ch0 = (!c0.is_empty()).then(|| p1.choose(&battle, dex, &c0));
                    let ch1 = (!c1.is_empty()).then(|| p2.choose(&battle, dex, &c1));
                    let is_prev = matches!(c0.first(), Some(SearchChoice::Team(_)))
                        || matches!(c1.first(), Some(SearchChoice::Team(_)));
                    steps.push(RecordedStep {
                        choices: [ch0.map(|c| c.to_input(dex)), ch1.map(|c| c.to_input(dex))],
                        searched: [!is_prev && c0.len() > 1, !is_prev && c1.len() > 1],
                        rng_after: [p1.rng.0, p2.rng.0],
                    });
                    battle.apply_choices(dex, [ch0, ch1]).unwrap();
                }
                let p1_score = match battle.outcome() {
                    Some(Outcome::P1Win) => 1.0,
                    Some(Outcome::P2Win) => 0.0,
                    Some(Outcome::Tie) | None => 0.5,
                };
                games_out.lock().unwrap().push(RecordedGame {
                    game_idx,
                    pair_idx: sched.pair_idx,
                    a_is_p1,
                    team_p1_idx: t1_idx,
                    team_p2_idx: t2_idx,
                    team_p1_id: catalog.teams[t1_idx].id.clone(),
                    team_p2_id: catalog.teams[t2_idx].id.clone(),
                    battle_seed: sched.battle_seed.clone(),
                    preview_p1: p1_prev,
                    preview_p2: p2_prev,
                    seat_seed_p1: sched.seat_seed_p1,
                    seat_seed_p2: sched.seat_seed_p2,
                    turns: battle.turn,
                    p1_score,
                    steps,
                });
            });
        }
    });
    let mut games = games_out.into_inner().unwrap();
    games.sort_by_key(|g| g.game_idx);
    let searched_decisions: usize = games
        .iter()
        .map(|g| {
            g.steps
                .iter()
                .map(|s| usize::from(s.searched[0]) + usize::from(s.searched[1]))
                .sum::<usize>()
        })
        .sum();
    let bundle = RecordedGamesBundle {
        schema: "nc2000-recorded-games-v1".into(),
        seed,
        collect_iters,
        preview_iters,
        games,
    };
    if let Some(parent) = out_path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(&out_path, serde_json::to_vec(&bundle).unwrap()).unwrap();
    println!(
        "{}",
        json!({
            "cmd": "collect-positions",
            "games": total_games,
            "searched_decisions": searched_decisions,
            "wall_s": (t0.elapsed().as_secs_f64() * 100.0).round() / 100.0,
            "out": out_path,
        })
    );
}

fn regime_label(r: EndgameRegime) -> &'static str {
    match r {
        EndgameRegime::Normal => "normal",
        EndgameRegime::FastWin => "fast_win",
        EndgameRegime::SafeWin => "safe_win",
        EndgameRegime::ComebackLoss => "comeback_loss",
    }
}

fn cmd_convergence(args: &[String]) {
    let games_path = PathBuf::from(flag(args, "--games").expect("--games PATH"));
    let out_path = PathBuf::from(flag(args, "--out").expect("--out PATH"));
    let samples: usize = flag(args, "--samples").map_or(200, |v| v.parse().unwrap());
    let seed: u64 = flag(args, "--seed").map_or(1, |v| v.parse().unwrap());
    let threads: usize = flag(args, "--threads").map_or_else(default_threads, |v| v.parse().unwrap());
    let max_worlds: Option<usize> = flag(args, "--max-worlds").map(|v| v.parse().unwrap());
    let mut checkpoints: Vec<u32> = flag(args, "--checkpoints")
        .unwrap_or_else(|| "3000,9000,27000,81000,243000,729000".into())
        .split(',')
        .map(|s| s.trim().parse().unwrap())
        .collect();
    checkpoints.sort_unstable();
    checkpoints.dedup();
    assert!(!checkpoints.is_empty());

    let bundle: RecordedGamesBundle =
        serde_json::from_slice(&std::fs::read(&games_path).unwrap()).unwrap();
    let root = repo_root();
    let dex = load_dex();
    let catalog = load_catalog(&root);
    let pool = load_meta_pool(&root.join("data/belief-pool-v3/belief-pool.json"));
    let t0 = Instant::now();

    let mut all_slots: Vec<(usize, usize, usize)> = Vec::new();
    for (gi, g) in bundle.games.iter().enumerate() {
        for (si, st) in g.steps.iter().enumerate() {
            for side in 0..2 {
                if st.searched[side] {
                    all_slots.push((gi, si, side));
                }
            }
        }
    }
    let mut rng = SplitMix64::new(seed ^ 0x51A9_2026_1118_0001);
    let mut indices: Vec<usize> = (0..all_slots.len()).collect();
    for i in (1..indices.len()).rev() {
        let j = rng.below(i + 1);
        indices.swap(i, j);
    }
    indices.truncate(samples.min(indices.len()));
    indices.sort_unstable();
    let selected: Vec<(usize, usize, usize)> = indices.into_iter().map(|i| all_slots[i]).collect();

    let cursor = AtomicUsize::new(0);
    let rows: Mutex<Vec<Value>> = Mutex::new(Vec::with_capacity(selected.len()));
    std::thread::scope(|scope| {
        for _ in 0..threads.min(selected.len()).max(1) {
            let (dex, catalog, pool, bundle, selected, checkpoints, cursor, rows) = (
                &dex,
                &catalog,
                &pool,
                &bundle,
                &selected,
                &checkpoints,
                &cursor,
                &rows,
            );
            scope.spawn(move || loop {
                let pos_idx = cursor.fetch_add(1, Ordering::Relaxed);
                if pos_idx >= selected.len() {
                    break;
                }
                let (gi, step_idx, side) = selected[pos_idx];
                let game = &bundle.games[gi];
                let (mut battle, mut players) =
                    replay_to_step(dex, catalog, pool, game, step_idx, bundle.collect_iters);
                let legal = battle.legal_choices(dex, side);
                let forced_switch = matches!(
                    battle.request_state,
                    nc2000_engine::state::RequestState::Switch
                );
                let p = &mut players[side];
                let alive_cands = p.belief.candidate_count();
                let mut bs = p.build_search(&battle, dex, &legal);
                if let Some(k) = max_worlds {
                    bs = bs.with_max_worlds(k);
                }

                let mut prev = 0u32;
                let mut ckpts_json = Vec::with_capacity(checkpoints.len());
                for &target_it in checkpoints {
                    let delta = target_it - prev;
                    let seg_t0 = Instant::now();
                    bs.step(dex, &p.belief, &p.observer, delta);
                    let seg_ms = seg_t0.elapsed().as_secs_f64() * 1000.0;
                    prev = target_it;

                    let eq_t0 = Instant::now();
                    let eq = bs.bayesian_equilibrium();
                    let eq_us = eq_t0.elapsed().as_nanos() as u64 / 1000;

                    let best = bs.best().unwrap();
                    let eg = bs.endgame_analysis().unwrap();
                    let legacy = bs.actions()[eg.legacy_idx];
                    let actions = bs.actions();
                    let visits = bs.visits();
                    let means = bs.means();
                    let (eligible, rtypes) = bs.root_policy_evidence();
                    let mut ranked: Vec<usize> =
                        (0..actions.len()).filter(|&i| eligible[i]).collect();
                    ranked.sort_by(|&a, &b| {
                        visits[b]
                            .cmp(&visits[a])
                            .then_with(|| means[b].total_cmp(&means[a]))
                    });
                    let v1 = ranked.first().map_or(0, |&i| visits[i]);
                    let v2 = ranked.get(1).map_or(0, |&i| visits[i]);
                    let m1 = ranked.first().map_or(0.5, |&i| means[i]);
                    let m2 = ranked.get(1).map_or(0.5, |&i| means[i]);
                    let best_idx = actions.iter().position(|&a| a == best).unwrap();
                    // Match best(): endgame choices return before the LP draw.
                    let eq_override = eq.as_ref().filter(|e| {
                        eg.regime == EndgameRegime::Normal
                            && bs.iterations() >= 64
                            && e.override_value
                                > e.pure_guarantees[eg.chosen_idx]
                                    + nc2000_bot::root_policy::MIN_MIX_MARGIN
                    });
                    let action_stats: Vec<_> = ranked.iter().map(|&i| json!({
                        "action": actions[i].to_input(dex),
                        "visits": visits[i],
                        "mean": means[i],
                        "eq_probability": eq.as_ref().map(|e| e.policy[i]),
                    })).collect();
                    let mut reply_visits = BTreeMap::<String, u32>::new();
                    for (_, reply, count, _) in bs.root_matrix() {
                        *reply_visits.entry(reply.to_input(dex)).or_default() += count;
                    }

                    ckpts_json.push(json!({
                        "iters": target_it,
                        "seg_ms": (seg_ms * 1000.0).round() / 1000.0,
                        "best": best.to_input(dex),
                        "legacy": legacy.to_input(dex),
                        "endgame_choice": actions[eg.chosen_idx].to_input(dex),
                        "regime": regime_label(eg.regime),
                        "endgame_override": eg.chosen_idx != eg.legacy_idx,
                        "eq_override": eq_override.is_some(),
                        "eq_mixed": eq_override.is_some_and(|e| e.policy.iter().filter(|&&p| p > 0.0).count() > 1),
                        "action_stats": action_stats,
                        "reply_visits": reply_visits,
                        "mean_depth": (bs.mean_depth() * 1000.0).round() / 1000.0,
                        "max_depth": bs.max_depth(),
                        "nodes": bs.node_count(),
                        "root_types": rtypes.len(),
                        "top1_v": v1,
                        "top2_v": v2,
                        "top1_m": (m1 * 10000.0).round() / 10000.0,
                        "top2_m": (m2 * 10000.0).round() / 10000.0,
                        "best_m": (means[best_idx] * 10000.0).round() / 10000.0,
                        "q_gap": ((m1 - m2).abs() * 10000.0).round() / 10000.0,
                        "single_eq_probe_us": eq_us,
                    }));
                }

                rows.lock().unwrap().push(json!({
                    "pos_idx": pos_idx,
                    "game_idx": gi,
                    "step_idx": step_idx,
                    "side": side,
                    "turn": battle.turn,
                    "forced_switch": forced_switch,
                    "n_choices": legal.len(),
                    "alive_cands": alive_cands,
                    "checkpoints": ckpts_json,
                }));
            });
        }
    });

    let mut rows = rows.into_inner().unwrap();
    rows.sort_by_key(|r| r["pos_idx"].as_u64().unwrap());
    if let Some(parent) = out_path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent).unwrap();
    }
    let mut writer = BufWriter::new(File::create(&out_path).unwrap());
    for row in &rows {
        serde_json::to_writer(&mut writer, row).unwrap();
        writeln!(writer).unwrap();
    }
    writer.flush().unwrap();

    let n = rows.len() as f64;
    let mut transitions = Vec::new();
    for w in checkpoints.windows(2) {
        let (a, b) = (w[0], w[1]);
        let idx_a = checkpoints.iter().position(|&x| x == a).unwrap();
        let idx_b = idx_a + 1;
        let mut flips = 0usize;
        let mut legacy_flips = 0usize;
        let mut switch_flips = 0usize;
        let mut eq_involved = 0usize;
        for r in &rows {
            let ca = &r["checkpoints"][idx_a];
            let cb = &r["checkpoints"][idx_b];
            let ba = ca["best"].as_str().unwrap();
            let bb = cb["best"].as_str().unwrap();
            if ba != bb {
                flips += 1;
                if ba.starts_with("switch") != bb.starts_with("switch") {
                    switch_flips += 1;
                }
                if ca["eq_override"].as_bool().unwrap() || cb["eq_override"].as_bool().unwrap() {
                    eq_involved += 1;
                }
            }
            if ca["legacy"].as_str().unwrap() != cb["legacy"].as_str().unwrap() {
                legacy_flips += 1;
            }
        }
        transitions.push(json!({
            "from": a,
            "to": b,
            "flips": flips,
            "flip_rate": (flips as f64 / n * 10000.0).round() / 10000.0,
            "legacy_flips": legacy_flips,
            "legacy_flip_rate": (legacy_flips as f64 / n * 10000.0).round() / 10000.0,
            "move_switch_flips": switch_flips,
            "eq_involved_flips": eq_involved,
        }));
    }

    let last_idx = checkpoints.len() - 1;
    let mut per_ckpt = Vec::new();
    for (ci, &it) in checkpoints.iter().enumerate() {
        let agree_final = rows
            .iter()
            .filter(|r| {
                r["checkpoints"][ci]["best"].as_str() == r["checkpoints"][last_idx]["best"].as_str()
            })
            .count();
        let mean_depth: f64 = rows
            .iter()
            .map(|r| r["checkpoints"][ci]["mean_depth"].as_f64().unwrap())
            .sum::<f64>()
            / n;
        let mean_nodes: f64 = rows
            .iter()
            .map(|r| r["checkpoints"][ci]["nodes"].as_f64().unwrap())
            .sum::<f64>()
            / n;
        let mean_seg_ms: f64 = rows
            .iter()
            .map(|r| r["checkpoints"][ci]["seg_ms"].as_f64().unwrap())
            .sum::<f64>()
            / n;
        let eq_rate = rows
            .iter()
            .filter(|r| r["checkpoints"][ci]["eq_override"].as_bool().unwrap())
            .count() as f64
            / n;
        let eg_rate = rows
            .iter()
            .filter(|r| r["checkpoints"][ci]["endgame_override"].as_bool().unwrap())
            .count() as f64
            / n;
        per_ckpt.push(json!({
            "iters": it,
            "agree_final_rate": (agree_final as f64 / n * 10000.0).round() / 10000.0,
            "mean_depth": (mean_depth * 1000.0).round() / 1000.0,
            "mean_nodes": mean_nodes.round(),
            "mean_seg_ms": (mean_seg_ms * 100.0).round() / 100.0,
            "eq_override_rate": (eq_rate * 10000.0).round() / 10000.0,
            "endgame_override_rate": (eg_rate * 10000.0).round() / 10000.0,
        }));
    }

    println!(
        "{}",
        json!({
            "cmd": "convergence",
            "positions": rows.len(),
            "total_pool_positions": all_slots.len(),
            "max_worlds": max_worlds.unwrap_or(24),
            "wall_s": (t0.elapsed().as_secs_f64() * 100.0).round() / 100.0,
            "checkpoints": per_ckpt,
            "transitions": transitions,
            "out": out_path,
        })
    );
}

fn cmd_counterfactual(args: &[String]) {
    let games_path = PathBuf::from(flag(args, "--games").expect("--games PATH"));
    let conv_path = PathBuf::from(flag(args, "--conv").expect("--conv PATH"));
    let out_path = PathBuf::from(flag(args, "--out").expect("--out PATH"));
    let pair_str = flag(args, "--pair").unwrap_or_else(|| "81000,243000".into());
    let (lo_s, hi_s) = pair_str.split_once(',').expect("--pair LO,HI");
    let lo_iters: u32 = lo_s.parse().unwrap();
    let hi_iters: u32 = hi_s.parse().unwrap();
    let trials: usize = flag(args, "--trials").map_or(20, |v| v.parse().unwrap());
    let cont_iters: u32 = flag(args, "--cont-iters").map_or(27_000, |v| v.parse().unwrap());
    let seed: u64 = flag(args, "--seed").map_or(1, |v| v.parse().unwrap());
    let threads: usize = flag(args, "--threads").map_or_else(default_threads, |v| v.parse().unwrap());

    let bundle: RecordedGamesBundle =
        serde_json::from_slice(&std::fs::read(&games_path).unwrap()).unwrap();
    let mut conv_rows: Vec<Value> = Vec::new();
    for line in BufReader::new(File::open(&conv_path).unwrap()).lines() {
        let line = line.unwrap();
        if !line.trim().is_empty() {
            conv_rows.push(serde_json::from_str(&line).unwrap());
        }
    }

    #[derive(Clone)]
    struct Disagreement {
        pos_idx: usize,
        game_idx: usize,
        step_idx: usize,
        side: usize,
        turn: u16,
        lo_choice: String,
        hi_choice: String,
        regime_lo: String,
        regime_hi: String,
    }

    let mut disagreements = Vec::new();
    for r in &conv_rows {
        let ckpts = r["checkpoints"].as_array().unwrap();
        let clo = ckpts
            .iter()
            .find(|c| c["iters"].as_u64() == Some(u64::from(lo_iters)));
        let chi = ckpts
            .iter()
            .find(|c| c["iters"].as_u64() == Some(u64::from(hi_iters)));
        if let (Some(lo), Some(hi)) = (clo, chi) {
            let bl = lo["best"].as_str().unwrap();
            let bh = hi["best"].as_str().unwrap();
            if bl != bh {
                disagreements.push(Disagreement {
                    pos_idx: r["pos_idx"].as_u64().unwrap() as usize,
                    game_idx: r["game_idx"].as_u64().unwrap() as usize,
                    step_idx: r["step_idx"].as_u64().unwrap() as usize,
                    side: r["side"].as_u64().unwrap() as usize,
                    turn: r["turn"].as_u64().unwrap() as u16,
                    lo_choice: bl.into(),
                    hi_choice: bh.into(),
                    regime_lo: lo["regime"].as_str().unwrap().into(),
                    regime_hi: hi["regime"].as_str().unwrap().into(),
                });
            }
        }
    }

    let root = repo_root();
    let dex = load_dex();
    let catalog = load_catalog(&root);
    let pool = load_meta_pool(&root.join("data/belief-pool-v3/belief-pool.json"));
    let t0 = Instant::now();

    if disagreements.is_empty() {
        println!(
            "{}",
            json!({
                "cmd": "counterfactual",
                "lo_iters": lo_iters,
                "hi_iters": hi_iters,
                "disagreements": 0,
            })
        );
        return;
    }

    let total_jobs = disagreements.len() * trials;
    let cursor = AtomicUsize::new(0);
    let job_scores = Mutex::new(Vec::with_capacity(total_jobs));
    std::thread::scope(|scope| {
        for _ in 0..threads.min(total_jobs).max(1) {
            let (dex, catalog, pool, bundle, disagreements, cursor, job_scores) = (
                &dex,
                &catalog,
                &pool,
                &bundle,
                &disagreements,
                &cursor,
                &job_scores,
            );
            scope.spawn(move || loop {
                let job = cursor.fetch_add(1, Ordering::Relaxed);
                if job >= total_jobs {
                    break;
                }
                let f_idx = job / trials;
                let t_idx = job % trials;
                let dis = &disagreements[f_idx];
                let game = &bundle.games[dis.game_idx];
                let side = dis.side;
                let mut trial_rng = SplitMix64::new(
                    seed ^ (dis.pos_idx as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)
                        ^ (t_idx as u64).wrapping_mul(0xBF58_476D_1CE4_E5B9),
                );
                let b_seed = trial_rng.next();
                let arm_rngs = [trial_rng.next(), trial_rng.next()];

                let run_arm = |forced_str: &str| {
                    let (mut b, mut pl) =
                        replay_to_step(dex, catalog, pool, game, dis.step_idx, cont_iters);
                    b.reseed(b_seed);
                    for s in 0..2 {
                        pl[s].cfg = blind_cfg(cont_iters);
                        pl[s].rng = SplitMix64::new(pl[s].rng.0 ^ arm_rngs[s]);
                    }
                    let c0 = b.legal_choices(dex, 0);
                    let c1 = b.legal_choices(dex, 1);
                    let ch0 = (!c0.is_empty()).then(|| {
                        if side == 0 {
                            find_choice(dex, &c0, forced_str)
                        } else {
                            pl[0].choose(&b, dex, &c0)
                        }
                    });
                    let ch1 = (!c1.is_empty()).then(|| {
                        if side == 1 {
                            find_choice(dex, &c1, forced_str)
                        } else {
                            pl[1].choose(&b, dex, &c1)
                        }
                    });
                    b.apply_choices(dex, [ch0, ch1]).unwrap();
                    while b.outcome().is_none() && b.turn < 500 {
                        let c0 = b.legal_choices(dex, 0);
                        let c1 = b.legal_choices(dex, 1);
                        if c0.is_empty() && c1.is_empty() {
                            break;
                        }
                        let ch0 = (!c0.is_empty()).then(|| pl[0].choose(&b, dex, &c0));
                        let ch1 = (!c1.is_empty()).then(|| pl[1].choose(&b, dex, &c1));
                        b.apply_choices(dex, [ch0, ch1]).unwrap();
                    }
                    let p1 = match b.outcome() {
                        Some(Outcome::P1Win) => 1.0,
                        Some(Outcome::P2Win) => 0.0,
                        Some(Outcome::Tie) | None => 0.5,
                    };
                    let reply = if side == 0 { ch1 } else { ch0 };
                    (if side == 0 { p1 } else { 1.0 - p1 }, reply)
                };

                let (s_lo, reply) = run_arm(&dis.lo_choice);
                let (s_hi, other_reply) = run_arm(&dis.hi_choice);
                assert_eq!(reply, other_reply, "paired arms differ before the forced move");
                job_scores.lock().unwrap().push((f_idx, s_lo, s_hi, reply));
            });
        }
    });

    let scores = job_scores.into_inner().unwrap();
    let mut per_fork: Vec<Vec<f64>> = vec![Vec::with_capacity(trials); disagreements.len()];
    let mut per_fork_lo = vec![0.0; disagreements.len()];
    let mut per_fork_hi = vec![0.0; disagreements.len()];
    let mut reply_counts = vec![BTreeMap::<String, usize>::new(); disagreements.len()];
    for (f_idx, s_lo, s_hi, reply) in scores {
        if let Some(choice) = reply {
            *reply_counts[f_idx].entry(choice.to_input(&dex)).or_default() += 1;
        }
        per_fork[f_idx].push(s_hi - s_lo);
        per_fork_lo[f_idx] += s_lo;
        per_fork_hi[f_idx] += s_hi;
    }

    if let Some(parent) = out_path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent).unwrap();
    }
    let mut writer = BufWriter::new(File::create(&out_path).unwrap());
    let mut fork_diffs = Vec::with_capacity(disagreements.len());
    for (i, dis) in disagreements.iter().enumerate() {
        let diff = per_fork[i].iter().sum::<f64>() / trials as f64;
        fork_diffs.push(diff);
        let row = json!({
            "pos_idx": dis.pos_idx,
            "game_idx": dis.game_idx,
            "step_idx": dis.step_idx,
            "turn": dis.turn,
            "side": dis.side,
            "lo_choice": dis.lo_choice,
            "hi_choice": dis.hi_choice,
            "regime_lo": dis.regime_lo,
            "regime_hi": dis.regime_hi,
            "lo_mean": per_fork_lo[i] / trials as f64,
            "hi_mean": per_fork_hi[i] / trials as f64,
            "hi_minus_lo": diff,
            "pairs_hi_better": per_fork[i].iter().filter(|&&d| d > 0.0).count(),
            "pairs_lo_better": per_fork[i].iter().filter(|&&d| d < 0.0).count(),
            "reply_counts": reply_counts[i],
            "trials": trials,
        });
        serde_json::to_writer(&mut writer, &row).unwrap();
        writeln!(writer).unwrap();
    }
    writer.flush().unwrap();

    let (mean_diff, ci_lo, ci_hi) = bootstrap_ci(&fork_diffs, seed);
    let hi_better = fork_diffs.iter().filter(|&&d| d > 0.0).count();
    let lo_better = fork_diffs.iter().filter(|&&d| d < 0.0).count();
    let tied = fork_diffs.len() - hi_better - lo_better;

    println!(
        "{}",
        json!({
            "cmd": "counterfactual",
            "lo_iters": lo_iters,
            "hi_iters": hi_iters,
            "positions_evaluated": conv_rows.len(),
            "disagreements": disagreements.len(),
            "disagreement_rate": (disagreements.len() as f64 / conv_rows.len().max(1) as f64 * 10000.0).round() / 10000.0,
            "trials_per_disagreement": trials,
            "cont_iters": cont_iters,
            "hi_better": hi_better,
            "lo_better": lo_better,
            "tied": tied,
            "mean_hi_minus_lo": (mean_diff * 10000.0).round() / 10000.0,
            "ci95_hi_minus_lo": [(ci_lo * 10000.0).round() / 10000.0, (ci_hi * 10000.0).round() / 10000.0],
            "wall_s": (t0.elapsed().as_secs_f64() * 100.0).round() / 100.0,
            "out": out_path,
        })
    );
}

fn cmd_duel(args: &[String]) {
    let iters_a: u32 = flag(args, "--iters-a").expect("--iters-a N").parse().unwrap();
    let iters_b: u32 = flag(args, "--iters-b").expect("--iters-b N").parse().unwrap();
    let out_path = PathBuf::from(flag(args, "--out").expect("--out PATH"));
    let pairs: usize = flag(args, "--pairs").map_or(256, |v| v.parse().unwrap());
    let pair_start: usize = flag(args, "--pair-start").map_or(0, |v| v.parse().unwrap());
    let preview_iters: u32 = flag(args, "--preview-iters").map_or(243_000, |v| v.parse().unwrap());
    let seed: u64 = flag(args, "--seed").map_or(20261017003, |v| v.parse().unwrap());
    let threads: usize = flag(args, "--threads").map_or_else(default_threads, |v| v.parse().unwrap());
    let preview_cache = flag(args, "--preview-cache").map(PathBuf::from);
    let max_turns: u16 = flag(args, "--max-turns").map_or(500, |v| v.parse().unwrap());
    let independent_swap_seeds = args.iter().any(|a| a == "--independent-swap-seeds");

    let root = repo_root();
    let dex = load_dex();
    let catalog = load_catalog(&root);
    let pool = Arc::new(load_meta_pool(&root.join("data/belief-pool-v3/belief-pool.json")));

    let t0 = Instant::now();
    let total_pairs_needed = pair_start + pairs;
    let schedules = build_schedules(
        &dex,
        &catalog,
        &pool,
        total_pairs_needed,
        seed,
        preview_iters,
        threads,
        preview_cache.as_deref(),
    );

    let mut existing: HashMap<usize, Value> = HashMap::new();
    if out_path.exists() {
        for line in BufReader::new(File::open(&out_path).unwrap()).lines() {
            let line = line.unwrap();
            if line.trim().is_empty() {
                continue;
            }
            let row: Value = serde_json::from_str(&line).unwrap();
            if row["iters_a"].as_u64() == Some(u64::from(iters_a))
                && row["iters_b"].as_u64() == Some(u64::from(iters_b))
                && row["seed"].as_u64() == Some(seed)
                && row["preview_iters"].as_u64() == Some(u64::from(preview_iters))
            {
                if let Some(gi) = row["game_idx"].as_u64() {
                    existing.insert(gi as usize, row);
                }
            }
        }
    }

    let mut todo_games = Vec::new();
    for pair_idx in pair_start..total_pairs_needed {
        for swap in 0..2 {
            let game_idx = pair_idx * 2 + swap;
            if !existing.contains_key(&game_idx) {
                todo_games.push(game_idx);
            }
        }
    }

    if let Some(parent) = out_path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent).unwrap();
    }
    let append = out_path.exists() && !existing.is_empty();
    let file = if append {
        std::fs::OpenOptions::new().append(true).open(&out_path).unwrap()
    } else {
        File::create(&out_path).unwrap()
    };
    let writer = Arc::new(Mutex::new(BufWriter::new(file)));
    let new_rows: Mutex<Vec<Value>> = Mutex::new(Vec::new());
    let cursor = AtomicUsize::new(0);

    if !todo_games.is_empty() {
        std::thread::scope(|scope| {
            for _ in 0..threads.min(todo_games.len()).max(1) {
                let (dex, catalog, pool, schedules, todo_games, cursor, writer, new_rows) = (
                    &dex,
                    &catalog,
                    &pool,
                    &schedules,
                    &todo_games,
                    &cursor,
                    &writer,
                    &new_rows,
                );
                scope.spawn(move || loop {
                    let k = cursor.fetch_add(1, Ordering::Relaxed);
                    if k >= todo_games.len() {
                        break;
                    }
                    let game_idx = todo_games[k];
                    let pair_idx = game_idx / 2;
                    let a_is_p1 = game_idx % 2 == 0;
                    let sched = &schedules[pair_idx];
                    let (t1_idx, t2_idx, p1_prev, p2_prev) = (
                        sched.team_a_idx,
                        sched.team_b_idx,
                        sched.preview_a,
                        sched.preview_b,
                    );
                    let (p1_seed, p2_seed) = if independent_swap_seeds {
                        (
                            seed ^ (game_idx as u64).wrapping_mul(0xA24B_AED4_963E_E407),
                            seed ^ (game_idx as u64).wrapping_mul(0x9FB2_1C65_1E98_DF25),
                        )
                    } else {
                        (sched.seat_seed_p1, sched.seat_seed_p2)
                    };
                    let (p1_iters, p2_iters) = if a_is_p1 {
                        (iters_a, iters_b)
                    } else {
                        (iters_b, iters_a)
                    };
                    let mut p1 = TimedBlindWrapper::new(
                        BlindAgent::new(blind_cfg(p1_iters), pool.clone(), None, p1_seed)
                            .with_own_preview_policy(one_hot_preview_policy(p1_prev)),
                    );
                    let mut p2 = TimedBlindWrapper::new(
                        BlindAgent::new(blind_cfg(p2_iters), pool.clone(), None, p2_seed)
                            .with_own_preview_policy(one_hot_preview_policy(p2_prev)),
                    );
                    let mut battle = Battle::from_fixture(
                        dex,
                        &sched.battle_seed,
                        &catalog.teams[t1_idx].sets,
                        &catalog.teams[t2_idx].sets,
                    )
                    .unwrap();
                    battle.set_log_enabled(true);
                    let res =
                        play_game(dex, &mut battle, &mut [&mut p1, &mut p2], max_turns).unwrap();
                    let p1_score = outcome_p1_score(res);
                    let a_score = if a_is_p1 { p1_score } else { 1.0 - p1_score };
                    let (a_ns, b_ns) = if a_is_p1 {
                        (p1.searched_battle_ns, p2.searched_battle_ns)
                    } else {
                        (p2.searched_battle_ns, p1.searched_battle_ns)
                    };
                    let row = json!({
                        "game_idx": game_idx,
                        "pair_idx": pair_idx,
                        "a_is_p1": a_is_p1,
                        "iters_a": iters_a,
                        "iters_b": iters_b,
                        "seed": seed,
                        "preview_iters": preview_iters,
                        "a_score": a_score,
                        "turns": battle.turn,
                        "turn_cap": matches!(res, GameResult::TurnCapped),
                        "a_searched_ns": a_ns,
                        "b_searched_ns": b_ns,
                    });
                    {
                        let mut w = writer.lock().unwrap();
                        serde_json::to_writer(&mut *w, &row).unwrap();
                        writeln!(w).unwrap();
                        w.flush().unwrap();
                    }
                    new_rows.lock().unwrap().push(row);
                });
            }
        });
    }

    for row in new_rows.into_inner().unwrap() {
        let gi = row["game_idx"].as_u64().unwrap() as usize;
        existing.insert(gi, row);
    }

    let mut pair_scores = Vec::with_capacity(pairs);
    let mut wins = 0usize;
    let mut losses = 0usize;
    let mut ties = 0usize;
    let mut turn_caps = 0usize;
    let mut turns_sum = 0u64;
    let mut a_ns_all = Vec::new();
    let mut b_ns_all = Vec::new();

    for pair_idx in pair_start..total_pairs_needed {
        let g0 = &existing[&(pair_idx * 2)];
        let g1 = &existing[&(pair_idx * 2 + 1)];
        let s0 = g0["a_score"].as_f64().unwrap();
        let s1 = g1["a_score"].as_f64().unwrap();
        pair_scores.push(0.5 * (s0 + s1));
        for g in [g0, g1] {
            let s = g["a_score"].as_f64().unwrap();
            if s == 1.0 {
                wins += 1;
            } else if s == 0.0 {
                losses += 1;
            } else {
                ties += 1;
            }
            if g["turn_cap"].as_bool().unwrap_or(false) {
                turn_caps += 1;
            }
            turns_sum += g["turns"].as_u64().unwrap();
            for v in g["a_searched_ns"].as_array().unwrap() {
                a_ns_all.push(v.as_u64().unwrap());
            }
            for v in g["b_searched_ns"].as_array().unwrap() {
                b_ns_all.push(v.as_u64().unwrap());
            }
        }
    }

    let games = pairs * 2;
    let split_pairs = pair_scores.iter().filter(|&&s| s != 0.5).count();
    let (mean, ci_lo, ci_hi) = bootstrap_ci(&pair_scores, seed);
    let med_a = percentile_ms(&a_ns_all, 50);
    let med_b = percentile_ms(&b_ns_all, 50);
    println!(
        "{}",
        json!({
            "cmd": "duel",
            "iters_a": iters_a,
            "iters_b": iters_b,
            "pairs": pairs,
            "games": games,
            "wins": wins,
            "losses": losses,
            "ties": ties,
            "turn_caps": turn_caps,
            "split_pairs": split_pairs,
            "score": (mean * 10000.0).round() / 10000.0,
            "ci95": [(ci_lo * 10000.0).round() / 10000.0, (ci_hi * 10000.0).round() / 10000.0],
            "elo_diff": (elo_diff(mean) * 10.0).round() / 10.0,
            "avg_turns": (turns_sum as f64 / games as f64 * 10.0).round() / 10.0,
            "a_median_ms": (med_a * 100.0).round() / 100.0,
            "b_median_ms": (med_b * 100.0).round() / 100.0,
            "time_ratio": ((med_a / med_b.max(1e-9)) * 1000.0).round() / 1000.0,
            "a_p95_ms": (percentile_ms(&a_ns_all, 95) * 100.0).round() / 100.0,
            "b_p95_ms": (percentile_ms(&b_ns_all, 95) * 100.0).round() / 100.0,
            "wall_s": (t0.elapsed().as_secs_f64() * 100.0).round() / 100.0,
            "out": out_path,
        })
    );
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("collect-positions") => cmd_collect_positions(&args[1..]),
        Some("convergence") => cmd_convergence(&args[1..]),
        Some("counterfactual") => cmd_counterfactual(&args[1..]),
        Some("duel") => cmd_duel(&args[1..]),
        _ => {
            eprintln!("usage: budget_probe <collect-positions|convergence|counterfactual|duel> ...");
            std::process::exit(2);
        }
    }
}
