use conformance::{fixture::repo_root, load_dex};
use nc2000_bot::{
    blind::BlindSearch,
    mcts::{playout_pick, Playout},
    preview::load_meta_pool,
    smmcts::{RmConfig, SearchTrace, SelRule},
    Agent, Belief, Observer, OpenAgent, SplitMix64,
};
use nc2000_engine::{battle::SearchChoice, dex::Dex, state::Battle};
use serde_json::{json, Value};
use std::{collections::BTreeMap, io::Write};

fn escape_leaf(
    b: &mut Battle,
    dex: &Dex,
    rng: &mut SplitMix64,
    rollout: bool,
    cfg: &RmConfig,
    cap: u16,
    overrides: &mut u64,
) -> f64 {
    let value = |b: &Battle| match &cfg.playout {
        Playout::Uniform => unreachable!("escape candidate requires heavy playout"),
        Playout::Heavy { weights, .. } => nc2000_bot::eval::eval_leaf(b, dex, weights),
    };
    if !rollout {
        return value(b);
    }
    let cutoff = match &cfg.playout {
        Playout::Uniform => cap,
        Playout::Heavy { turns, .. } => cap.min(b.turn.saturating_add(*turns)),
    };
    loop {
        if let Some(o) = b.outcome() {
            return match o {
                nc2000_engine::battle::Outcome::P1Win => 1.0,
                nc2000_engine::battle::Outcome::P2Win => 0.0,
                nc2000_engine::battle::Outcome::Tie => 0.5,
            };
        }
        if b.turn > cutoff {
            return value(b);
        }
        let chosen = std::array::from_fn(|s| {
            let actions = b.legal_choices(dex, s);
            if actions.is_empty() {
                return None;
            }
            let normal = playout_pick(b, dex, &cfg.playout, s, &actions, rng, cfg.rollout_rules());
            let expires = b.active_id(s).is_some_and(|id| {
                dex.conds_id("perishsong")
                    .and_then(|c| b.poke(id).volatile(c))
                    .and_then(|v| v.duration)
                    == Some(1)
            });
            if !expires {
                return Some(normal);
            }
            let switches = actions
                .iter()
                .copied()
                .filter(|a| matches!(a, SearchChoice::Switch(_)));
            let score = |a: &SearchChoice| {
                let SearchChoice::Switch(pos) = a else {
                    unreachable!()
                };
                let id = nc2000_engine::state::PokeId {
                    side: s as u8,
                    slot: b.sides[s].party[*pos as usize - 1],
                };
                let p = b.poke(id);
                let incoming = b.active_id(1 - s).map_or(0.0, |foe| {
                    nc2000_bot::eval::best_hit_fraction(b, dex, foe, id, true)
                });
                p.hp as f64 / p.maxhp as f64 - incoming
            };
            let chosen = switches
                .max_by(|a, b| score(a).total_cmp(&score(b)))
                .unwrap_or(normal);
            *overrides += u64::from(chosen != normal);
            Some(chosen)
        });
        b.apply_choices(dex, chosen).unwrap();
    }
}

