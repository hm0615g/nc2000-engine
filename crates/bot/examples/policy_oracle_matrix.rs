//! Diagnose a root's frozen-search matrix from an archived, blind duel trace.
//! These are fixed-continuation search values, NOT optimal-play win-rate bounds.
use conformance::load_dex;
use nc2000_bot::{
    exact::solve_matrix_full, frozen::FrozenPolicy, player::PlayerChannel, preview::MetaPool,
    smmcts::SelRule, ProtocolAgent, RmConfig, SplitMix64,
};
use nc2000_engine::{
    battle::PokemonSet,
    state::{Battle, RequestState},
};
use serde_json::{json, Value};
use std::{path::PathBuf, time::Instant};

fn main() {
    let args: Vec<_> = std::env::args().collect();
    assert_eq!(args.len(), 3, "requests.json output.json");
    let requests: Value = serde_json::from_slice(&std::fs::read(&args[1]).unwrap()).unwrap();
    let dex = load_dex();
    let mut output = Vec::new();
    for request in requests.as_array().unwrap() {
        let start = Instant::now();
        let text = std::fs::read_to_string(request["trace"].as_str().unwrap()).unwrap();
        let records: Vec<Value> = text
            .lines()
            .map(|s| serde_json::from_str(s).unwrap())
            .collect();
        let setup = &records[0];
        let side = setup["blue_side"].as_u64().unwrap() as usize;
        let blue: Vec<PokemonSet> = serde_json::from_value(setup["blue_sets"].clone()).unwrap();
        let red: Vec<PokemonSet> = serde_json::from_value(setup["red_sets"].clone()).unwrap();
        let teams = if side == 0 {
            [&blue, &red]
        } else {
            [&red, &blue]
        };
        let mut battle = Battle::from_fixture(
            &dex,
            setup["battle_seed"].as_str().unwrap(),
            teams[0],
            teams[1],
        )
        .unwrap();
        battle.set_log_enabled(true);
        let pool: MetaPool =
            serde_json::from_slice(&std::fs::read(request["prior"].as_str().unwrap()).unwrap())
                .unwrap();
        let mut agent = ProtocolAgent::new(
            &dex,
            side,
            pool,
            RmConfig {
                iterations: 81000,
                c: 0.4,
                rule: SelRule::Ucb,
                ..Default::default()
            },
            setup["blue_seed"].as_u64().unwrap() as u32 as u64,
        );
        agent.set_own_team(blue);
        let mut channel = PlayerChannel::new(side);
        let turn = request["turn"].as_u64().unwrap() as u16;
        let mut found = false;
        for record in &records[1..] {
            let legal = [0, 1].map(|s| battle.legal_choices(&dex, s));
            if !legal[side].is_empty() {
                let frame = channel.frame(&mut battle, &dex).unwrap();
                for line in frame.lines {
                    agent.push_line(&dex, &line);
                }
                assert!(agent.on_request(&dex, &frame.request.to_string()).unwrap());
            }
            if battle.turn == turn && battle.request_state == RequestState::Move {
                agent.step(&dex, 81000).unwrap();
                let played = agent.best(&dex).unwrap();
                assert_eq!(
                    played,
                    record["blue_choice"].as_str().unwrap(),
                    "reconstructed decision"
                );
                let search = agent.search().unwrap();
                let expected = record["blue_search"]["actions"].as_array().unwrap();
                for (i, a) in search.actions().iter().enumerate() {
                    assert_eq!(a.to_input(&dex), expected[i]["input"]);
                    assert_eq!(
                        search.visits()[i] as u64,
                        expected[i]["visits"].as_u64().unwrap(),
                        "search evidence must reproduce"
                    );
                }
                let belief = agent.belief().unwrap();
                let observer = agent.observer().unwrap();
                let seed = request["seed"].as_u64().unwrap();
                let mut rng = SplitMix64::new(seed ^ 0x6d6174726978);
                let mut simulated =
                    belief.determinize(&dex, agent.battle().unwrap(), observer, &mut rng);
                let mut replies = simulated.legal_choices(&dex, 1 - side);
                // Only fixed input policies legal across the sampled private worlds.
                // This does not include arbitrary type-contingent reply policies.
                for _ in 0..4096 {
                    simulated =
                        belief.determinize(&dex, agent.battle().unwrap(), observer, &mut rng);
                    let legal = simulated.legal_choices(&dex, 1 - side);
                    replies.retain(|a| legal.contains(a));
                }
                assert!(!replies.is_empty());
                let rows = search.actions();
                let samples = request["samples"].as_u64().unwrap() as u32;
                assert!(samples >= 2, "at least two samples per cell");
                let mut variants = Vec::new();
                for (label, policy) in [
                    ("most-visited", FrozenPolicy::MostVisited),
                    ("visit-sampled", FrozenPolicy::SampleVisits),
                ] {
                    let mut cells = Vec::new();
                    let mut values = Vec::new();
                    for (i, &a) in rows.iter().enumerate() {
                        for (j, &b) in replies.iter().enumerate() {
                            let mut joint = [None, None];
                            joint[side] = Some(a);
                            joint[1 - side] = Some(b);
                            let mut sum = 0.;
                            let mut square = 0.;
                            let mut terminal = 0u32;
                            for k in 0..samples {
                                let draw_seed = seed
                                    ^ (k as u64).wrapping_mul(0x94d049bb133111eb)
                                    ^ ((i * replies.len() + j) as u64)
                                        .wrapping_mul(0xa24baed4963ee407);
                                let result = search.evaluate_frozen_joint(
                                    &dex,
                                    belief,
                                    observer,
                                    joint,
                                    [policy; 2],
                                    draw_seed,
                                    &mut |_| {},
                                );
                                let v = if side == 0 {
                                    result.reward0
                                } else {
                                    1. - result.reward0
                                };
                                sum += v;
                                square += v * v;
                                terminal += u32::from(result.terminal);
                            }
                            let mean = sum / samples as f64;
                            let se = ((square / samples as f64 - mean * mean).max(0.)
                                / (samples - 1) as f64)
                                .sqrt();
                            cells.push(
                                json!({"row":i,"column":j,"mean":mean,"se":se,"terminal":terminal}),
                            );
                            values.push(mean);
                        }
                    }
                    let solution = solve_matrix_full(&values, rows.len(), replies.len());
                    let row_min: Vec<f64> = values
                        .chunks(replies.len())
                        .map(|r| r.iter().copied().fold(1., f64::min))
                        .collect();
                    let pure = row_min.iter().copied().fold(0., f64::max);
                    let best_pure = row_min.iter().position(|v| *v == pure).unwrap();
                    let col_max: Vec<f64> = (0..replies.len())
                        .map(|j| {
                            (0..rows.len())
                                .map(|i| values[i * replies.len() + j])
                                .fold(0., f64::max)
                        })
                        .collect();
                    let pure_upper = col_max.iter().copied().fold(1., f64::min);
                    variants.push(json!({"continuation":label,"cells":cells,"row_worst_values":row_min,
                        "best_pure":best_pure,"pure_security":pure,"pure_upper":pure_upper,
                        "mixed_value":solution.value,"matrix_solver_gap":solution.gap,
                        "row_mix":solution.x,"column_mix":solution.y,"estimated_mix_gain":solution.value-pure}));
                }
                output.push(json!({"id":request["id"],"turn":turn,"played":played,
                    "rows":rows.iter().map(|a|a.to_input(&dex)).collect::<Vec<_>>(),
                    "columns":replies.iter().map(|a|a.to_input(&dex)).collect::<Vec<_>>(),
                    "samples_per_cell":samples,"request":request,"variants":variants,
                    "scope":"Blind reconstructed root, frozen sampled continuation. Approximate search-payoff matrix; not a full-game equilibrium certificate. Fixed input reply policies only.",
                    "elapsed_seconds":start.elapsed().as_secs_f64()}));
                found = true;
                break;
            }
            let mut joint = [None, None];
            for (s, key) in [(side, "blue_choice"), (1 - side, "red_choice")] {
                if let Some(input) = record[key].as_str() {
                    joint[s] = Some(nc2000_bot::fork::find_choice(&dex, &legal[s], input).unwrap());
                }
            }
            let log_start = battle.log.len();
            battle.apply_choices(&dex, joint).unwrap();
            assert_eq!(
                json!(&battle.log[log_start..]),
                record["engine_log"],
                "replay engine log"
            );
        }
        assert!(found, "target not reached");
        eprintln!(
            "{} complete in {:.1}s",
            request["id"],
            start.elapsed().as_secs_f64()
        );
        let path = PathBuf::from(&args[2]);
        std::fs::write(path, serde_json::to_vec_pretty(&output).unwrap()).unwrap();
    }
}
