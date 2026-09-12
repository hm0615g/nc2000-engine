use conformance::{fixture::repo_root, load_dex};
use nc2000_bot::{
    import::ProtocolAgent,
    position::PositionSpec,
    preview::load_meta_pool,
    smmcts::{RmConfig, SearchTrace, SelRule},
};
use nc2000_engine::{battle::{PokemonSet, SearchChoice}, dex::Dex, state::Battle};
use serde_json::{json, Value};
use std::io::Write;

fn state(battle: &Battle, dex: &Dex) -> Value {
    json!({
        "turn":battle.turn,
        "quick_claw":battle.quick_claw_roll,
        "key":format!("{:016x}", battle.state_key_bucketed(16)),
        "sides":battle.sides.iter().map(|side| json!({
            "active":side.active,
            "party":side.party.iter().map(|&slot| {
                let p = &side.roster[slot as usize];
                json!({"slot":slot,"species":dex.species.key(p.species),
                    "hp":p.hp,"maxhp":p.maxhp,"status":p.status.as_str(),
                    "boosts":format!("{:?}",p.boosts),
                    "volatiles":p.volatiles.iter().map(|(id,_)|dex.conds_key(*id)).collect::<Vec<_>>(),
                    "durations":p.volatiles.iter().map(|(id,v)|(dex.conds_key(*id),v.duration)).collect::<Vec<_>>(),
                })
            }).collect::<Vec<_>>(),
        })).collect::<Vec<_>>(),
    })
}

