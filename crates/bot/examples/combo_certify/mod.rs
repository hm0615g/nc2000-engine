use nc2000_bot::bounds::{BoundConfig, BoundSolver, Bounds};
use nc2000_engine::{
    battle::{enumerate::enumerate_step, Outcome, SearchChoice},
    dex::Dex,
    state::Battle,
};
use serde_json::{json, Value};
use std::collections::BTreeMap;

const UNKNOWN: Bounds = Bounds { lo: 0.0, hi: 1.0 };

struct Cell {
    joint: [Option<SearchChoice>; 2],
    successors: Vec<(f64, usize)>,
    complete: bool,
    runs: usize,
}

fn terminal(b: &Battle) -> Option<Bounds> {
    b.outcome().map(|o| {
        let value = match o {
            Outcome::P1Win => 1.0,
            Outcome::P2Win => 0.0,
            Outcome::Tie => 0.5,
        };
        Bounds {
            lo: value,
            hi: value,
        }
    })
}

fn perspective(bounds: Bounds, side: usize) -> Bounds {
    if side == 0 {
        bounds
    } else {
        Bounds {
            lo: 1.0 - bounds.hi,
            hi: 1.0 - bounds.lo,
        }
    }
}

pub fn certify(dex: &Dex, b: &Battle, side: usize, total_work: usize, cell_cap: usize) -> Value {
    assert!(side < 2);
    if let Some(bounds) = terminal(b) {
        let own = perspective(bounds, side);
        return json!({"side":side,"terminal":true,"lo":own.lo,"hi":own.hi,
            "work_budget":total_work,"runs":0,"actions":[],"cells":[]});
    }
    let mut probe = b.clone();
    let needs = b.needs_choice();
    let acts: [Vec<Option<SearchChoice>>; 2] = std::array::from_fn(|s| {
        if needs[s] {
            let choices = probe.legal_choices(dex, s);
            assert!(!choices.is_empty(), "requested side has no legal action");
            choices.into_iter().map(Some).collect()
        } else {
            vec![None]
        }
    });
    let mut cells = Vec::new();
    let mut states = Vec::<Battle>::new();
    let mut indexes = BTreeMap::new();
    let mut enumeration_runs = 0;
    for &a0 in &acts[0] {
        for &a1 in &acts[1] {
            let joint = [a0, a1];
            let cap = cell_cap.min(total_work - enumeration_runs);
            let mut cell = Cell {
                joint,
                successors: Vec::new(),
                complete: false,
                runs: 0,
            };
            if cap > 0 {
                match enumerate_step(dex, b, joint, cap) {
                    Some(step) => {
                        cell.complete = true;
                        cell.runs = step.runs;
                        let mass: f64 = step.leaves.iter().map(|leaf| leaf.prob).sum();
                        assert!((mass - 1.0).abs() < 1e-8, "incomplete chance mass");
                        let mut merged = BTreeMap::<usize, f64>::new();
                        for leaf in step.leaves {
                            let key = leaf.battle.state_key128();
                            let index = *indexes.entry(key).or_insert_with(|| {
                                let index = states.len();
                                states.push(leaf.battle);
                                index
                            });
                            *merged.entry(index).or_default() += leaf.prob;
                        }
                        cell.successors = merged.into_iter().map(|(i, p)| (p, i)).collect();
                    }
                    None => cell.runs = cap,
                }
                enumeration_runs += cell.runs;
            }
            cells.push(cell);
        }
    }
    let mut solver = BoundSolver::new(dex, BoundConfig::default());
    let unresolved: Vec<usize> = states
        .iter()
        .enumerate()
        .filter_map(|(i, state)| terminal(state).is_none().then_some(i))
        .collect();
    let mut solver_runs = 0;
    let mut stops = Vec::new();
    for (position, &index) in unresolved.iter().enumerate() {
        let available = total_work - enumeration_runs - solver_runs;
        let allowance = available / (unresolved.len() - position);
        if allowance == 0 {
            continue;
        }
        // A BoundSolver trial can spend cell_cap eager runs plus one lazy run
        // before checking work_budget again; reserve that possible overshoot.
        solver.cfg.cell_cap = cell_cap.min(allowance / 4);
        solver.cfg.work_budget = allowance - solver.cfg.cell_cap;
        let report = solver.solve(&states[index], None);
        assert!(report.runs <= allowance);
        solver_runs += report.runs;
        stops.push(json!({"state":index,"stop":format!("{:?}",report.stop),"runs":report.runs}));
    }
    let bounds: Vec<Bounds> = states
        .iter()
        .map(|state| {
            terminal(state)
                .or_else(|| solver.peek(state))
                .unwrap_or(UNKNOWN)
        })
        .collect();
    let cell_bounds: Vec<Bounds> = cells
        .iter()
        .map(|cell| {
            if !cell.complete {
                return UNKNOWN;
            }
            let mut interval = Bounds { lo: 0.0, hi: 0.0 };
            for &(mass, index) in &cell.successors {
                interval.lo += mass * bounds[index].lo;
                interval.hi += mass * bounds[index].hi;
            }
            Bounds {
                lo: interval.lo.clamp(0.0, 1.0),
                hi: interval.hi.clamp(0.0, 1.0),
            }
        })
        .collect();
    let actions: Vec<Value> = acts[side]
        .iter()
        .map(|action| {
            let mut security = Bounds { lo: 1.0, hi: 1.0 };
            for (cell, &bounds) in cells.iter().zip(&cell_bounds) {
                if cell.joint[side] == *action {
                    let own = perspective(bounds, side);
                    security.lo = security.lo.min(own.lo);
                    security.hi = security.hi.min(own.hi);
                }
            }
            json!({"action":action.map(|a|a.to_input(dex)),"lo":security.lo,"hi":security.hi})
        })
        .collect();
    let rows: Vec<Value> = cells
        .iter()
        .zip(cell_bounds)
        .map(|(cell, bounds)| {
            let own = perspective(bounds, side);
            json!({"joint":cell.joint.map(|a|a.map(|a|a.to_input(dex))),
            "complete":cell.complete,"enumeration_runs":cell.runs,
            "successors":cell.successors.len(),"lo":own.lo,"hi":own.hi})
        })
        .collect();
    json!({"side":side,"terminal":false,"work_budget":total_work,
        "runs":enumeration_runs+solver_runs,"enumeration_runs":enumeration_runs,
        "solver_runs":solver_runs,"unique_successors":states.len(),
        "actions":actions,"cells":rows,"solver_stops":stops})
}

