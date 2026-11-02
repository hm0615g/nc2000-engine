//! Diagnose PS root exclusions with an unchanged public view and prior distribution.
//! cargo run --release -p nc2000-bot --example audit_protocol_exclusions -- OUTPUT.json
use nc2000_bot::{
    import::announce_hp,
    player::PlayerChannel,
    preview::{MetaPool, MetaTeam},
    smmcts::SelRule,
    ProtocolAgent, RmConfig, SplitMix64,
};
use nc2000_engine::{battle::PokemonSet, state::Battle};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

fn mon(species: &str, item: &str, moves: &[&str], speed_ev: i32) -> PokemonSet {
    serde_json::from_value(json!({
        "species":species,"name":species,"gender":"M","item":item,"level":50,"moves":moves,
        "ivs":{"hp":30,"atk":30,"def":30,"spa":30,"spd":30,"spe":30},
        "evs":{"hp":255,"atk":255,"def":255,"spa":255,"spd":255,"spe":speed_ev}
    }))
    .unwrap()
}

fn main() {
    let output = std::env::args().nth(1).expect("OUTPUT.json");
    let dex = conformance::load_dex();
    let profile: Value =
        serde_json::from_str(include_str!("../../../data/search-profiles.json")).unwrap();
    let live: Value = serde_json::from_str(include_str!("../../../data/live-search.json")).unwrap();
    let iterations = live["battle"]["strong"].as_u64().unwrap() as u32;
    let cfg = RmConfig {
        rule: SelRule::Ucb,
        c: profile["blind"]["c"].as_f64().unwrap(),
        iterations,
        ..Default::default()
    };
    let cases = [
        (
            "unrevealed-item",
            mon("Starmie", "", &["Recover", "Surf"], 255),
            [
                mon("Snorlax", "Leftovers", &["Body Slam"], 255),
                mon("Snorlax", "Quick Claw", &["Body Slam"], 255),
            ],
        ),
        // At level 50 these legal speed spreads put an opposing Starmie
        // below and above our partially trained one, with identical preview.
        (
            "unrevealed-speed",
            mon("Starmie", "", &["Recover", "Surf"], 128),
            [
                mon("Starmie", "Leftovers", &["Surf"], 0),
                mon("Starmie", "Leftovers", &["Surf"], 255),
            ],
        ),
    ];
    let mut records = Vec::new();
    for (name, own, candidates) in cases {
        let mut battle =
            Battle::from_fixture(&dex, "1,2,3,4", &[own.clone()], &[candidates[0].clone()])
                .unwrap();
        battle.choose(&dex, 0, "team 1").unwrap();
        battle.choose(&dex, 1, "team 1").unwrap();
        let frame = PlayerChannel::new(0).frame(&mut battle, &dex).unwrap();
        let pool = MetaPool {
            teams: candidates
                .iter()
                .enumerate()
                .map(|(i, mon)| MetaTeam {
                    id: format!("candidate-{i}"),
                    sets: vec![mon.clone()],
                    weight: Some(1.0),
                })
                .collect(),
        };
        let mut runs = Vec::new();
        for reversed in [false, true] {
            let mut pool = pool.clone();
            if reversed {
                pool.teams.reverse();
            }
            let mut agent = ProtocolAgent::new(&dex, 0, pool, cfg.clone(), 97);
            agent.set_own_team(vec![own.clone()]);
            for line in &frame.lines {
                agent.push_line(&dex, line);
            }
            assert!(agent.on_request(&dex, &frame.request.to_string()).unwrap());
            assert_eq!(agent.belief().unwrap().candidate_count(), 2);
            let b = agent.battle().unwrap();
            let own = b.active_id(0).unwrap();
            let foe = b.active_id(1).unwrap();
            let root = json!({
                "own_speed":b.get_pokemon_action_speed(&dex, own),
                "foe_speed":b.get_pokemon_action_speed(&dex, foe),
                "foe_item":b.poke(foe).item.map(|id|dex.items.key(id)),
                "quick_claw_roll":b.quick_claw_roll,
            });
            let search = agent.search().unwrap();
            let actions: Vec<_> = search.actions().iter().map(|a| a.to_input(&dex)).collect();
            let mask = search.dominated().to_vec();
            assert_eq!(agent.step(&dex, iterations).unwrap(), iterations);
            let search = agent.search().unwrap();
            let visits = search.visits().to_vec();
            let means = search.means();
            let best = agent.best(&dex).unwrap();
            runs.push(json!({"reversed":reversed,"candidates":2,"root":root,
                "actions":actions,"excluded":mask,"visits":visits,"means":means,"best":best}));
        }
        assert_eq!(runs[0]["actions"], runs[1]["actions"]);
        assert_eq!(runs[0]["excluded"], json!([true, false]));
        assert_eq!(runs[1]["excluded"], json!([false, false]));
        records.push(json!({"case":name,"same_player_frame":true,
            "same_weighted_prior_up_to_order":true,"frame":frame,"runs":runs}));
    }
    // The public HP is projected using the first candidate maximum, but
    // shared determinization can then replace that maximum without scaling HP.
    let own = mon("Starmie", "", &["Recover", "Surf"], 255);
    let high_hp = mon("Snorlax", "Leftovers", &["Body Slam"], 255);
    let mut low_hp = high_hp.clone();
    low_hp.evs.as_mut().unwrap().insert("hp".into(), 0);
    let mut battle =
        Battle::from_fixture(&dex, "1,2,3,4", &[own.clone()], &[high_hp.clone()]).unwrap();
    battle.choose(&dex, 0, "team 1").unwrap();
    battle.choose(&dex, 1, "team 1").unwrap();
    let foe = battle.active_id(1).unwrap();
    battle.poke_mut(foe).hp = battle.poke(foe).maxhp / 2;
    let (secret, shared) = battle.get_health(foe);
    assert_eq!(shared, "50/100");
    battle.add_split(
        "p2",
        &["-damage", "p2a: Snorlax", &secret],
        &["-damage", "p2a: Snorlax", &shared],
    );
    let frame = PlayerChannel::new(0).frame(&mut battle, &dex).unwrap();
    let pool = MetaPool {
        teams: [high_hp, low_hp]
            .into_iter()
            .enumerate()
            .map(|(i, m)| MetaTeam {
                id: format!("hp-{i}"),
                sets: vec![m],
                weight: Some(1.0),
            })
            .collect(),
    };
    let mut agent = ProtocolAgent::new(&dex, 0, pool, cfg.clone(), 97);
    agent.set_own_team(vec![own]);
    for line in &frame.lines {
        agent.push_line(&dex, line);
    }
    assert!(agent.on_request(&dex, &frame.request.to_string()).unwrap());
    let belief = agent.belief().unwrap();
    assert_eq!(belief.candidate_count(), 2);
    let hp_models: Vec<_> = belief
        .alive()
        .iter()
        .map(|&candidate| {
            let b = belief.determinize_with(
                &dex,
                agent.battle().unwrap(),
                agent.observer().unwrap(),
                Some(candidate),
                &mut SplitMix64::new(123),
            );
            let p = b.poke(b.active_id(1).unwrap());
            json!({"candidate":belief.candidate_id(candidate), "hp":p.hp, "maxhp":p.maxhp,
            "announced_percent":announce_hp(p.hp,p.maxhp,100)})
        })
        .collect();
    assert_eq!(hp_models[0]["announced_percent"], 50);
    assert_ne!(hp_models[1]["announced_percent"], 50);
    let paths = [
        "crates/bot/examples/audit_protocol_exclusions.rs",
        "crates/bot/src/import.rs",
        "crates/bot/src/blind.rs",
        "crates/bot/src/smmcts.rs",
        "data/live-search.json",
        "data/search-profiles.json",
    ];
    let hashes: std::collections::BTreeMap<_, _> = paths
        .iter()
        .map(|p| {
            (
                *p,
                format!("{:x}", Sha256::digest(std::fs::read(p).unwrap())),
            )
        })
        .collect();
    let record = json!({"schema":"nc2000-protocol-exclusions-audit-v1","source_sha256":hashes,
        "iterations":iterations,"c":cfg.c,"rule":"Ucb","cases":records,
        "hp_consistency":{"public_percent":50,"models":hp_models},
        "scope":"Constructed one-mon fixtures. Prior-order-dependent exclusions are model assumptions, not access to true opponent secrets. No production changes."});
    std::fs::write(
        output,
        serde_json::to_string_pretty(&record).unwrap() + "\n",
    )
    .unwrap();
}