fn traced_search(
    search: &mut BlindSearch,
    dex: &Dex,
    belief: &Belief,
    obs: &Observer,
    cfg: &RmConfig,
    root: &Battle,
    side: usize,
) -> Value {
    let mut root_action = String::new();
    let mut path = Vec::new();
    let mut groups: BTreeMap<String, (u64, f64)> = BTreeMap::new();
    let mut replay_end = None;
    let record = |path: &mut Vec<String>,
                  b: &Battle,
                  actions: &[Vec<SearchChoice>; 2],
                  chosen: [Option<SearchChoice>; 2],
                  phase: &str,
                  root_action: &str| {
        for s in 0..2 {
            let Some(p) = b.active_id(s).map(|id| b.poke(id)) else {
                continue;
            };
            let Some(counter) = dex
                .conds_id("perishsong")
                .and_then(|c| p.volatile(c))
                .and_then(|v| v.duration)
            else {
                continue;
            };
            if let Some(action) = chosen[s] {
                path.push(json!({"root":root_action,"phase":phase,"side":s,"counter":counter,"trapped":p.trapped,
                    "switch_available":actions[s].iter().any(|a| matches!(a, SearchChoice::Switch(_))),
                    "choice":action.to_input(dex)}).to_string());
            }
        }
    };
    search.step_observed(dex, belief, obs, cfg.iterations, &mut |event| match event {
        SearchTrace::Choice {
            battle,
            actions,
            chosen,
            ..
        } => {
            if root_action.is_empty() {
                root_action = chosen[side].unwrap().to_input(dex);
            }
            record(&mut path, battle, actions, chosen, "tree", &root_action);
        }
        SearchTrace::Leaf {
            battle,
            rng,
            rollout: true,
        } => {
            let mut b = battle.clone();
            let mut rng = rng.clone();
            let cap = root.turn.saturating_add(cfg.horizon);
            let cutoff = match &cfg.playout {
                Playout::Uniform => cap,
                Playout::Heavy { turns, .. } => cap.min(b.turn.saturating_add(*turns)),
            };
            while b.outcome().is_none() && b.turn <= cutoff {
                let actions = [b.legal_choices(dex, 0), b.legal_choices(dex, 1)];
                let chosen = std::array::from_fn(|s| {
                    (!actions[s].is_empty()).then(|| {
                        playout_pick(
                            &b,
                            dex,
                            &cfg.playout,
                            s,
                            &actions[s],
                            &mut rng,
                            cfg.rollout_rules(),
                        )
                    })
                });
                record(&mut path, &b, &actions, chosen, "rollout", &root_action);
                b.apply_choices(dex, chosen).unwrap();
            }
            replay_end = Some((b.state_key128(), b.prng.seed_str()));
        }
        SearchTrace::Result { battle, reward0 } => {
            if let Some(key) = replay_end.take() {
                assert_eq!(key, (battle.state_key128(), battle.prng.seed_str()));
            }
            let reward = if side == 0 { reward0 } else { 1.0 - reward0 };
            for event in path.drain(..) {
                let row = groups.entry(event).or_default();
                row.0 += 1;
                row.1 += reward;
            }
            root_action.clear();
        }
        _ => {}
    });
    json!(groups.into_iter().map(|(event,(n,reward))|json!({"event":serde_json::from_str::<Value>(&event).unwrap(),"n":n,"mean":reward/n as f64})).collect::<Vec<_>>())
}

fn choice(b: &mut Battle, dex: &Dex, side: usize, input: &str) -> SearchChoice {
    b.legal_choices(dex, side)
        .into_iter()
        .find(|a| {
            if a.to_input(dex) == input {
                return true;
            }
            if let SearchChoice::Switch(pos) = a {
                let slot = b.sides[side].party[*pos as usize - 1];
                return input
                    == format!(
                        "switch {}",
                        dex.species.key(b.sides[side].roster[slot as usize].species)
                    );
            }
            false
        })
        .unwrap_or_else(|| panic!("illegal scripted action {side}: {input}"))
}

fn describe(b: &Battle, dex: &Dex) -> Value {
    json!({"turn":b.turn,"sides":(0..2).map(|s| {
        let Some(id) = b.active_id(s) else { return Value::Null; };
        let p=b.poke(id);
        json!({"active":dex.species.key(p.species),"hp":p.hp,"boosts":p.boosts,
            "trapped":p.trapped,"perish":dex.conds_id("perishsong").and_then(|c|p.volatile(c)).and_then(|v|v.duration)})
    }).collect::<Vec<_>>()})
}

