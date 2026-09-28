use conformance::{fixture::repo_root, load_dex};
use nc2000_bot::{
    corpus::{load_battle, load_sources, reconstruct_context_with_cfg},
    import::ProtocolAgent,
    mcts::{playout_pick, Playout},
    position::PositionSpec,
    preview::load_meta_pool,
    smmcts::{RmConfig, SearchTrace, SelRule},
};
use nc2000_engine::{
    battle::{PokemonSet, SearchChoice},
    dex::Dex,
    state::Battle,
};
use serde_json::{json, Value};
use std::{collections::BTreeMap, path::PathBuf};

fn count(b: &Battle, dex: &Dex, side: usize) -> Option<i32> {
    b.poke(b.active_id(side)?)
        .volatile(dex.conds_id("perishsong")?)
        .and_then(|v| v.duration)
}

fn snapshot(b: &Battle, dex: &Dex) -> Value {
    json!({"turn": b.turn, "sides": (0..2).map(|s| {
        let p = b.poke(b.active_id(s).unwrap());
        json!({"species":dex.species.key(p.species),"hp":p.hp,"maxhp":p.maxhp,
            "perish":count(b,dex,s),"trapped":p.trapped,
            "moves":p.move_slots.iter().map(|m|dex.moves.key(m.id)).collect::<Vec<_>>()})
    }).collect::<Vec<_>>()})
}

fn events(
    b: &Battle,
    dex: &Dex,
    choices: &[Vec<SearchChoice>; 2],
    chosen: [Option<SearchChoice>; 2],
    phase: &str,
    analyzed: usize,
    exploration: [Option<bool>; 2],
) -> Vec<Value> {
    let mut out = Vec::new();
    for s in 0..2 {
        let (Some(id), Some(foe), Some(action)) = (b.active_id(s), b.active_id(1 - s), chosen[s])
        else {
            continue;
        };
        let p = b.poke(id);
        let opp = b.poke(foe);
        if p.fainted || opp.fainted {
            continue;
        }
        let song = count(b, dex, s);
        let foe_song = count(b, dex, 1 - s);
        let kit = p
            .move_slots
            .iter()
            .any(|m| dex.moves.key(m.id) == "perishsong");
        let foe_kit = opp
            .move_slots
            .iter()
            .any(|m| dex.moves.key(m.id) == "perishsong");
        if !kit && !foe_kit && song.is_none() && foe_song.is_none() {
            continue;
        }
        out.push(json!({"phase":phase,"side":s,"own":s==analyzed,
            "species":dex.species.key(p.species),"foe":dex.species.key(opp.species),
            "perish":song,"foe_perish":foe_song,"trapped":p.trapped,"foe_trapped":opp.trapped,
            "switch_available":choices[s].iter().any(|a|matches!(a,SearchChoice::Switch(_))),
            "move_available":choices[s].iter().any(|a|matches!(a,SearchChoice::Move(_))),
            "action":action.to_input(dex),"below_best_mean":exploration[s]}));
    }
    out
}