#[cfg(test)]
#[path = "../../tests/support/perish_deadline.rs"]
mod perish_deadline;

#[cfg(test)]
mod tests {
    use super::*;
    use nc2000_engine::battle::PokemonSet;
    use perish_deadline::WaitingMove;

    fn start(dex: &Dex) -> Battle {
        let mon: PokemonSet = serde_json::from_value(json!({
            "name":"Pikachu","species":"Pikachu","item":"",
            "ability":"","moves":["Splash","Swift"],"level":50
        }))
        .unwrap();
        let mut b = Battle::from_fixture(dex, "1,2,3,4", &[mon.clone()], &[mon]).unwrap();
        let joint = std::array::from_fn(|side| Some(b.legal_choices(dex, side)[0]));
        b.apply_choices(dex, joint).unwrap();
        b
    }

    #[test]
    fn zero_budget_keeps_every_reply_unknown() {
        let dex = conformance::load_dex();
        let mut b = start(&dex);
        let counts = [
            b.legal_choices(&dex, 0).len(),
            b.legal_choices(&dex, 1).len(),
        ];
        for side in 0..2 {
            let result = certify(&dex, &b, side, 0, 4096);
            assert_eq!(result["runs"], 0);
            assert_eq!(result["actions"].as_array().unwrap().len(), counts[side]);
            let cells = result["cells"].as_array().unwrap();
            assert_eq!(cells.len(), counts[0] * counts[1]);
            for cell in cells {
                assert_eq!(cell["complete"], false);
                assert_eq!(cell["lo"], 0.0);
                assert_eq!(cell["hi"], 1.0);
            }
            for action in result["actions"].as_array().unwrap() {
                assert_eq!(action["lo"], 0.0);
                assert_eq!(action["hi"], 1.0);
            }
        }
    }

    #[test]
    fn terminal_roots_need_no_work_and_use_observer_perspective() {
        let dex = conformance::load_dex();
        for winner in [Some(0), Some(1), None] {
            let mut b = start(&dex);
            b.win(winner);
            for side in 0..2 {
                let result = certify(&dex, &b, side, 0, 0);
                let expected = winner.map_or(0.5, |winner| f64::from(winner == side));
                assert_eq!(result["terminal"], true);
                assert_eq!(result["runs"], 0);
                assert_eq!(result["lo"], expected);
                assert_eq!(result["hi"], expected);
                assert!(result["cells"].as_array().unwrap().is_empty());
            }
        }
    }

