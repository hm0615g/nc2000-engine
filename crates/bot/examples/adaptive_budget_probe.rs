//! Compare a stopped search with the same tree continued to 243k.
//! Usage: adaptive_budget_probe CORPUS_DIR [GAMES] [THREADS] [SEED]
//!        adaptive_budget_probe duel [PAIRS] [THREADS] [SEED]
use nc2000_bot::adaptive_budget::BATTLE_CAP;
use nc2000_bot::corpus::{corpus_files, load_battle, load_sources, reconstruct_context_with_cfg};
use nc2000_bot::preview::load_meta_pool;
use nc2000_bot::smmcts::SelRule;
use nc2000_bot::{run_duel, Agent, BlindAgent, DuelSpec, RmConfig, SplitMix64};
use serde_json::json;
use std::collections::BTreeMap;
use std::path::Path;
use std::process::{self, Command};
use std::sync::Arc;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Mutex,
};

fn config() -> RmConfig {
    RmConfig {
        iterations: BATTLE_CAP,
        rule: SelRule::Ucb,
        c: 0.4,
        ..Default::default()
    }
}

fn duel(pairs: usize, threads: usize, seed: u64) {
    let root = conformance::fixture::repo_root();
    let dex = conformance::load_dex();
    let pool = Arc::new(load_meta_pool(
        &root.join("data/belief-pool-v3/belief-pool.json"),
    ));
    let catalog = load_meta_pool(&root.join("data/team-pool-v2/team-pool.json"));
    let teams: Vec<_> = catalog.teams.into_iter().map(|t| t.sets).collect();
    let agent = |seed| BlindAgent::new(config(), pool.clone(), None, seed);
    let adaptive = |seed| Box::new(agent(seed).with_adaptive_battle_budget()) as Box<dyn Agent>;
    let fixed = |seed| Box::new(agent(seed)) as Box<dyn Agent>;
    let result = run_duel(
        &dex,
        &teams,
        &adaptive,
        &fixed,
        DuelSpec {
            games: 2 * pairs,
            base_seed: seed,
            threads,
            max_turns: 500,
            progress: false,
            log_on: true,
            crn_agent_seeds: true,
        },
    );
    println!(
        "{}",
        json!({
            "seed": seed, "pairs": pairs, "wins": result.wins, "losses": result.losses,
            "ties": result.ties, "turn_caps": result.turn_caps, "score": result.score,
            "pair_ci95_half_width": result.ci95.is_finite().then_some(result.ci95),
            "adaptive_ms_per_choice": result.a_ms_per_move,
            "fixed_ms_per_choice": result.b_ms_per_move
        })
    );
}

fn main() {
    let mut args = std::env::args().skip(1);
    let mode = args.next().expect("expected corpus directory or duel");
    // The shared reconstruction code writes pool-index diagnostics for each
    // position. Keep them off the benchmark streams without muting failures.
    if mode != "duel" && mode != "--corpus-worker" {
        let log = std::env::temp_dir().join(format!("nc2000-budget-{}.log", process::id()));
        let status = Command::new(std::env::current_exe().unwrap())
            .arg("--corpus-worker")
            .arg(mode)
            .args(args)
            .stderr(std::fs::File::create_new(&log).unwrap())
            .status()
            .unwrap();
        assert!(
            status.success(),
            "corpus probe failed; see {}",
            log.display()
        );
        std::fs::remove_file(log).unwrap();
        return;
    }
    let dir = if mode == "duel" {
        None
    } else {
        Some(args.next().expect("missing corpus directory"))
    };
    let count: usize = args.next().map_or(96, |v| v.parse().unwrap());
    let threads: usize = args.next().map_or(4, |v| v.parse().unwrap());
    let seed: u64 = args.next().map_or(20261118, |v| v.parse().unwrap());
    assert!(threads > 0 && count > 0 && args.next().is_none());
    let Some(dir) = dir else {
        return duel(count, threads, seed);
    };
    let mut files = corpus_files(Path::new(&dir));
    assert!(count <= files.len());
    let mut rng = SplitMix64::new(seed);
    let jobs: Vec<_> = (0..count)
        .map(|_| {
            let i = rng.below(files.len());
            (files.swap_remove(i), rng.next())
        })
        .collect();
    let root = conformance::fixture::repo_root();
    let dex = conformance::load_dex();
    let src = load_sources(&dex, &root);
    let pool = load_meta_pool(&root.join("data/belief-pool-v3/belief-pool.json"));
    let cursor = AtomicUsize::new(0);
    let results = Mutex::new(Vec::new());
    std::thread::scope(|s| {
        for _ in 0..threads {
            s.spawn(|| {
                while let Some((path, game_seed)) = jobs.get(cursor.fetch_add(1, Ordering::Relaxed))
                {
                    let battle = load_battle(path);
                    if battle.decisions.is_empty() {
                        continue;
                    }
                    let mut rng = SplitMix64::new(*game_seed);
                    let d = &battle.decisions[rng.below(battle.decisions.len())];
                    let Some(ctx) = reconstruct_context_with_cfg(
                        &dex,
                        &src,
                        pool.clone(),
                        &battle.lines,
                        &battle.evidence,
                        d,
                        rng.next(),
                        config(),
                    ) else {
                        continue;
                    };
                    let mut agent = ctx.agent;

                    let at = agent.step_adaptive_battle(&dex, 81_000).unwrap();
                    let early = agent.best(&dex).unwrap();
                    if agent.step(&dex, 81_000 - at).unwrap() != 81_000 {
                        continue; // already bypassed by the fixed-budget protocol agent
                    }
                    let baseline = agent.best(&dex).unwrap();
                    let (cost, pick) = if at < 81_000 {
                        (at, early)
                    } else {
                        let cost = agent.step_adaptive_battle(&dex, BATTLE_CAP - at).unwrap();
                        (cost, agent.best(&dex).unwrap())
                    };
                    assert_eq!(
                        agent.step(&dex, BATTLE_CAP - agent.iterations()).unwrap(),
                        BATTLE_CAP
                    );
                    let full = agent.best(&dex).unwrap();
                    results
                        .lock()
                        .unwrap()
                        .push((cost, pick != full, baseline != full));
                }
            });
        }
    });
    let rows = results.into_inner().unwrap();
    assert!(!rows.is_empty(), "no searched positions");
    let mut histogram = BTreeMap::new();
    for &(cost, _, _) in &rows {
        *histogram.entry(cost).or_insert(0) += 1;
    }
    let total: u64 = rows.iter().map(|row| u64::from(row.0)).sum();
    println!(
        "{}",
        json!({
            "seed": seed, "games_sampled": count, "positions": rows.len(),
            "adaptive_iterations": histogram, "mean_iterations": total as f64 / rows.len() as f64,
            "adaptive_flips_vs_243k": rows.iter().filter(|r| r.1).count(),
            "fixed_81k_flips_vs_243k": rows.iter().filter(|r| r.2).count()
        })
    );
}
