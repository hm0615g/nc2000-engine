use conformance::{fixture::repo_root, load_dex};
use nc2000_bot::{
    frozen::FrozenPolicy,
    import::ProtocolAgent,
    position::PositionSpec,
    preview::load_meta_pool,
    rng::SplitMix64,
    smmcts::{RmConfig, SelRule},
};
use nc2000_engine::battle::PokemonSet;
use serde_json::json;
use std::{collections::BTreeMap, io::Write};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let arg = |key: &str, default: &str| {
        args.iter()
            .position(|s| s == key)
            .map(|i| args[i + 1].clone())
            .unwrap_or_else(|| default.into())
    };
    let first: u64 = arg("--seed", "61001").parse().unwrap();
    let seeds: u64 = arg("--seeds", "1").parse().unwrap();
    let iterations: u32 = arg("--iters", "30000").parse().unwrap();
    let evaluation: u32 = arg("--eval-iters", "7500").parse().unwrap();
    assert!(iterations > evaluation && evaluation > 0 && seeds > 0);
    let train = iterations - evaluation;
    let dex = load_dex();
    let spec =
        PositionSpec::parse(&std::fs::read_to_string(arg("--position", "")).unwrap()).unwrap();
    let team_path = arg("--opponent-team", "");
    let team: Option<Vec<PokemonSet>> = (!team_path.is_empty())
        .then(|| serde_json::from_str(&std::fs::read_to_string(&team_path).unwrap()).unwrap());
    let pool = load_meta_pool(&repo_root().join("data/meta-pool-v0/meta-pool.json"));
    let cfg = RmConfig {
        iterations,
        rule: SelRule::Ucb,
        c: arg("--c", "1").parse().unwrap(),
        ..RmConfig::default()
    };
    let mut stdout = std::io::BufWriter::new(std::io::stdout().lock());
    for seed in first..first + seeds {
        let start = std::time::Instant::now();
        let mut agent = ProtocolAgent::new(&dex, spec.side, pool.clone(), cfg.clone(), seed);
        if let Some(team) = &team {
            agent.pin_opponent(team.clone());
        }
        agent.set_position(&dex, &spec).unwrap();
        agent.step(&dex, train).unwrap();
        let training_ms = start.elapsed().as_millis();
        let search = agent.search().unwrap();
        let actions = search.actions();
        assert!(evaluation >= actions.len() as u32);
        let eligible: Vec<usize> = (0..actions.len())
            .filter(|&a| !search.dominated()[a])
            .collect();
        let eligible = if eligible.is_empty() {
            (0..actions.len()).collect()
        } else {
            eligible
        };
        if args.iter().any(|s| s == "--matrix") {
            assert!(
                team.is_some(),
                "root matrix requires a pinned opponent sheet"
            );
            let mut base = agent.resynthesize(&dex, seed).unwrap();
            let replies = base.legal_choices(&dex, 1 - spec.side);
            let cells = actions.len() * replies.len();
            assert!(evaluation >= cells as u32);
            let mut counts = vec![0u32; cells];
            let mut sums = vec![0.0; cells];
            let mut terminals = vec![0u32; cells];
            let mut rng = SplitMix64::new(seed ^ 0xd1b54a32d192ed03);
            let eval_start = std::time::Instant::now();
            for round in 0..evaluation.div_ceil(cells as u32) {
                let eval_seed = rng.next();
                for (a, &action) in actions.iter().enumerate() {
                    for (b, &reply) in replies.iter().enumerate() {
                        let cell = a * replies.len() + b;
                        if round * cells as u32 + cell as u32 >= evaluation {
                            continue;
                        }
                        let mut forced = [None, None];
                        forced[spec.side] = Some(action);
                        forced[1 - spec.side] = Some(reply);
                        let result = search.evaluate_frozen_joint(
                            &dex,
                            agent.belief().unwrap(),
                            agent.observer().unwrap(),
                            forced,
                            [FrozenPolicy::MostVisited; 2],
                            eval_seed,
                            &mut |_| {},
                        );
                        counts[cell] += 1;
                        sums[cell] += if spec.side == 0 {
                            result.reward0
                        } else {
                            1.0 - result.reward0
                        };
                        terminals[cell] += u32::from(result.terminal);
                    }
                }
            }
            let means: Vec<f64> = sums
                .iter()
                .zip(&counts)
                .map(|(w, n)| w / *n as f64)
                .collect();
            let reduced: Vec<f64> = eligible
                .iter()
                .flat_map(|&a| {
                    means[a * replies.len()..(a + 1) * replies.len()]
                        .iter()
                        .copied()
                })
                .collect();
            let (own, foe) =
                nc2000_bot::smmcts::solve_rm_plus(&reduced, [eligible.len(), replies.len()], 2000);
            let best = eligible[(0..own.len())
                .max_by(|&a, &b| own[a].total_cmp(&own[b]))
                .unwrap()];
            writeln!(stdout, "{}", json!({"seed":seed,"mode":"open","turn":spec.turn,"arm":"matrix",
                "training":train,"evaluation":evaluation,"training_ms":training_ms,"evaluation_ms":eval_start.elapsed().as_millis(),
                "modal_action":actions[best].to_input(&dex),"trained_best":search.best().map(|c|c.to_input(&dex)),
                "actions":actions.iter().map(|c|c.to_input(&dex)).collect::<Vec<_>>(),
                "replies":replies.iter().map(|c|c.to_input(&dex)).collect::<Vec<_>>(),
                "eligible":eligible,"policy":own,"foe_policy":foe,"n":counts,"means":means,"terminal":terminals
            })).unwrap();
            stdout.flush().unwrap();
        }
        let arms: Vec<(&str, [FrozenPolicy; 2])> = if args.iter().any(|s| s == "--ablate") {
            vec![
                ("execute_both", [FrozenPolicy::MostVisited; 2]),
                (
                    "sample_ours",
                    std::array::from_fn(|s| {
                        if s == spec.side {
                            FrozenPolicy::SampleVisits
                        } else {
                            FrozenPolicy::MostVisited
                        }
                    }),
                ),
                (
                    "sample_foe",
                    std::array::from_fn(|s| {
                        if s != spec.side {
                            FrozenPolicy::SampleVisits
                        } else {
                            FrozenPolicy::MostVisited
                        }
                    }),
                ),
                ("sample_both", [FrozenPolicy::SampleVisits; 2]),
            ]
        } else {
            vec![("execute_both", [FrozenPolicy::MostVisited; 2])]
        };
        for (arm, policies) in arms {
            let eval_start = std::time::Instant::now();
            let mut seeds_rng = SplitMix64::new(seed ^ 0xd1b54a32d192ed03);
            let rounds = evaluation.div_ceil(actions.len() as u32);
            let mut counts = vec![0u32; actions.len()];
            let mut sums = vec![0.0; actions.len()];
            let mut terminals = vec![0u32; actions.len()];
            let mut tree_choices = vec![0u64; actions.len()];
            let mut starmie: Vec<BTreeMap<String, u32>> = vec![BTreeMap::new(); actions.len()];
            let mut starmie_joints: Vec<BTreeMap<String, u32>> =
                vec![BTreeMap::new(); actions.len()];
            let mut roots: Vec<BTreeMap<String, u32>> = vec![BTreeMap::new(); actions.len()];
            for round in 0..rounds {
                let rollout_seed = seeds_rng.next();
                for (a, &action) in actions.iter().enumerate() {
                    if round * actions.len() as u32 + a as u32 >= evaluation {
                        continue;
                    }
                    let mut root = true;
                    let mut found_starmie = false;
                    let result = search.evaluate_frozen(
                        &dex,
                        agent.belief().unwrap(),
                        agent.observer().unwrap(),
                        action,
                        policies,
                        rollout_seed,
                        &mut |event| {
                            let reply = event.chosen[1 - spec.side]
                                .map(|c| c.to_input(&dex))
                                .unwrap_or_default();
                            if root {
                                *roots[a].entry(reply.clone()).or_default() += 1;
                                root = false;
                            }
                            if let (Some(foe), Some(own)) = (
                                event.battle.active_id(1 - spec.side),
                                event.battle.active_id(spec.side),
                            ) {
                                let (foe, own) = (event.battle.poke(foe), event.battle.poke(own));
                                if !found_starmie
                                    && dex.species.key(foe.species) == "starmie"
                                    && dex.species.key(own.species) == "marowak"
                                    && foe.hp == foe.maxhp
                                    && own.hp > 0
                                {
                                    let own_action = event.chosen[spec.side]
                                        .map(|c| c.to_input(&dex))
                                        .unwrap_or_default();
                                    *starmie_joints[a]
                                        .entry(format!("{own_action} / {reply}"))
                                        .or_default() += 1;
                                    *starmie[a].entry(reply).or_default() += 1;
                                    found_starmie = true;
                                }
                            }
                        },
                    );
                    counts[a] += 1;
                    sums[a] += if spec.side == 0 {
                        result.reward0
                    } else {
                        1.0 - result.reward0
                    };
                    terminals[a] += u32::from(result.terminal);
                    tree_choices[a] += result.prefix_choices as u64;
                }
            }
            let means: Vec<f64> = sums
                .iter()
                .zip(&counts)
                .map(|(w, n)| w / *n as f64)
                .collect();
            let best = *eligible
                .iter()
                .max_by(|&&a, &&b| means[a].total_cmp(&means[b]))
                .unwrap();
            writeln!(stdout, "{}", json!({"seed":seed,"mode":if team.is_some(){"open"}else{"blind"},
                "turn":spec.turn,"arm":arm,"training":train,"evaluation":evaluation,"training_ms":training_ms,
                "evaluation_ms":eval_start.elapsed().as_millis(),"best":actions[best].to_input(&dex),
                "trained_best":search.best().map(|c|c.to_input(&dex)),"nodes":search.node_count(),
                "actions":actions.iter().enumerate().map(|(a,c)|json!({"action":c.to_input(&dex),
                    "n":counts[a],"mean":means[a],"terminal":terminals[a],"prefix_choices":tree_choices[a],
                    "root_replies":roots[a],"first_full_starmie":starmie[a],"trained_n":search.visits()[a],
                    "first_full_starmie_joints":starmie_joints[a],
                    "trained_mean":search.means()[a]})).collect::<Vec<_>>()
            })).unwrap();
            stdout.flush().unwrap();
        }
        agent.step(&dex, evaluation).unwrap();
        let search = agent.search().unwrap();
        writeln!(stdout, "{}", json!({"seed":seed,"turn":spec.turn,"arm":"baseline",
            "iterations":iterations,"best":search.best().map(|c|c.to_input(&dex)),
            "actions":search.actions().iter().enumerate().map(|(a,c)|json!({"action":c.to_input(&dex),
                "n":search.visits()[a],"mean":search.means()[a]})).collect::<Vec<_>>()
        })).unwrap();
        stdout.flush().unwrap();
    }
}