    #[test]
    fn exact_terminal_cells_complement_between_sides() {
        let dex = conformance::load_dex();
        let mut b = start(&dex);
        b.turn = 1000;
        for side in 0..2 {
            let active = b.active_id(side).unwrap();
            b.poke_mut(active).hp = 1;
        }
        let results = [
            certify(&dex, &b, 0, 4096, 1024),
            certify(&dex, &b, 1, 4096, 1024),
        ];
        for result in &results {
            assert!(result["runs"].as_u64().unwrap() <= 4096);
            assert_eq!(result["cells"].as_array().unwrap().len(), 4);
        }
        let cells0 = results[0]["cells"].as_array().unwrap();
        let cells1 = results[1]["cells"].as_array().unwrap();
        for (a, b) in cells0.iter().zip(cells1) {
            assert_eq!(a["joint"], b["joint"]);
            assert_eq!(a["complete"], true);
            assert_eq!(b["complete"], true);
            let (lo, hi) = (a["lo"].as_f64().unwrap(), a["hi"].as_f64().unwrap());
            assert!((lo - hi).abs() < 1e-10);
            assert!((lo + b["hi"].as_f64().unwrap() - 1.0).abs() < 1e-10);
            assert!((hi + b["lo"].as_f64().unwrap() - 1.0).abs() < 1e-10);
        }
        for action in ["move splash", "move swift"] {
            let values: Vec<&Value> = results
                .iter()
                .map(|result| {
                    result["actions"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .find(|row| row["action"] == action)
                        .unwrap()
                })
                .collect();
            assert!(
                (values[0]["lo"].as_f64().unwrap() - values[1]["lo"].as_f64().unwrap()).abs()
                    < 1e-10
            );
            assert!(
                (values[0]["hi"].as_f64().unwrap() - values[1]["hi"].as_f64().unwrap()).abs()
                    < 1e-10
            );
        }
    }

    fn certify_deadline(wait: WaitingMove) {
        let dex = conformance::load_dex();
        let (_, b) = perish_deadline::position(&dex, wait);
        let result = certify(&dex, &b, 0, 100_000, 4096);
        assert!(result["runs"].as_u64().unwrap() <= 100_000);
        let actions = result["actions"].as_array().unwrap();
        assert_eq!(actions.len(), 3);
        let waiting = format!("move {}", wait.key());
        let waiting = actions.iter().find(|a| a["action"] == waiting).unwrap();
        assert!(waiting["lo"].as_f64().unwrap() > 0.98, "{result}");
        for attack in ["move shadowball", "move psychic"] {
            let attack = actions.iter().find(|a| a["action"] == attack).unwrap();
            assert_eq!(attack["hi"], 0.0, "{result}");
        }
    }

    #[test]
    fn immune_return_outvalues_attacking_destiny_bond_at_perish_deadline() {
        certify_deadline(WaitingMove::Return);
    }

    #[test]
    fn awake_snore_outvalues_attacking_destiny_bond_at_perish_deadline() {
        certify_deadline(WaitingMove::Snore);
    }

    #[test]
    fn exhausted_enumeration_retains_unknown_replies_and_full_action_set() {
        let dex = conformance::load_dex();
        let (_, mut b) = perish_deadline::position(&dex, WaitingMove::Return);
        let counts = [
            b.legal_choices(&dex, 0).len(),
            b.legal_choices(&dex, 1).len(),
        ];
        let result = certify(&dex, &b, 0, 1, 4096);
        assert_eq!(result["runs"], 1);
        assert_eq!(result["actions"].as_array().unwrap().len(), counts[0]);
        let cells = result["cells"].as_array().unwrap();
        assert_eq!(cells.len(), counts[0] * counts[1]);
        let incomplete: Vec<_> = cells
            .iter()
            .filter(|cell| cell["complete"] == false)
            .collect();
        assert!(!incomplete.is_empty());
        for cell in incomplete {
            assert_eq!(cell["lo"], 0.0);
            assert_eq!(cell["hi"], 1.0);
        }
        for action in result["actions"].as_array().unwrap() {
            assert_eq!(action["lo"], 0.0);
        }
    }

    #[test]
    fn deadline_sets_pass_the_learnset_validator_on_complete_sheets() {
        use nc2000_engine::validate::{validate_team, Learnsets};
        let dex = conformance::load_dex();
        let ls = Learnsets::from_json(
            &std::fs::read_to_string(
                conformance::fixture::repo_root().join("data/learnsets-gen2.json"),
            )
            .unwrap(),
        )
        .unwrap();
        for wait in [WaitingMove::Return, WaitingMove::Snore] {
            let (preview, _) = perish_deadline::position(&dex, wait);
            for side in &preview.sides {
                for p in &side.roster {
                    let keys = ["hp", "atk", "def", "spa", "spd", "spe"];
                    let ivs: BTreeMap<_, _> = keys.into_iter().zip(p.set_ivs).collect();
                    let evs: BTreeMap<_, _> = keys.into_iter().zip(p.set_evs).collect();
                    let mut sheet = vec![json!({
                        "name":dex.species.key(p.species),"species":dex.species.key(p.species),
                        "level":p.level,"item":p.item.map(|id|dex.items.key(id)).unwrap_or(""),
                        "ability":"No Ability","ivs":ivs,"evs":evs,"happiness":p.happiness,
                        "moves":p.base_move_slots.iter().map(|m|dex.moves.key(m.id)).collect::<Vec<_>>()
                    })];
                    for (species, attack) in [
                        ("Bulbasaur", "Tackle"),
                        ("Charmander", "Scratch"),
                        ("Squirtle", "Tackle"),
                        ("Pikachu", "Thunder Shock"),
                        ("Caterpie", "Tackle"),
                    ] {
                        sheet.push(json!({"name":species,"species":species,"level":50,
                            "moves":[attack],"item":"","ability":"No Ability"}));
                    }
                    let validation =
                        validate_team(&dex, &ls, &serde_json::to_string(&sheet).unwrap());
                    assert_eq!(validation["ok"], true, "{validation}");
                }
            }
        }
    }
}
