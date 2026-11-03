//! Read-only diagnostics. Truth snapshots are analyst output, never agent input.
use super::*;
use std::io::Write;

pub struct Trace(std::io::BufWriter<std::fs::File>);

impl Trace {
    pub fn new(path: &std::path::Path) -> Self {
        Self(std::io::BufWriter::new(
            std::fs::File::create(path).unwrap(),
        ))
    }

    pub fn write(&mut self, value: Value) {
        serde_json::to_writer(&mut self.0, &value).unwrap();
        self.0.write_all(b"\n").unwrap();
        self.0.flush().unwrap();
    }
}

pub fn search(search: &BlindSearch, dex: &Dex) -> Value {
    let means = search.means();
    json!({"iterations": search.iterations(),
        "actions": search.actions().iter().enumerate().map(|(i, a)| json!({
            "input": a.to_input(dex), "visits": search.visits()[i], "mean": means[i]
        })).collect::<Vec<_>>()})
}

pub fn belief(belief: &Belief) -> Value {
    json!({"fallback": belief.is_fallback(),
        "candidates": belief.alive().iter().map(|&i| belief.candidate_id(i)).collect::<Vec<_>>()})
}

pub fn state(battle: &Battle, dex: &Dex) -> Value {
    json!({"turn": battle.turn, "request": format!("{:?}", battle.request_state),
        "sides": (0..2).map(|side| battle.sides[side].party.iter().map(|&slot| {
            let p = &battle.sides[side].roster[slot as usize];
            json!({"slot": slot + 1, "species": dex.species.key(p.species),
                "active": p.is_active, "hp": p.hp, "maxhp": p.maxhp,
                "status": format!("{:?}", p.status), "boosts": format!("{:?}", p.boosts),
                "item": p.item.map(|id| dex.items.key(id)),
                "moves": p.move_slots.iter().map(|m| json!({"move": dex.moves.key(m.id), "pp": m.pp})).collect::<Vec<_>>()})
        }).collect::<Vec<_>>()).collect::<Vec<_>>()})
}
