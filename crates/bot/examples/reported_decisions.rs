use conformance::{fixture::repo_root, load_dex};
use nc2000_bot::{
    analysis,
    blind::BlindSearch,
    corpus::{load_battle, load_sources, reconstruct_context_with_cfg},
    import::ProtocolAgent,
    preview::load_meta_pool,
    smmcts::{RmConfig, SelRule},
};
use nc2000_engine::{battle::PokemonSet, dex::toid};
use serde_json::{json, Value};
use std::{io::Write, path::PathBuf};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let arg = |key: &str, default: &str| -> String {
        args.iter()
            .position(|s| s == key)
            .map(|i| args[i + 1].clone())
            .unwrap_or_else(|| default.into())
    };
    let root = repo_root();
    let profiles: Value = serde_json::from_str(
        &std::fs::read_to_string(root.join("data/search-profiles.json")).unwrap(),
    )
    .unwrap();
    let profile = arg("--profile", "blind");
    let iters: u32 = arg("--iters", &profiles[&profile]["iterations"].to_string())
        .parse()
        .unwrap();
    let c: f64 = arg("--c", &profiles[&profile]["c"].to_string())
        .parse()
        .unwrap();
    let side: usize = arg("--side", "1").parse().unwrap();
    let seeds: u64 = arg("--seeds", "1").parse().unwrap();
    let first_seed: u64 = arg("--seed", "1").parse().unwrap();
    assert!(side < 2 && seeds > 0 && iters > 0 && c.is_finite() && c > 0.0);
    let turns: Vec<u16> = arg("--turns", "11,18,24,26")
        .split(',')
        .map(|s| s.parse().unwrap())
        .collect();
    let out = PathBuf::from(arg("--out", "tmp/reported-decisions"));
    std::fs::create_dir_all(&out).unwrap();
    let dex = load_dex();
    let mut src = load_sources(&dex, &root);
    let own: Vec<PokemonSet> = serde_json::from_str(
        &std::fs::read_to_string(arg("--own-team", "")).expect("--own-team FILE is required"),
    )
    .unwrap();
    for set in &own {
        let value = serde_json::to_value(set).unwrap();
        let id = dex
            .species
            .id(&toid(value["species"].as_str().unwrap()))
            .unwrap();
        src.by_species.insert(id, vec![value]);
    }
    let log_path = PathBuf::from(arg("--log", ""));
    let log = load_battle(&log_path);
    let pool_path = root.join(arg("--pool", "data/meta-pool-v0/meta-pool.json"));
    let pool = load_meta_pool(&pool_path);
    let pinned: Option<Vec<PokemonSet>> = args
        .iter()
        .position(|s| s == "--opponent-team")
        .map(|i| serde_json::from_str(&std::fs::read_to_string(&args[i + 1]).unwrap()).unwrap());
    assert!(
        profile != "open" || pinned.is_some(),
        "OpenSheet requires --opponent-team"
    );
    let balanced = args.iter().any(|s| s == "--balanced");
    let rollout_turns: u16 = arg("--rollout-turns", "8").parse().unwrap();
    let m16c = args.iter().any(|s| s == "--m16c");
    let key_no_damage = args.iter().any(|s| s == "--key-no-damage");
    let cfg = RmConfig {
        iterations: iters,
        rule: SelRule::Ucb,
        c,
        rollout_m16c: m16c,
        key_no_damage,
        playout: nc2000_bot::mcts::Playout::Heavy {
            eps: 0.2,
            turns: rollout_turns,
            weights: nc2000_bot::eval::EvalWeights::default(),
        },
        ..RmConfig::default()
    };
    let mut output = std::io::BufWriter::new(std::io::stdout().lock());
    for turn in turns {
        let d = log
            .decisions
            .iter()
            .find(|d| d.side == side && d.turn == turn)
            .unwrap_or_else(|| panic!("missing decision at turn {turn}"));
        for seed in first_seed..first_seed + seeds {
            let rec = reconstruct_context_with_cfg(
                &dex,
                &src,
                pool.clone(),
                &log.lines,
                &log.evidence,
                d,
                seed,
                cfg.clone(),
            )
            .expect("reconstruction");
            assert!(!rec.imputed_pick, "own selection is incomplete");
            assert!(
                rec.provenance.iter().all(|s| *s == "cand-full"),
                "own set contradicts log"
            );
            let mut agent = rec.agent;
            let spec = agent.to_position_spec(&dex).unwrap();
            if let Some(team) = &pinned {
                let mut open = ProtocolAgent::new(&dex, side, pool.clone(), cfg.clone(), seed);
                open.pin_opponent(team.clone());
                open.set_position(&dex, &spec).unwrap();
                agent = open;
            }
            if seed == first_seed {
                std::fs::write(
                    out.join(format!("turn-{turn}.json")),
                    serde_json::to_string_pretty(&spec).unwrap(),
                )
                .unwrap();
            }
            let start = std::time::Instant::now();
            let result = if balanced {
                let mut search =
                    BlindSearch::new(agent.battle().unwrap(), &dex, cfg.clone(), side, seed);
                let n = search.actions().len();
                for cycle in 0..iters {
                    for i in 0..n {
                        search.step_forced(
                            &dex,
                            agent.belief().unwrap(),
                            agent.observer().unwrap(),
                            (i + cycle as usize) % n,
                        );
                    }
                }
                json!({"estimand":"shared-opponent-root-search-reward",
                    "actions":search.actions().iter().map(|a| a.to_input(&dex)).collect::<Vec<_>>(),
                    "means":search.means(), "visits":search.visits(),
                    "matrix":search.root_matrix().iter().map(|(i,a,n,v)|
                        json!([i,a.to_input(&dex),n,v])).collect::<Vec<_>>()})
            } else {
                agent.step(&dex, iters).unwrap();
                let best = agent.best(&dex).unwrap();
                json!({"best":best, "analysis":analysis::report(&agent, &dex, 0, seed)})
            };
            writeln!(
                output,
                "{}",
                json!({"turn":turn,"seed":seed,"c":c,"iters":iters,"profile":profile,
                "pinned_opponent":pinned.is_some(), "balanced":balanced,
                "rollout_turns":rollout_turns,"m16c":m16c,"key_no_damage":key_no_damage,
                "elapsed_ms":start.elapsed().as_millis(),"result":result})
            )
            .unwrap();
            output.flush().unwrap();
        }
    }
}