fn probe(battle: &Battle, dex: &Dex, chosen: [Option<SearchChoice>; 2], side: usize, alternative: &str) -> Value {
    let mut copy = battle.clone();
    let replacement = copy.legal_choices(dex, 1-side).into_iter()
        .find(|c| c.to_input(dex) == alternative).expect("probe action must be legal");
    let mut changed = chosen;
    changed[1-side] = Some(replacement);
    let ids = [battle.active_id(0).unwrap(), battle.active_id(1).unwrap()];
    let branches = [chosen, changed].map(|joint| {
        let enumerated = nc2000_engine::battle::enumerate::enumerate_step(dex, battle, joint, 4096)
            .expect("one-step probe exceeded enumeration cap");
        let ko = ids.map(|id| enumerated.leaves.iter().filter(|l| l.battle.poke(id).hp <= 0)
            .map(|l| l.prob).sum::<f64>());
        let mut copy = battle.clone();
        copy.set_log_enabled(true);
        copy.apply_choices(dex, joint).unwrap();
        json!({"chosen":joint.map(|c|c.map(|c|c.to_input(dex))),
            "ko_probability":ko,"leaves":enumerated.leaves.len(),
            "probability_mass":enumerated.leaves.iter().map(|l|l.prob).sum::<f64>(),
            "sample_after":state(&copy,dex),"sample_log":copy.log})
    });
    json!({"before":state(battle,dex),"branches":branches,
        "dominated":nc2000_bot::smmcts::dominated_actions(battle,dex,1-side)
            .iter().map(|(c,why)|json!([c.to_input(dex),why])).collect::<Vec<_>>()})
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let arg = |key: &str, default: &str| {
        args.iter().position(|a| a == key)
            .map(|i| args[i + 1].clone()).unwrap_or_else(|| default.into())
    };
    let dex = load_dex();
    let spec = PositionSpec::parse(&std::fs::read_to_string(arg("--position", "")).unwrap()).unwrap();
    let team: Vec<PokemonSet> = serde_json::from_str(
        &std::fs::read_to_string(arg("--opponent-team", "")).unwrap(),
    ).unwrap();
    let first: u64 = arg("--seed", "61001").parse().unwrap();
    let seeds: u64 = arg("--seeds", "1").parse().unwrap();
    let iterations: u32 = arg("--iters", "30000").parse().unwrap();
    let interval: u32 = arg("--interval", "1000").parse().unwrap();
    let c: f64 = arg("--c", "1").parse().unwrap();
    let trace_path = arg("--trace-out", "");
    let probe_iteration: u32 = arg("--probe-iteration", "0").parse().unwrap();
    let probe_step: usize = arg("--probe-step", "0").parse().unwrap();
    let probe_action = arg("--probe-action", "");
    let probe_out = arg("--probe-out", "");
    let mut trace_out = if trace_path.is_empty() { None } else {
        Some(std::io::BufWriter::new(std::fs::File::create(trace_path).unwrap()))
    };
    assert!(probe_iteration == 0 || (trace_out.is_some() && probe_step > 0
        && !probe_action.is_empty() && !probe_out.is_empty() && seeds == 1
        && probe_iteration <= iterations));
    assert!(interval > 0 && iterations > 0 && seeds > 0 && c.is_finite() && c >= 0.0);
    let cfg = RmConfig { iterations, c, rule: SelRule::Ucb, ..RmConfig::default() };
    let pool = load_meta_pool(&repo_root().join("data/meta-pool-v0/meta-pool.json"));
    let mut out = std::io::BufWriter::new(std::io::stdout().lock());
    for seed in first..first + seeds {
        let mut agent = ProtocolAgent::new(&dex, spec.side, pool.clone(), cfg.clone(), seed);
        agent.pin_opponent(team.clone());
        agent.set_position(&dex, &spec).unwrap();
        let mut done = 0;
        while done < iterations {
            let count = interval.min(iterations - done);
            if let Some(trace_out) = trace_out.as_mut() {
                for iteration in done..done + count {
                    let mut tree = Vec::new();
                    let mut leaf = Value::Null;
                    let mut probe_result = None;
                    let root_before = if iteration + 1 == probe_iteration {
                        let search = agent.search().unwrap();
                        json!({"visits":search.visits(),"means":search.means()})
                    } else { Value::Null };
                    agent.step_observed(&dex, 1, &mut |event| match event {
                        SearchTrace::Choice { battle, node, actions, visits, rewards, chosen } => {
                            tree.push(json!({
                                "state":state(battle,&dex),"node":node,
                                "actions":actions.iter().map(|cs|cs.iter().map(|c|c.to_input(&dex)).collect::<Vec<_>>()).collect::<Vec<_>>(),
                                "visits":visits,"rewards":rewards,
                                "chosen":chosen.map(|c|c.map(|c|c.to_input(&dex))),
                            }));
                            if iteration + 1 == probe_iteration && tree.len() == probe_step {
                                let mut result = probe(battle,&dex,chosen,spec.side,&probe_action);
                                let s = 1-spec.side;
                                let counts = actions[s].iter().enumerate().map(|(i,a)|
                                    visits[s][i]-u32::from(actions[s].len()>1 && Some(*a)==chosen[s])
                                ).collect::<Vec<_>>();
                                let total: u32 = counts.iter().sum();
                                result["selection"] = json!({"node":node,
                                    "actions":actions[s].iter().enumerate().map(|(i,a)| {
                                        let mean = if counts[i]>0 {Some(rewards[s][i]/counts[i] as f64)} else {None};
                                        let bonus = if counts[i]>0 {Some(cfg.c*((total as f64).ln()/counts[i] as f64).sqrt())} else {None};
                                        json!({"action":a.to_input(&dex),"visits_before":counts[i],
                                            "mean":mean,"exploration_bonus":bonus,"ucb":mean.zip(bonus).map(|(m,b)|m+b)})
                                    }).collect::<Vec<_>>()});
                                probe_result = Some(result);
                            }
                        },
                        SearchTrace::Leaf { battle, rollout, .. } => {
                            leaf = json!({"state":state(battle,&dex),"rollout":rollout});
                        },
                        SearchTrace::Result { battle, reward0 } => {
                            writeln!(trace_out,"{}",json!({
                                "seed":seed,"c":c,"iteration":iteration+1,"tree":tree,"leaf":leaf,
                                "end":state(battle,&dex),"terminal":battle.outcome().is_some(),
                                "reward":if spec.side==0 {reward0} else {1.0-reward0},
                            })).unwrap();
                        },
                    }).unwrap();
                    if let Some(mut result) = probe_result {
                        let search = agent.search().unwrap();
                        result["seed"] = json!(seed);
                        result["iteration"] = json!(iteration+1);
                        result["tree_step"] = json!(probe_step);
                        result["root_before"] = root_before;
                        result["root_after"] = json!({"visits":search.visits(),"means":search.means(),
                            "actions":search.actions().iter().map(|a|a.to_input(&dex)).collect::<Vec<_>>()});
                        std::fs::write(&probe_out,serde_json::to_string_pretty(&result).unwrap()).unwrap();
                    }
                }
            } else {
                agent.step(&dex, count).unwrap();
            }
            let search = agent.search().unwrap();
            done = search.iterations();
            let means = search.means();
            let actions = search.actions().iter().enumerate().map(|(i, action)| json!({
                "action":action.to_input(&dex), "visits":search.visits()[i],
                "mean":means[i], "dominated":search.dominated()[i],
                "ucb":means[i]+cfg.c*((done as f64).ln()/search.visits()[i] as f64).sqrt(),
            })).collect::<Vec<_>>();
            let matrix = search.root_matrix().into_iter().map(|(i, reply, n, mean)| json!({
                "action":search.actions()[i].to_input(&dex),
                "reply":reply.to_input(&dex), "visits":n, "mean":mean,
            })).collect::<Vec<_>>();
            writeln!(out, "{}", json!({
                "seed":seed,"c":c,"iterations":done,"nodes":search.node_count(),
                "best":search.best().unwrap().to_input(&dex),
                "actions":actions,"matrix":matrix,
            })).unwrap();
        }
        if args.iter().any(|a| a == "--verify") {
            let mut control = ProtocolAgent::new(&dex, spec.side, pool.clone(), cfg.clone(), seed);
            control.pin_opponent(team.clone());
            control.set_position(&dex, &spec).unwrap();
            control.step(&dex, iterations).unwrap();
            for extra in [0, 100] {
                agent.step(&dex, extra).unwrap();
                control.step(&dex, extra).unwrap();
                let a = agent.search().unwrap();
                let b = control.search().unwrap();
                assert_eq!(a.visits(),b.visits());
                assert_eq!(a.means(),b.means());
                assert_eq!(a.root_matrix(),b.root_matrix());
                assert_eq!(a.node_count(),b.node_count());
            }
            eprintln!("seed {seed}: observed and ordinary search statistics match exactly, including 100 subsequent iterations");
        }
        out.flush().unwrap();
    }
    if let Some(trace_out) = trace_out.as_mut() {
        trace_out.flush().unwrap();
    }
}