fn continuation(
    dex: &Dex,
    preview: &Battle,
    root: &Battle,
    side: usize,
    forced: SearchChoice,
    seed: u64,
    iterations: u32,
) -> Value {
    let cfg = RmConfig {
        iterations,
        rule: SelRule::Ucb,
        c: 1.0,
        ..RmConfig::default()
    };
    let mut rng = SplitMix64::new(seed);
    let mut b = root.clone();
    b.reseed(rng.next());
    let mut agents =
        std::array::from_fn::<_, 2, _>(|_| OpenAgent::new(cfg.clone(), None, rng.next()));
    for s in 0..2 {
        let mut p = preview.clone();
        let choices = p.legal_choices(dex, s);
        agents[s].choose(preview, dex, s, &choices[..1]);
    }
    let mut first = true;
    let mut first_step = Value::Null;
    let mut decisions = 0;
    while b.outcome().is_none() {
        assert!(b.turn <= 1100, "continuation exceeded engine turn limit");
        let mut joint = [None, None];
        for s in 0..2 {
            let legal = b.legal_choices(dex, s);
            if legal.is_empty() {
                continue;
            }
            joint[s] = Some(if first && s == side {
                assert!(legal.contains(&forced));
                forced
            } else {
                agents[s].choose(&b, dex, s, &legal)
            });
        }
        let log_start = b.log.len();
        b.apply_choices(dex, joint).unwrap();
        if first {
            first_step = json!({"choices":joint.map(|a| a.map(|a| a.to_input(dex))),
                "log":&b.log[log_start..],"state":describe(&b,dex)});
        }
        first = false;
        decisions += 1;
    }
    let score = match b.outcome().unwrap() {
        nc2000_engine::battle::Outcome::P1Win => {
            if side == 0 {
                1.0
            } else {
                0.0
            }
        }
        nc2000_engine::battle::Outcome::P2Win => {
            if side == 1 {
                1.0
            } else {
                0.0
            }
        }
        nc2000_engine::battle::Outcome::Tie => 0.5,
    };
    json!({"seed":seed,"forced":forced.to_input(dex),"score":score,"turns":b.turn-root.turn,"decisions":decisions,"first_step":first_step})
}

