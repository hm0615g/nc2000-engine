//! The shipped team artifacts (docs/TEAM-POOL-REBUILD-PLAN.md step 5) are
//! exactly what the classified inventory says they are: memberships follow
//! the labels, sets are the inventory's, weights are distributions, every
//! dominated verdict names a live replacement, and every shipped team is
//! legal and plays a game to completion.

use std::collections::{BTreeMap, BTreeSet};

use conformance::fixture::repo_root;
use conformance::load_dex;
use nc2000_engine::state::Battle;
use nc2000_engine::validate::{validate_team, Learnsets};
use serde_json::Value;

fn read(rel: &str) -> Value {
    let p = repo_root().join(rel);
    serde_json::from_str(&std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{rel}: {e}")))
        .unwrap_or_else(|e| panic!("{rel}: {e}"))
}

fn ids(v: &Value) -> Vec<String> {
    v["teams"].as_array().unwrap().iter().map(|t| t["id"].as_str().unwrap().to_string()).collect()
}

fn sum(v: &Value, key: &str) -> f64 {
    v["teams"].as_array().unwrap().iter().map(|t| t[key].as_f64().unwrap()).sum()
}

struct Products {
    inventory: BTreeMap<String, Value>,
    labels: BTreeMap<String, String>,
    own: Value,
    prior: Value,
    nash: Value,
    reference: Value,
}

fn products() -> Products {
    let inv = read("data/team-inventory-v1/inventory.json");
    let cls = read("data/team-inventory-v1/classification.json");
    Products {
        inventory: inv["teams"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| (t["id"].as_str().unwrap().to_string(), t.clone()))
            .collect(),
        labels: cls["labels"]
            .as_object()
            .unwrap()
            .iter()
            .map(|(k, v)| (k.clone(), v["label"].as_str().unwrap().to_string()))
            .collect(),
        own: read("data/team-pool-v1/team-pool.json"),
        prior: read("data/belief-pool-v2/belief-pool.json"),
        nash: read("data/meta-nash-v2/pool-artifact.json"),
        reference: read("data/team-inventory-v1/reference.json"),
    }
}

#[test]
fn memberships_follow_the_labels() {
    let p = products();
    let measured: Vec<&String> = p
        .inventory
        .iter()
        .filter(|(id, t)| t["measuredAs"].as_str() == Some(id.as_str()))
        .map(|(id, _)| id)
        .collect();
    for id in &measured {
        assert!(p.labels.contains_key(*id), "measured {id} has no label");
        assert_eq!(p.inventory[*id]["eligibility"]["status"], "eligible", "{id}");
    }
    let want_own: BTreeSet<String> =
        measured.iter().filter(|id| p.labels[**id] == "strong").map(|id| id.to_string()).collect();
    let want_prior: BTreeSet<String> =
        measured.iter().filter(|id| p.labels[**id] != "dominated").map(|id| id.to_string()).collect();
    assert_eq!(ids(&p.own).into_iter().collect::<BTreeSet<_>>(), want_own);
    assert_eq!(ids(&p.prior).into_iter().collect::<BTreeSet<_>>(), want_prior);
    for id in ids(&p.nash) {
        assert!(want_own.contains(&id), "Nash support {id} is not in the own-team pool");
    }
    for r in p.reference["teams"].as_array().unwrap() {
        let id = r["id"].as_str().unwrap();
        assert_eq!(r["ordinaryDraw"].as_bool().unwrap(), want_own.contains(id), "{id}");
        assert_eq!(r["opponentPrior"].as_bool().unwrap(), want_prior.contains(id), "{id}");
    }
}

#[test]
fn shipped_sets_are_the_inventory_sets() {
    let p = products();
    for file in [&p.own, &p.prior, &p.nash] {
        for t in file["teams"].as_array().unwrap() {
            let id = t["id"].as_str().unwrap();
            assert_eq!(t["sets"], p.inventory[id]["sets"], "{id}: sets differ from the inventory");
        }
    }
}

#[test]
fn weights_are_distributions() {
    let p = products();
    assert!((sum(&p.own, "drawWeight") - 1.0).abs() < 1e-3, "own-pool draw weights");
    assert!((sum(&p.nash, "weight") - 1.0).abs() < 5e-3, "nash weights");
    for t in p.prior["teams"].as_array().unwrap() {
        let w = t["weight"].as_f64().unwrap();
        assert!(w > 0.0 && w.is_finite(), "{}: prior weight {w}", t["id"]);
    }
    // No variant cluster buys weight by count.
    let mut per_cluster: BTreeMap<String, f64> = BTreeMap::new();
    for t in p.prior["teams"].as_array().unwrap() {
        let id = t["id"].as_str().unwrap();
        let c = p.inventory[id]["variantCluster"].as_str().unwrap_or(id).to_string();
        *per_cluster.entry(c).or_default() += t["weight"].as_f64().unwrap();
    }
    for (c, w) in per_cluster {
        assert!(w <= 1.0 + 1e-6, "cluster {c} carries weight {w}");
    }
}

#[test]
fn dominated_verdicts_name_a_live_replacement() {
    let p = products();
    let cls = read("data/team-inventory-v1/classification.json");
    for (id, l) in cls["labels"].as_object().unwrap() {
        if l["label"] != "dominated" {
            continue;
        }
        let r = l["replacement"]["id"].as_str().unwrap_or_else(|| panic!("{id}: no replacement"));
        assert_ne!(p.labels[r], "dominated", "{id}: replacement {r} is dominated");
        assert!(ids(&p.prior).contains(&r.to_string()), "{id}: replacement {r} is not in the prior");
        assert!(l["replacement"]["version"].is_string(), "{id}: replacement version");
        assert!(l["execution"].is_object(), "{id}: execution evidence");
    }
}

#[test]
fn every_shipped_team_is_legal_and_plays_out() {
    let p = products();
    let dex = load_dex();
    let ls = Learnsets::from_json(&std::fs::read_to_string(repo_root().join("data/learnsets-gen2.json")).unwrap())
        .unwrap();
    let mut seen = BTreeSet::new();
    let teams: Vec<&Value> = [&p.own, &p.prior, &p.nash]
        .iter()
        .flat_map(|f| f["teams"].as_array().unwrap().iter())
        .filter(|t| seen.insert(t["id"].as_str().unwrap().to_string()))
        .collect();
    for (i, t) in teams.iter().enumerate() {
        let id = t["id"].as_str().unwrap();
        let verdict = validate_team(&dex, &ls, &t["sets"].to_string());
        assert_eq!(verdict["ok"], true, "{id}: {verdict}");
        let sets: Vec<nc2000_engine::battle::PokemonSet> = serde_json::from_value(t["sets"].clone()).unwrap();
        let opp: Vec<nc2000_engine::battle::PokemonSet> =
            serde_json::from_value(teams[(i + 1) % teams.len()]["sets"].clone()).unwrap();
        let mut battle = Battle::from_fixture(&dex, "1,2,3,4", &sets, &opp).unwrap();
        battle.set_log_enabled(false);
        let mut x: u64 = 0x5EED ^ i as u64;
        let mut steps = 0;
        while battle.outcome().is_none() {
            steps += 1;
            assert!(steps < 10_000, "{id}: no termination");
            let picks = [0usize, 1].map(|s| {
                let legal = battle.legal_choices(&dex, s);
                x = x.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
                (!legal.is_empty()).then(|| legal[(x >> 33) as usize % legal.len()])
            });
            battle.apply_choices(&dex, picks).unwrap();
        }
    }
}