#[derive(Default)]
struct Aggregate {
    n: u64,
    reward: f64,
    first_iteration: u32,
    first_step: usize,
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let arg = |key: &str, default: &str| {
        args.iter()
            .position(|a| a == key)
            .map(|i| args[i + 1].clone())
            .unwrap_or_else(|| default.into())
    };
    let root = repo_root();
    let dex = load_dex();
    let pool = load_meta_pool(&root.join("data/meta-pool-v0/meta-pool.json"));
    let profiles: Value = serde_json::from_str(
        &std::fs::read_to_string(root.join("data/search-profiles.json")).unwrap(),
    )
    .unwrap();
    let profile = arg("--profile", "blind");
    assert!(matches!(profile.as_str(), "blind" | "open"));
    let iterations: u32 = arg("--iters", &profiles[&profile]["iterations"].to_string())
        .parse()
        .unwrap();
    let first: u64 = arg("--seed", "81001").parse().unwrap();
    let seeds: u64 = arg("--seeds", "1").parse().unwrap();
    let cfg = RmConfig {
        iterations,
        rule: SelRule::Ucb,
        c: arg("--c", &profiles[&profile]["c"].to_string())
            .parse()
            .unwrap(),
        ..RmConfig::default()
    };
    assert!(iterations > 0 && seeds > 0 && cfg.c.is_finite() && cfg.c >= 0.0);
    let out = PathBuf::from(arg("--out", "tmp/perish-audit"));
    std::fs::create_dir_all(&out).unwrap();
    let (spec, provenance) = if args.iter().any(|a| a == "--log") {
        let log = load_battle(&PathBuf::from(arg("--log", "")));
        let side: usize = arg("--side", "1").parse().unwrap();
        let turn: u16 = arg("--turn", "26").parse().unwrap();
        let d = log
            .decisions
            .iter()
            .find(|d| d.side == side && d.turn == turn)
            .expect("missing decision");
        let src = load_sources(&dex, &root);
        let rec = reconstruct_context_with_cfg(
            &dex,
            &src,
            pool.clone(),
            &log.lines,
            &log.evidence,
            d,
            1,
            cfg.clone(),
        )
        .unwrap();
        assert!(
            !rec.imputed_pick,
            "freeze only positions with known own selection"
        );
        let provenance = json!({"own_sets":rec.provenance,"own_selection_imputed":rec.imputed_pick,
            "active_revealed_moves":rec.revealed_moves,"belief":rec.agent.belief_info(),
            "opponent_future_moves_injected":false});
        (rec.agent.to_position_spec(&dex).unwrap(), provenance)
    } else {
        (
            PositionSpec::parse(&std::fs::read_to_string(arg("--position", "")).unwrap()).unwrap(),
            json!({"source":"position"}),
        )
    };
    let team: Option<Vec<PokemonSet>> = args
        .iter()
        .position(|a| a == "--opponent-team")
        .map(|i| serde_json::from_str(&std::fs::read_to_string(&args[i + 1]).unwrap()).unwrap());
    assert_eq!(
        profile == "open",
        team.is_some(),
        "open requires a supplied sheet; blind must not pin one"
    );
    std::fs::write(
        out.join("position.json"),
        serde_json::to_string_pretty(&spec).unwrap(),
    )
    .unwrap();
    std::fs::write(
        out.join("provenance.json"),
        serde_json::to_string_pretty(&provenance).unwrap(),
    )
    .unwrap();
    let make_agent = |seed| {
        let mut a = ProtocolAgent::new(&dex, spec.side, pool.clone(), cfg.clone(), seed);
        if let Some(team) = &team {
            a.pin_opponent(team.clone());
        }
        a.set_position(&dex, &spec).unwrap();
        a
    };
    for seed in first..first + seeds {
        let mut a = make_agent(seed);
        let mut grouped: BTreeMap<String, Aggregate> = BTreeMap::new();
        let mut roots: BTreeMap<String, Aggregate> = BTreeMap::new();
        let mut root_states: BTreeMap<String, u64> = BTreeMap::new();
        let mut leaf_count = 0u64;
        let start = std::time::Instant::now();
        for iteration in 1..=iterations {
            let mut path = Vec::new();
            let mut root_action = String::new();
            let mut replay_end = None;
            let mut tree_steps = 0;
            a.step_observed(&dex, 1, &mut |event| match event {
                SearchTrace::Choice {
                    battle,
                    actions,
                    chosen,
                    visits,
                    rewards,
                    ..
                } => {
                    tree_steps += 1;
                    if root_action.is_empty() {
                        root_action = chosen[spec.side].unwrap().to_input(&dex);
                        *root_states
                            .entry(snapshot(battle, &dex).to_string())
                            .or_default() += 1;
                    }
                    let exploration = std::array::from_fn(|s| {
                        let selected = actions[s].iter().position(|c| Some(*c) == chosen[s])?;
                        if actions[s].len() < 2 {
                            return Some(false);
                        }
                        let counts: Vec<_> = visits[s]
                            .iter()
                            .enumerate()
                            .map(|(i, n)| n - u32::from(i == selected))
                            .collect();
                        if counts[selected] == 0 {
                            return None;
                        }
                        let mean = rewards[s][selected] / counts[selected] as f64;
                        Some(
                            counts
                                .iter()
                                .enumerate()
                                .any(|(i, n)| *n > 0 && rewards[s][i] / (*n as f64) > mean + 1e-12),
                        )
                    });
                    for e in events(
                        battle,
                        &dex,
                        actions,
                        chosen,
                        "tree",
                        spec.side,
                        exploration,
                    ) {
                        path.push((tree_steps, e));
                    }
                }
                SearchTrace::Leaf {
                    battle,
                    rng,
                    rollout,
                } if rollout => {
                    leaf_count += 1;
                    let mut b = battle.clone();
                    let mut rng = rng.clone();
                    let cap = spec.turn.saturating_add(cfg.horizon);
                    let cutoff = match &cfg.playout {
                        Playout::Uniform => cap,
                        Playout::Heavy { turns, .. } => cap.min(b.turn.saturating_add(*turns)),
                    };
                    let mut step = tree_steps;
                    while b.outcome().is_none() && b.turn <= cutoff {
                        step += 1;
                        let actions = [b.legal_choices(&dex, 0), b.legal_choices(&dex, 1)];
                        let picks = std::array::from_fn(|s| {
                            if actions[s].is_empty() {
                                None
                            } else {
                                Some(playout_pick(
                                    &b,
                                    &dex,
                                    &cfg.playout,
                                    s,
                                    &actions[s],
                                    &mut rng,
                                    cfg.rollout_m16c,
                                ))
                            }
                        });
                        for e in events(&b, &dex, &actions, picks, "rollout", spec.side, [None; 2])
                        {
                            path.push((step, e));
                        }
                        b.apply_choices(&dex, picks).unwrap();
                    }
                    replay_end = Some((b.state_key128(), b.prng.seed_str()));
                }
                SearchTrace::Result { battle, reward0 } => {
                    if let Some(key) = replay_end.take() {
                        assert_eq!(
                            key,
                            (battle.state_key128(), battle.prng.seed_str()),
                            "rollout replay diverged"
                        );
                    }
                    let reward = if spec.side == 0 {
                        reward0
                    } else {
                        1.0 - reward0
                    };
                    let r = roots.entry(root_action.clone()).or_default();
                    r.n += 1;
                    r.reward += reward;
                    for (step, mut e) in path.drain(..) {
                        e["root_action"] = json!(root_action);
                        e["quarter"] = json!(((iteration - 1) as u64 * 4) / iterations as u64);
                        let row = grouped.entry(e.to_string()).or_default();
                        if row.n == 0 {
                            row.first_iteration = iteration;
                            row.first_step = step;
                        }
                        row.n += 1;
                        row.reward += reward;
                    }
                }
                _ => {}
            })
            .unwrap();
        }
        let search = a.search().unwrap();
        for (i, action) in search.actions().iter().enumerate() {
            let r = &roots[&action.to_input(&dex)];
            assert_eq!(r.n, search.visits()[i] as u64);
            assert!((r.reward / r.n as f64 - search.means()[i]).abs() < 1e-12);
        }
        let actions: Vec<_> = search.actions().iter().enumerate().map(|(i,c)|json!({
            "action":c.to_input(&dex),"visits":search.visits()[i],"mean":search.means()[i],"dominated":search.dominated()[i]
        })).collect();
        let mut result = json!({"seed":seed,"profile":profile,"iterations":iterations,"c":cfg.c,
            "best":search.best().unwrap().to_input(&dex),"actions":actions,"leaf_replays":leaf_count,
            "observed_seconds":start.elapsed().as_secs_f64(),"root_states":root_states.into_iter().map(|(s,n)|json!({"state":serde_json::from_str::<Value>(&s).unwrap(),"n":n})).collect::<Vec<_>>(),
            "events":grouped.into_iter().map(|(k,v)| {
                let mut e: Value = serde_json::from_str(&k).unwrap();
                e["n"]=json!(v.n);e["reward_sum"]=json!(v.reward);
                e["first_iteration"]=json!(v.first_iteration);e["first_step"]=json!(v.first_step);e
            }).collect::<Vec<_>>()});
        let mut control = make_agent(seed);
        control.step(&dex, iterations).unwrap();
        for extra in [0, 100] {
            a.step(&dex, extra).unwrap();
            control.step(&dex, extra).unwrap();
            let x = a.search().unwrap();
            let y = control.search().unwrap();
            assert_eq!(x.visits(), y.visits());
            assert_eq!(x.means(), y.means());
            assert_eq!(x.root_matrix(), y.root_matrix());
            assert_eq!(x.node_count(), y.node_count());
        }
        result["ordinary_control_equal"] = json!(true);
        std::fs::write(
            out.join(format!("seed-{seed}.json")),
            serde_json::to_string(&result).unwrap(),
        )
        .unwrap();
        println!(
            "{}",
            json!({"seed":seed,"best":result["best"],"actions":result["actions"],"leaf_replays":leaf_count,"verified":true})
        );
    }
}