fn main() {
    let args: Vec<_> = std::env::args().collect();
    let arg = |key: &str, default: &str| {
        args.iter()
            .position(|a| a == key)
            .map(|i| args[i + 1].clone())
            .unwrap_or_else(|| default.into())
    };
    let dex = load_dex();
    let root = repo_root();
    let profile = arg("--profile", "open");
    assert!(matches!(profile.as_str(), "open" | "blind"));
    let profiles: Value = serde_json::from_str(
        &std::fs::read_to_string(root.join("data/search-profiles.json")).unwrap(),
    )
    .unwrap();
    let profile_cfg = &profiles[&profile];
    let trace = args.iter().any(|s| s == "--trace");
    let escape = args.iter().any(|s| s == "--perish-escape");
    assert!(!(trace && escape));
    let pool = load_meta_pool(&root.join("data/meta-pool-v0/meta-pool.json"));
    let fixture: Value =
        serde_json::from_str(&std::fs::read_to_string(arg("--fixture", "")).unwrap()).unwrap();
    let teams: [_; 2] = std::array::from_fn(|s| {
        pool.teams
            .iter()
            .find(|t| t.id == fixture["teams"][s].as_str().unwrap())
            .unwrap()
    });
    let mut b = Battle::from_fixture(&dex, "1,2,3,4", &teams[0].sets, &teams[1].sets).unwrap();
    let preview = b.clone();
    let mut obs = std::array::from_fn::<_, 2, _>(|s| Observer::new(&b, s));
    let mut beliefs = std::array::from_fn::<_, 2, _>(|s| {
        if profile == "open" {
            Belief::pinned_from_battle(&b, &obs[s])
        } else {
            Belief::new(&dex, &pool, &obs[s])
        }
    });
    let picks: [Vec<usize>; 2] = std::array::from_fn(|s| {
        fixture["picks"][s]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| p.as_u64().unwrap() as usize)
            .collect()
    });
    for s in 0..2 {
        b.choose(
            &dex,
            s,
            &format!(
                "team {}",
                picks[s]
                    .iter()
                    .map(|p| p.to_string())
                    .collect::<Vec<_>>()
                    .join(",")
            ),
        )
        .unwrap();
    }
    if fixture["reveal_picks"].as_bool().unwrap_or(false) {
        for i in [1, 2, 0] {
            let joint = std::array::from_fn(|s| {
                Some(choice(
                    &mut b,
                    &dex,
                    s,
                    &format!(
                        "switch {}",
                        nc2000_engine::dex::toid(&teams[s].sets[picks[s][i] - 1].species)
                    ),
                ))
            });
            b.apply_choices(&dex, joint).unwrap();
        }
    }
    let iterations: u32 = arg("--iters", &profile_cfg["iterations"].to_string())
        .parse()
        .unwrap();
    let first: u64 = arg("--seed", "91001").parse().unwrap();
    let seeds: u64 = arg("--seeds", "1").parse().unwrap();
    let continuations: u64 = arg("--continuations", "0").parse().unwrap();
    assert!(
        continuations == 0 || profile == "open",
        "continuations use open agents"
    );
    assert!(
        continuations == 0 || !escape,
        "continuations use shipped agents"
    );
    let follow_iterations: u32 = arg("--follow-iters", "3000").parse().unwrap();
    let follow_actions: Vec<_> = arg("--follow-actions", "")
        .split(',')
        .map(str::to_owned)
        .filter(|s| !s.is_empty())
        .collect();
    let cfg = RmConfig {
        iterations,
        rule: SelRule::Ucb,
        c: profile_cfg["c"].as_f64().unwrap(),
        ..RmConfig::default()
    };
    let stages: Vec<usize> = arg("--stages", "0,1,2,3,4")
        .split(',')
        .map(|s| s.parse().unwrap())
        .collect();
    let sides: Vec<usize> = arg("--sides", "0,1")
        .split(',')
        .map(|s| s.parse().unwrap())
        .collect();
    let prefix = fixture["prefix"].as_array().unwrap();
    let mut out = std::io::BufWriter::new(std::io::stdout().lock());
    for stage in 0..=prefix.len() {
        for s in 0..2 {
            obs[s].observe(&b, &dex);
            beliefs[s].sync(&dex, &obs[s]);
        }
        if stages.contains(&stage) {
            if let Some(expected) = fixture
                .get("expected")
                .and_then(|e| e.get(stage.to_string()))
            {
                assert_eq!(
                    &describe(&b, &dex),
                    expected,
                    "fixture stage {stage} changed"
                );
            }
            for &side in &sides {
                if continuations > 0 {
                    assert!(!follow_actions.is_empty());
                    for input in &follow_actions {
                        let forced = choice(&mut b, &dex, side, input);
                        for seed in first..first + continuations {
                            let result = continuation(
                                &dex,
                                &preview,
                                &b,
                                side,
                                forced,
                                seed,
                                follow_iterations,
                            );
                            writeln!(out,"{}",json!({"stage":stage,"side":side,"follow_iterations":follow_iterations,"continuation":result})).unwrap();
                            out.flush().unwrap();
                        }
                    }
                    continue;
                }
                for seed in first..first + seeds {
                    let mut search = BlindSearch::new(&b, &dex, cfg.clone(), side, seed);
                    let start = std::time::Instant::now();
                    let mut overrides = 0;
                    let events = if escape {
                        search.step_with_leaf(
                            &dex,
                            &beliefs[side],
                            &obs[side],
                            iterations,
                            &mut |sim, rng, rollout| {
                                escape_leaf(
                                    sim,
                                    &dex,
                                    rng,
                                    rollout,
                                    &cfg,
                                    b.turn.saturating_add(cfg.horizon),
                                    &mut overrides,
                                )
                            },
                        );
                        Value::Null
                    } else if trace {
                        let events = traced_search(
                            &mut search,
                            &dex,
                            &beliefs[side],
                            &obs[side],
                            &cfg,
                            &b,
                            side,
                        );
                        let mut control = BlindSearch::new(&b, &dex, cfg.clone(), side, seed);
                        control.step(&dex, &beliefs[side], &obs[side], iterations);
                        assert_eq!(control.visits(), search.visits());
                        assert_eq!(control.means(), search.means());
                        assert_eq!(control.best(), search.best());
                        events
                    } else {
                        search.step(&dex, &beliefs[side], &obs[side], iterations);
                        Value::Null
                    };
                    let actions:Vec<_>=search.actions().iter().enumerate().map(|(i,a)|json!({"action":a.to_input(&dex),"visits":search.visits()[i],"mean":search.means()[i],"dominated":search.dominated()[i]})).collect();
                    writeln!(out,"{}",json!({"stage":stage,"side":side,"seed":seed,"iterations":iterations,"profile":profile,"perish_escape":escape,
                        "state":describe(&b,&dex),"best":search.best().unwrap().to_input(&dex),"actions":actions,
                        "events":events,"escape_overrides":overrides,"seconds":start.elapsed().as_secs_f64()})).unwrap();
                    out.flush().unwrap();
                }
            }
        }
        if let Some(joint) = prefix.get(stage) {
            let joint =
                std::array::from_fn(|s| Some(choice(&mut b, &dex, s, joint[s].as_str().unwrap())));
            b.apply_choices(&dex, joint).unwrap();
        }
    }
}
