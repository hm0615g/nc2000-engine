//! Audit the frozen prior against maximum training at unchanged DVs/loadouts.
//! No search, completed-match evaluation, or product data mutation. Most move
//! evidence is synthetic; the Rain Dance witness executes one legal turn.
//!
//! cargo run --quiet -p nc2000-bot --example audit_prior_training -- OUTPUT.json
use std::collections::BTreeMap;

use nc2000_bot::{preview::MetaPool, Belief, Observer, SplitMix64};
use nc2000_engine::{
    battle::PokemonSet,
    dex::Dex,
    state::{Battle, Pokemon},
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

const KEYS: [&str; 6] = ["hp", "atk", "def", "spa", "spd", "spe"];
const PRIOR: &str = "data/belief-pool-v3/belief-pool.json";

fn stats(p: &Pokemon) -> [i32; 6] {
    let [atk, def, spa, spd, spe] = p.base_stored_stats;
    [p.base_maxhp, atk, def, spa, spd, spe]
}

fn maximum_training(sets: &[PokemonSet]) -> Vec<PokemonSet> {
    let mut out = sets.to_vec();
    for s in &mut out {
        s.evs = Some(KEYS.into_iter().map(|k| (k.into(), 255)).collect());
    }
    out
}

fn fixture(dex: &Dex, sets: &[PokemonSet]) -> Battle {
    Battle::from_fixture(dex, "1,2,3,4", sets, sets).unwrap()
}

fn samples(dex: &Dex, b: &Battle, obs: &Observer, belief: &Belief, slot: usize) -> Vec<Value> {
    belief
        .alive()
        .iter()
        .map(|&i| {
            let d = belief.determinize_with(dex, b, obs, Some(i), &mut SplitMix64::new(17));
            let p = &d.sides[1].roster[slot];
            json!({"id": belief.candidate_id(i), "stats": stats(p), "evs": p.set_evs})
        })
        .collect()
}

// The observer sees only normal move-usage evidence. No hidden spread is
// passed to Belief; the EV-255 counterfactual preserves all preview facts.
fn reveal(dex: &Dex, b: &Battle, slot: usize, moves: &[usize]) -> (Battle, Observer) {
    let mut obs = Observer::new(b, 0);
    let mut out = b.clone();
    for &m in moves {
        out.sides[1].roster[slot].base_move_slots[m].used = true;
    }
    obs.observe(&out, dex);
    (out, obs)
}

fn supports(samples: &[Value], stat: usize, target: i32) -> bool {
    samples
        .iter()
        .any(|r| r["stats"][stat].as_i64().unwrap() >= i64::from(target))
}

fn bump(map: &mut BTreeMap<String, usize>, key: &str) {
    *map.entry(key.into()).or_default() += 1;
}

fn played_rain_dance_witness(dex: &Dex, pool: &MetaPool) -> Value {
    let team = pool
        .teams
        .iter()
        .find(|t| t.id == "lineage-fine-s07")
        .unwrap();
    let mut b = fixture(dex, &maximum_training(&team.sets));
    b.set_log_enabled(true);
    let mut obs = Observer::new(&b, 0);
    // Electrode 50 + Cloyster 50 + Zapdos 55 is a legal selected party.
    b.choose(dex, 0, "team 1,3,6").unwrap();
    b.choose(dex, 1, "team 1,3,6").unwrap();
    obs.observe(&b, dex);
    let mut belief = Belief::new(dex, pool, &obs);
    let before: Vec<_> = belief
        .alive()
        .iter()
        .map(|&i| belief.candidate_id(i).to_owned())
        .collect();
    // Both leads execute Rain Dance: no damage, status or HP reconstruction
    // is involved in the candidate elimination.
    b.choose(dex, 0, "move raindance").unwrap();
    b.choose(dex, 1, "move raindance").unwrap();
    obs.observe(&b, dex);
    belief.sync(dex, &obs);
    assert_eq!(belief.candidate_count(), 1);
    assert_eq!(belief.candidate_id(belief.alive()[0]), "lineage-fine-s07");
    let mons: Vec<_> = [2, 5]
        .into_iter()
        .map(|slot| {
            json!({
                "species": dex.species.key(b.sides[1].roster[slot].species),
                "true_stats": stats(&b.sides[1].roster[slot]),
                "posterior": samples(dex, &b, &obs, &belief, slot)
            })
        })
        .collect();
    json!({"selected": [1, 3, 6], "total_level": 155,
        "candidates_before": before, "mons_after_rain_dance": mons, "log": b.log})
}

fn audit_catalog(dex: &Dex, pool: &MetaPool, ls: &nc2000_engine::validate::Learnsets) -> Value {
    let bytes = std::fs::read("data/team-pool-v2/team-pool.json").unwrap();
    let catalog: MetaPool = serde_json::from_slice(&bytes).unwrap();
    let mut rows = vec![];
    let mut hp_attack_gaps = vec![];
    for t in &catalog.teams {
        let validation = nc2000_engine::validate::validate_team(
            dex,
            ls,
            &serde_json::to_string(&t.sets).unwrap(),
        );
        assert_eq!(validation["ok"], true, "catalog {}: {validation}", t.id);
        let b = fixture(dex, &t.sets);
        let obs = Observer::new(&b, 0);
        let belief = Belief::new(dex, pool, &obs);
        assert!(!belief.is_fallback(), "catalog fallback: {}", t.id);
        for (slot, p) in b.sides[1].roster.iter().enumerate() {
            let candidates = samples(dex, &b, &obs, &belief, slot);
            let actual = stats(p);
            let missing: Vec<_> = (0..6)
                .filter(|&k| {
                    !candidates
                        .iter()
                        .any(|c| c["stats"][k].as_i64() == Some(i64::from(actual[k])))
                })
                .map(|k| KEYS[k])
                .collect();
            if !missing.is_empty() {
                rows.push(json!({"team": t.id, "species": dex.species.key(p.species),
                    "actual_stats": actual, "missing_exact_stat_values": missing,
                    "preview_candidates": candidates}));
            }
            let has_hp = t.sets[slot]
                .moves
                .iter()
                .any(|m| nc2000_engine::dex::toid(m).starts_with("hiddenpower"));
            // base_move_slots canonicalize typed HP to generic Hidden Power;
            // use the original typed names for the Gen 2 category here.
            let has_physical = t.sets[slot].moves.iter().any(|m| {
                let id = dex.moves.id(&nc2000_engine::dex::toid(m)).unwrap();
                dex.moves.get(id).category == "Physical"
            });
            if !has_hp || has_physical {
                continue;
            }
            // Independently check the JS report's special Hidden Power gap
            // against native stat, gender, type and power calculations.
            for atk in 0..=p.set_ivs[1] / 2 {
                let mut sets = t.sets.clone();
                sets[slot]
                    .ivs
                    .as_mut()
                    .unwrap()
                    .insert("atk".into(), (atk * 2) as u8);
                sets[slot].evs.as_mut().unwrap().insert("atk".into(), 0);
                let changed = fixture(dex, &sets);
                let q = &changed.sides[1].roster[slot];
                if q.gender == p.gender
                    && q.base_maxhp == p.base_maxhp
                    && q.base_stored_stats[1..] == p.base_stored_stats[1..]
                    && q.base_hp_type == p.base_hp_type
                    && q.base_hp_power == p.base_hp_power
                {
                    let validation = nc2000_engine::validate::validate_team(
                        dex,
                        ls,
                        &serde_json::to_string(&sets).unwrap(),
                    );
                    assert_eq!(
                        validation["ok"], true,
                        "HP counterfactual {}: {validation}",
                        t.id
                    );
                    if q.base_stored_stats[0] < p.base_stored_stats[0] {
                        hp_attack_gaps.push(json!({"team": t.id, "species": dex.species.key(p.species),
                            "attack_dv_before": p.set_ivs[1] / 2, "attack_ev_before": p.set_evs[1],
                            "attack_dv_after": atk, "attack_ev_after": 0,
                            "attack_before": p.base_stored_stats[0], "attack_after": q.base_stored_stats[0],
                            "hp_power_before": p.base_hp_power, "hp_power_after": q.base_hp_power}));
                    }
                    break;
                }
            }
        }
    }
    json!({"catalog_sha256": format!("{:x}", Sha256::digest(&bytes)),
        "teams": catalog.teams.len(), "preview_fallbacks": 0,
        "mons_with_a_stat_value_absent_at_preview": rows,
        "native_special_hidden_power_attack_gaps": hp_attack_gaps})
}

fn main() {
    let output = std::env::args().nth(1).expect("OUTPUT.json required");
    let bytes = std::fs::read(PRIOR).unwrap();
    let raw: Value = serde_json::from_slice(&bytes).unwrap();
    let pool: MetaPool = serde_json::from_slice(&bytes).unwrap();
    let dex = conformance::load_dex();
    let ls = nc2000_engine::validate::Learnsets::from_json(
        &std::fs::read_to_string("data/learnsets-gen2.json").unwrap(),
    )
    .unwrap();
    let mut origins = BTreeMap::new();
    let mut affected_origins = BTreeMap::new();
    let mut affected_non_attack_origins = BTreeMap::new();
    let mut stat_counts = BTreeMap::new();
    let mut affected_mons = 0;
    let mut affected_non_attack_mons = 0;
    let mut details = vec![];
    for (index, team) in pool.teams.iter().enumerate() {
        let origin = raw["teams"][index]["provenance"]["origin"]
            .as_str()
            .unwrap();
        bump(&mut origins, origin);
        let max_sets = maximum_training(&team.sets);
        for sets in [&team.sets, &max_sets] {
            let validation = nc2000_engine::validate::validate_team(
                &dex,
                &ls,
                &serde_json::to_string(sets).unwrap(),
            );
            assert_eq!(validation["ok"], true, "{}: {validation}", team.id);
        }
        let original = fixture(&dex, &team.sets);
        let maximum = fixture(&dex, &max_sets);
        let deficits: Vec<Vec<usize>> = original.sides[1]
            .roster
            .iter()
            .zip(&maximum.sides[1].roster)
            .map(|(a, b)| (0..6).filter(|&k| stats(a)[k] < stats(b)[k]).collect())
            .collect();
        if deficits.iter().any(|d| !d.is_empty()) {
            bump(&mut affected_origins, origin);
        }
        let non_attack = deficits.iter().any(|d| d.iter().any(|&k| k != 1));
        if non_attack {
            bump(&mut affected_non_attack_origins, origin);
        }
        for d in &deficits {
            affected_mons += usize::from(!d.is_empty());
            affected_non_attack_mons += usize::from(d.iter().any(|&k| k != 1));
            for &k in d {
                bump(&mut stat_counts, KEYS[k]);
            }
        }
        if !non_attack {
            continue;
        }
        let obs = Observer::new(&maximum, 0);
        let belief = Belief::new(&dex, &pool, &obs);
        assert!(
            belief.alive().contains(&index),
            "source lost at preview: {}",
            team.id
        );
        let mut mons = vec![];
        for (slot, d) in deficits
            .iter()
            .enumerate()
            .filter(|(_, d)| d.iter().any(|&k| k != 1))
        {
            let mon = &original.sides[1].roster[slot];
            let target = stats(&maximum.sides[1].roster[slot]);
            let preview = samples(&dex, &maximum, &obs, &belief, slot);
            // Reveal all four moves of just this mon; no six-mon reveal or
            // whole-team selection assumption is needed.
            let moves: Vec<_> = (0..mon.base_move_slots.len()).collect();
            let (revealed, robs) = reveal(&dex, &maximum, slot, &moves);
            let rb = Belief::new(&dex, &pool, &robs);
            assert!(
                rb.alive().contains(&index),
                "source lost after reveals: {}",
                team.id
            );
            let after_moves = samples(&dex, &revealed, &robs, &rb, slot);
            let mut single_move_witnesses = vec![];
            for (other, p) in maximum.sides[1].roster.iter().enumerate() {
                for m in 0..p.base_move_slots.len() {
                    let (r, o) = reveal(&dex, &maximum, other, &[m]);
                    let b = Belief::new(&dex, &pool, &o);
                    let ss = samples(&dex, &r, &o, &b, slot);
                    let lost: Vec<_> = d
                        .iter()
                        .copied()
                        .filter(|&k| {
                            k != 1
                                && supports(&preview, k, target[k])
                                && !supports(&ss, k, target[k])
                        })
                        .map(|k| KEYS[k])
                        .collect();
                    if !lost.is_empty() {
                        single_move_witnesses.push(json!({
                            "species": dex.species.key(p.species),
                            "move": dex.moves.key(p.base_move_slots[m].id),
                            "lost_maximum_stats": lost, "candidates": ss
                        }));
                    }
                }
            }
            mons.push(
                json!({"species": dex.species.key(mon.species), "level": mon.level,
                "evs": mon.set_evs, "ivs": mon.set_ivs,
                "original_stats": stats(mon), "maximum_training_stats": target,
                "deficient_stats": d.iter().map(|&k| KEYS[k]).collect::<Vec<_>>(),
                "preview": preview, "after_own_moves": after_moves,
                "single_move_witnesses": single_move_witnesses}),
            );
        }
        details.push(json!({"id": team.id, "origin": origin, "weight": team.weight, "mons": mons}));
    }
    let report = json!({
        "format": "nc2000-prior-training-audit-v1",
        "prior": PRIOR, "prior_sha256": format!("{:x}", Sha256::digest(&bytes)),
        "stat_order": KEYS,
        "method": "Native engine stats and Belief candidate filtering; EV=255 counterfactual at unchanged DVs, loadout, gender and level. Synthetic single-mon move evidence plus one played legal Rain Dance turn; no search or win-rate measurement.",
        "summary": {"teams": pool.teams.len(), "origins": origins,
            "teams_with_any_training_deficit_by_origin": affected_origins,
            "teams_with_non_attack_training_deficit_by_origin": affected_non_attack_origins,
            "mons_with_any_training_deficit": affected_mons,
            "mons_with_non_attack_training_deficit": affected_non_attack_mons,
            "deficient_stat_counts": stat_counts},
        "non_attack_details": details,
        "catalog": audit_catalog(&dex, &pool, &ls),
        "played_rain_dance_witness": played_rain_dance_witness(&dex, &pool)
    });
    std::fs::write(
        output,
        serde_json::to_string_pretty(&report).unwrap() + "\n",
    )
    .unwrap();
    println!("{}", report["summary"]);
}
