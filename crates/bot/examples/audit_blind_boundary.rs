//! Reproduce hidden-state exposure without changing production behavior.
//! cargo run --release -p nc2000-bot --example audit_blind_boundary -- OUTPUT.json
use std::{collections::BTreeMap, path::Path};

use nc2000_bot::{
    import::announce_hp,
    player::{PlayerChannel, PlayerFrame},
    preview::{load_meta_pool, MetaPool},
    smmcts::SelRule,
    Belief, BlindSearch, Observer, ProtocolAgent, RmConfig, SplitMix64,
};
use nc2000_engine::{
    battle::PokemonSet,
    dex::Dex,
    state::{Battle, Status, DK},
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

fn config(iterations: u32) -> RmConfig {
    let profile: Value =
        serde_json::from_str(include_str!("../../../data/search-profiles.json")).unwrap();
    RmConfig {
        iterations,
        rule: SelRule::Ucb,
        c: profile["blind"]["c"].as_f64().unwrap(),
        ..Default::default()
    }
}

fn frame(dex: &Dex, battle: &Battle, side: usize) -> PlayerFrame {
    PlayerChannel::new(side)
        .frame(&mut battle.clone(), dex)
        .unwrap()
}

fn health(battle: &Battle, side: usize) -> Value {
    let p = battle.poke(battle.active_id(side).unwrap());
    json!({"hp": p.hp, "maxhp": p.maxhp, "status": p.status.as_str(),
        "sleep_time": p.status_state.get_int(DK::Time),
        "sleep_start_time": p.status_state.get_int(DK::StartTime)})
}

fn direct(dex: &Dex, pool: &MetaPool, battle: &Battle, side: usize, iterations: u32) -> Value {
    let mut obs = Observer::new(battle, side);
    obs.observe(battle, dex);
    let mut belief = Belief::new(dex, pool, &obs);
    belief.sync(dex, &obs);
    let modeled = belief.determinize(dex, battle, &obs, &mut SplitMix64::new(123));
    let mut search = BlindSearch::new(battle, dex, config(iterations), side, 97);
    let mask = search.dominated().to_vec();
    assert_eq!(search.step(dex, &belief, &obs, iterations), iterations);
    let foe = modeled.poke(modeled.active_id(1 - side).unwrap());
    json!({"modeled_foe": health(&modeled, 1-side),
        "modeled_foe_item": foe.item.map(|id| dex.items.key(id)),
        "candidates": belief.candidate_count(),
        "actions": search.actions().iter().map(|c| c.to_input(dex)).collect::<Vec<_>>(),
        "masked": mask, "visits": search.visits(), "means": search.means(),
        "best": search.best().map(|c| c.to_input(dex)), "iterations": search.iterations()})
}

fn protocol(
    dex: &Dex,
    pool: &MetaPool,
    battle: &Battle,
    side: usize,
    own: &[PokemonSet],
    iterations: u32,
) -> Value {
    let f = frame(dex, battle, side);
    let mut agent = ProtocolAgent::new(dex, side, pool.clone(), config(iterations), 97);
    agent.set_own_team(own.to_vec());
    for line in &f.lines {
        agent.push_line(dex, line);
    }
    assert!(agent.on_request(dex, &f.request.to_string()).unwrap());
    assert_eq!(agent.step(dex, iterations).unwrap(), iterations);
    let search = agent.search().unwrap();
    json!({"modeled_foe": health(agent.battle().unwrap(), 1-side),
        "visits": search.visits(), "means": search.means(),
        "best": search.best().map(|c| c.to_input(dex)), "iterations": agent.iterations()})
}

fn append_hp(battle: &mut Battle, side: usize) {
    let id = battle.active_id(side).unwrap();
    let subject = battle.poke(id).name.as_str();
    let subject = format!("p{}a: {subject}", side + 1);
    let (exact, public) = battle.get_health(id);
    battle.add_split(
        if side == 0 { "p1" } else { "p2" },
        &["-damage", &subject, &exact],
        &["-damage", &subject, &public],
    );
}

fn mon(species: &str, moves: &[&str], item: &str) -> PokemonSet {
    serde_json::from_value(json!({"species":species, "name":species, "level":50,
        "moves":moves, "item":item}))
    .unwrap()
}

fn started(dex: &Dex, teams: [&[PokemonSet]; 2]) -> Battle {
    let mut b = Battle::from_fixture(dex, "1,2,3,4", teams[0], teams[1]).unwrap();
    let picks = [0, 1].map(|s| Some(b.legal_choices(dex, s)[0]));
    b.apply_choices(dex, picks).unwrap();
    b
}

fn main() {
    let output = std::env::args().nth(1).expect("OUTPUT.json");
    let dex = conformance::load_dex();
    let pool_path = "data/belief-pool-v3/belief-pool.json";
    let catalog_path = "data/team-pool-v2/team-pool.json";
    let pool = load_meta_pool(Path::new(pool_path));
    let catalog = load_meta_pool(Path::new(catalog_path));
    let live: Value = serde_json::from_str(include_str!("../../../data/live-search.json")).unwrap();
    let budgets = [
        live["battle"]["standard"].as_u64().unwrap() as u32,
        live["battle"]["strong"].as_u64().unwrap() as u32,
    ];
    let mut hp_cases = Vec::new();
    for side in 0..2 {
        // Real catalog/prior teams and normal 3-of-6 preview selection. Only
        // current foe HP is intervened on; this is a snapshot differential,
        // not a claim that a scripted battle reaches both entire histories.
        let own = &catalog.teams[0].sets;
        let foe = &pool.teams[0].sets;
        let teams = if side == 0 {
            [own.as_slice(), foe.as_slice()]
        } else {
            [foe.as_slice(), own.as_slice()]
        };
        let base = started(&dex, teams);
        let id = base.active_id(1 - side).unwrap();
        let maxhp = base.poke(id).maxhp;
        let hp = (maxhp / 3..maxhp - 1)
            .find(|&h| {
                announce_hp(h, maxhp, 100) == announce_hp(h + 1, maxhp, 100)
                    && (100.0 * h as f64 / maxhp as f64).round()
                        == (100.0 * (h + 1) as f64 / maxhp as f64).round()
            })
            .expect("two HP values with identical public and rounded UI labels");
        let roots = [hp, hp + 1].map(|h| {
            let mut b = base.clone();
            b.poke_mut(id).hp = h;
            append_hp(&mut b, 1 - side);
            b
        });
        let frames = roots.each_ref().map(|b| frame(&dex, b, side));
        assert_eq!(
            frames[0], frames[1],
            "public projection must be independent of the determinizer"
        );
        let mut runs = Vec::new();
        for budget in budgets {
            let raw = roots
                .each_ref()
                .map(|b| direct(&dex, &pool, b, side, budget));
            let sanitized = roots
                .each_ref()
                .map(|b| protocol(&dex, &pool, b, side, own, budget));
            assert_eq!(raw[0]["modeled_foe"]["hp"], hp);
            assert_eq!(raw[1]["modeled_foe"]["hp"], hp + 1);
            assert_ne!(
                raw[0]["means"], raw[1]["means"],
                "exact HP must reach decision values"
            );
            assert_eq!(
                sanitized[0], sanitized[1],
                "public-only path must be invariant"
            );
            eprintln!("HP side={side} budget={budget}: direct differs; protocol identical");
            runs.push(json!({"budget":budget, "direct":raw, "protocol":sanitized}));
        }
        // Positive control: equality above must not come from ignoring HP.
        let mut changed = base.clone();
        changed.poke_mut(id).hp = maxhp / 2;
        append_hp(&mut changed, 1 - side);
        assert_ne!(frame(&dex, &changed, side), frames[0]);
        let public_change = protocol(&dex, &pool, &changed, side, own, budgets[0]);
        assert_ne!(
            public_change["modeled_foe"]["hp"],
            runs[0]["protocol"][0]["modeled_foe"]["hp"]
        );
        assert_ne!(public_change["means"], runs[0]["protocol"][0]["means"]);
        hp_cases.push(
            json!({"side":side, "own_team":catalog.teams[0].id, "foe_team":pool.teams[0].id,
            "truth_hp":[hp,hp+1], "maxhp":maxhp, "public_hp":announce_hp(hp,maxhp,100),
            "public_frames_equal":true, "frame":frames[0], "runs":runs,
            "public_change_control":public_change}),
        );
    }

    // Hidden maximum HP must not be replaced while retaining absolute HP:
    // check EVERY integral true HP on the active catalog/prior lead pairs.
    let mut maxhp_checks = 0;
    let mut maxhp_violations = 0;
    let mut maxhp_example = None;
    let mut full_hp_checks = 0;
    let mut full_hp_violations = 0;
    let mut full_hp_example = None;
    for (label, source) in [("catalog", &catalog), ("prior", &pool)] {
        for team in &source.teams {
            let mut truth = team.sets.clone();
            let ivs = truth[0].ivs.get_or_insert_with(BTreeMap::new);
            for key in ["hp", "atk", "def", "spa", "spd", "spe"] {
                ivs.insert(key.into(), 0);
            }
            let mut b = started(&dex, [&catalog.teams[0].sets, &truth]);
            let mut obs = Observer::new(&b, 0);
            obs.observe(&b, &dex);
            let mut belief = Belief::new(&dex, &pool, &obs);
            belief.sync(&dex, &obs);
            let id = b.active_id(1).unwrap();
            for hp in 1..=b.poke(id).maxhp {
                b.poke_mut(id).hp = hp;
                let det = belief.determinize(&dex, &b, &obs, &mut SplitMix64::new(123));
                let public = announce_hp(hp, b.poke(id).maxhp, 100);
                let modeled = announce_hp(det.poke(id).hp, det.poke(id).maxhp, 100);
                maxhp_checks += 1;
                if modeled != public {
                    maxhp_violations += 1;
                    maxhp_example.get_or_insert(json!({"source":label,"team":team.id,
                        "truth":health(&b,1),"model":health(&det,1),"public_percent":public,"modeled_percent":modeled}));
                }
                if hp == b.poke(id).maxhp {
                    full_hp_checks += 1;
                    if modeled != public {
                        full_hp_violations += 1;
                        full_hp_example.get_or_insert(json!({"source":label,"team":team.id,
                            "truth":health(&b,1),"model":health(&det,1),"public_percent":public,"modeled_percent":modeled}));
                    }
                }
            }
        }
    }
    assert!(maxhp_violations > 0);

    // Non-Rest sleep is a hidden random duration. Keep both complete player
    // frames equal while changing only the engine's remaining counter.
    let own = [mon("Raikou", &["Thunderbolt", "Rest"], "")];
    let foe = [mon("Snorlax", &["Body Slam", "Rest"], "")];
    let mut base = started(&dex, [&own, &foe]);
    let id = base.active_id(1).unwrap();
    let source = base.active_id(0);
    base.restore_status(&dex, id, Status::Slp, source);
    base.add(&["-status", "p2a: Snorlax", "slp"]);
    let roots = [1, 3].map(|time| {
        let mut b = base.clone();
        b.poke_mut(id).status_state.set_int(DK::Time, time);
        b
    });
    assert_eq!(frame(&dex, &roots[0], 0), frame(&dex, &roots[1], 0));
    let raw = roots
        .each_ref()
        .map(|b| direct(&dex, &pool, b, 0, budgets[1]));
    let sanitized = roots
        .each_ref()
        .map(|b| protocol(&dex, &pool, b, 0, &own, budgets[1]));
    assert_eq!(raw[0]["modeled_foe"]["sleep_time"], 1);
    assert_eq!(raw[1]["modeled_foe"]["sleep_time"], 3);
    assert_ne!(raw[0]["means"], raw[1]["means"]);
    assert_eq!(sanitized[0], sanitized[1]);

    // Root masking runs BEFORE determinization. Even the observing side's
    // not-yet-visible Quick Claw coin must not change the decision policy.
    let own_claw = [mon("Snorlax", &["Rest", "Swagger"], "Quick Claw")];
    let foe_claw = [mon("Snorlax", &["Body Slam"], "")];
    let base = started(&dex, [&own_claw, &foe_claw]);
    let roots = [false, true].map(|coin| {
        let mut b = base.clone();
        b.quick_claw_roll = coin;
        b.update_all_speeds(&dex);
        b
    });
    assert_eq!(frame(&dex, &roots[0], 0), frame(&dex, &roots[1], 0));
    let claw = roots
        .each_ref()
        .map(|b| direct(&dex, &pool, b, 0, budgets[1]));
    assert_ne!(claw[0]["masked"], claw[1]["masked"]);

    // Changing a NEVER revealed item with unchanged public item presence
    // also changes root masking, even though determinization overwrites it.
    let own_item = [mon("Starmie", &["Recover", "Surf"], "")];
    let roots = ["Leftovers", "Quick Claw"].map(|item| {
        let foe = [mon("Snorlax", &["Body Slam"], item)];
        started(&dex, [&own_item, &foe])
    });
    assert_eq!(frame(&dex, &roots[0], 0), frame(&dex, &roots[1], 0));
    let items = roots
        .each_ref()
        .map(|b| direct(&dex, &pool, b, 0, budgets[1]));
    assert_eq!(items[0]["modeled_foe_item"], items[1]["modeled_foe_item"]);
    assert_ne!(items[0]["masked"], items[1]["masked"]);
    let item_protocol = roots
        .each_ref()
        .map(|b| protocol(&dex, &pool, b, 0, &own_item, budgets[1]));
    assert_eq!(item_protocol[0], item_protocol[1]);

    let inputs = [
        pool_path,
        catalog_path,
        "data/search-profiles.json",
        "data/live-search.json",
        "crates/bot/src/belief.rs",
        "crates/bot/src/blind.rs",
        "crates/bot/src/import.rs",
        "crates/bot/src/smmcts.rs",
        "crates/bot/src/player.rs",
        "crates/wasm/src/lib.rs",
        "web/src/bot-worker.ts",
        "crates/bot/examples/audit_blind_boundary.rs",
    ];
    let hashes: BTreeMap<_, _> = inputs
        .iter()
        .map(|path| {
            (
                *path,
                format!("{:x}", Sha256::digest(std::fs::read(path).unwrap())),
            )
        })
        .collect();
    let result = json!({"schema":"nc2000-blind-boundary-audit-v1", "source_sha256":hashes,
        "config":{"budgets":budgets,"c":config(1).c,"rule":"Ucb","hp_buckets":16,"other_fields":"RmConfig::default"},
        "hp":hp_cases,
        "maxhp":{"checks":maxhp_checks,"percent_contradictions":maxhp_violations,"example":maxhp_example,
            "full_hp_checks":full_hp_checks,"full_hp_contradictions":full_hp_violations,"full_hp_example":full_hp_example},
        "sleep":{"public_frames_equal":true,"direct":raw,"protocol":sanitized},
        "quick_claw":{"public_frames_equal":true,"direct":claw},
        "unrevealed_item":{"public_frames_equal":true,"direct":items,"protocol":item_protocol},
        "scope":"Constructed differential snapshots; no production runtime change, deployment, prevalence or win-rate claim."});
    std::fs::write(
        output,
        serde_json::to_string_pretty(&result).unwrap() + "\n",
    )
    .unwrap();
}
