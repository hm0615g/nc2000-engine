use conformance::{fixture::repo_root, load_dex};
use nc2000_bot::{
    agent::{Agent, MaxDamageAgent, RandomAgent},
    blind::OpenAgent,
    position::{synthesize_spec, PositionSpec},
    preview::load_meta_pool,
    rng::SplitMix64,
    smmcts::{RmAgent, RmConfig, SelRule},
};
use nc2000_engine::{
    battle::{enumerate::enumerate_step, Outcome, PokemonSet},
    prng::{BattleRng, Prng},
};
use serde_json::json;
use std::io::Write;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let arg = |key: &str, default: &str| -> String {
        args.iter()
            .position(|s| s == key)
            .map(|i| args[i + 1].clone())
            .unwrap_or_else(|| default.into())
    };
    let spec =
        PositionSpec::parse(&std::fs::read_to_string(arg("--position", "")).unwrap()).unwrap();
    let sets: Vec<PokemonSet> =
        serde_json::from_str(&std::fs::read_to_string(arg("--opponent-team", "")).unwrap())
            .unwrap();
    let trials: usize = arg("--trials", "32").parse().unwrap();
    let first_trial: usize = arg("--start-trial", "0").parse().unwrap();
    let seed: u64 = arg("--seed", "1").parse().unwrap();
    let iters: u32 = arg("--iters", "300").parse().unwrap();
    let foe_iters: u32 = arg("--foe-iters", &iters.to_string()).parse().unwrap();
    let reply = arg("--reply", "search");
    let only = arg("--action", "all");
    let tail = arg("--tail", "search");
    let policy = arg("--policy", "skuct");
    assert!(trials > 0 && iters > 0 && foe_iters > 0);
    assert!(policy == "skuct" || policy == "open", "unknown policy");
    let dex = load_dex();
    let pool = load_meta_pool(&repo_root().join("data/meta-pool-v0/meta-pool.json"));
    let mut initial = synthesize_spec(&dex, &spec, &pool, Some(&sets), seed).unwrap();
    let actions = initial.legal_choices(&dex, spec.side);
    assert!(
        only == "all" || actions.iter().any(|c| c.to_input(&dex) == only),
        "illegal action {only}"
    );
    let cfg = |iterations| RmConfig {
        iterations,
        rule: SelRule::Ucb,
        ..RmConfig::default()
    };
    let search_agent = |iterations, seed| -> Box<dyn Agent> {
        if policy == "open" {
            Box::new(OpenAgent::new(cfg(iterations), None, seed))
        } else {
            Box::new(RmAgent::new(cfg(iterations), seed))
        }
    };
    let mut output = std::io::BufWriter::new(std::io::stdout().lock());
    if args.iter().any(|s| s == "--one-step") {
        let replies = initial.legal_choices(&dex, 1 - spec.side);
        assert!(
            reply == "all" || replies.iter().any(|c| c.to_input(&dex) == reply),
            "illegal reply {reply}"
        );
        let me = initial.active_id(spec.side).unwrap();
        let target = initial.active_id(1 - spec.side).unwrap();
        for &action in &actions {
            if only != "all" && action.to_input(&dex) != only {
                continue;
            }
            for &response in &replies {
                if reply != "all" && response.to_input(&dex) != reply {
                    continue;
                }
                let mut joint = [None, None];
                joint[spec.side] = Some(action);
                joint[1 - spec.side] = Some(response);
                let step = enumerate_step(&dex, &initial, joint, 200_000).expect("enumeration cap");
                let mass: f64 = step.leaves.iter().map(|l| l.prob).sum();
                assert!((mass - 1.0).abs() < 1e-8);
                let foe_ko: f64 = step
                    .leaves
                    .iter()
                    .filter(|l| l.battle.poke(target).fainted)
                    .map(|l| l.prob)
                    .sum();
                let self_ko: f64 = step
                    .leaves
                    .iter()
                    .filter(|l| l.battle.poke(me).fainted)
                    .map(|l| l.prob)
                    .sum();
                let foe_hp: f64 = step
                    .leaves
                    .iter()
                    .map(|l| l.prob * l.battle.poke(target).hp as f64)
                    .sum();
                writeln!(
                    output,
                    "{}",
                    json!({"turn":spec.turn,"action":action.to_input(&dex),
                    "reply":response.to_input(&dex),"estimand":"exact-one-step-state",
                    "original_target_ko":foe_ko,"actor_ko":self_ko,"original_target_expected_hp":foe_hp,
                    "runs":step.runs,"leaves":step.leaves.len(),"mass":mass})
                )
                .unwrap();
            }
        }
        return;
    }
    let mut rng = SplitMix64::new(seed);
    for _ in 0..first_trial {
        rng.next();
        rng.next();
        rng.next();
    }
    for trial in first_trial..first_trial.checked_add(trials).unwrap() {
        let battle_seed = rng.next();
        let my_seed = rng.next();
        let foe_seed = rng.next();
        for &action in &actions {
            if only != "all" && action.to_input(&dex) != only {
                continue;
            }
            let mut b = initial.clone();
            b.set_log_enabled(policy == "open");
            b.prng = BattleRng::seeded(Prng::new(battle_seed));
            let mut mine: Box<dyn Agent> = match tail.as_str() {
                "search" => search_agent(iters, my_seed),
                "greedy" => Box::new(MaxDamageAgent::conformant()),
                "random" => Box::new(RandomAgent::new(my_seed)),
                _ => panic!("unknown tail"),
            };
            let mut foe = search_agent(foe_iters, foe_seed);
            if policy == "open" {
                if tail == "search" {
                    mine.choose(&b, &dex, spec.side, &[action]);
                }
                let legal = b.legal_choices(&dex, 1 - spec.side);
                if let Some(&choice) = legal.first() {
                    foe.choose(&b, &dex, 1 - spec.side, &[choice]);
                }
            }
            let mut steps = 0;
            let start = std::time::Instant::now();
            let (score, outcome) = loop {
                if let Some(o) = b.outcome() {
                    break match (o, spec.side) {
                        (Outcome::P1Win, 0) | (Outcome::P2Win, 1) => (Some(1.0), "win"),
                        (Outcome::Tie, _) => (Some(0.5), "tie"),
                        _ => (Some(0.0), "loss"),
                    };
                }
                if b.turn > 1100 || steps > 3300 {
                    break (None, "cap");
                }
                let mut choices = [None, None];
                for s in 0..2 {
                    let legal = b.legal_choices(&dex, s);
                    if legal.is_empty() {
                        continue;
                    }
                    let pick = if steps == 0 && s == spec.side {
                        action
                    } else if steps == 0 && reply != "search" && s != spec.side {
                        *legal
                            .iter()
                            .find(|c| c.to_input(&dex) == reply)
                            .unwrap_or_else(|| panic!("illegal reply {reply}"))
                    } else if s == spec.side {
                        mine.choose(&b, &dex, s, &legal)
                    } else {
                        foe.choose(&b, &dex, s, &legal)
                    };
                    assert!(legal.contains(&pick));
                    choices[s] = Some(pick);
                }
                assert!(choices != [None, None]);
                b.apply_choices(&dex, choices).unwrap();
                steps += 1;
            };
            writeln!(
                output,
                "{}",
                json!({"turn":spec.turn,"trial":trial,"seed":seed,
                "battle_seed":battle_seed,"action":action.to_input(&dex),"reply":reply,
                "tail":tail,"policy":policy,"iters":iters,"foe_iters":foe_iters,
                "score":score,"outcome":outcome,"final_turn":b.turn,"steps":steps,
                "elapsed_ms":start.elapsed().as_millis()})
            )
            .unwrap();
            output.flush().unwrap();
        }
    }
}
