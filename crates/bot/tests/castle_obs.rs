use std::collections::BTreeSet;

use conformance::load_dex;
use nc2000_bot::blind::resample_hidden_timers;
use nc2000_bot::duel::DuelSpec;
use nc2000_bot::import::{announce_hp_mode, impute_hp_mode, ProtocolAgent, ProtocolTracker};
use nc2000_bot::player_state::{HiddenTimerKind, HiddenTimerSpec};
use nc2000_bot::preview::{BakeCfg, MetaPool, MetaTeam};
use nc2000_bot::runner::{effective_turn_cap, judge_castle_turn_limit};
use nc2000_bot::teamgen::TeamGen;
use nc2000_bot::{Belief, Observer, RmConfig, SplitMix64};
use nc2000_engine::battle::{Outcome, PokemonSet};
use nc2000_engine::dex::{toid, Dex};
use nc2000_engine::rules::Ruleset;
use nc2000_engine::state::{ActionKind, Battle, Status, DK};

fn make_set(species: &str, level: u8, item: &str, moves: &[&str]) -> PokemonSet {
    serde_json::from_value(serde_json::json!({
        "name": species,
        "species": species,
        "level": level,
        "item": item,
        "gender": "M",
        "moves": moves,
        "ivs": { "hp": 30, "atk": 30, "def": 30, "spa": 30, "spd": 30, "spe": 30 },
        "evs": { "hp": 255, "atk": 255, "def": 255, "spa": 255, "spd": 255, "spe": 255 }
    }))
    .unwrap()
}

fn make_set_evs(species: &str, level: u8, item: &str, moves: &[&str], hp_ev: u16, hp_iv: u8) -> PokemonSet {
    serde_json::from_value(serde_json::json!({
        "name": species,
        "species": species,
        "level": level,
        "item": item,
        "gender": "M",
        "moves": moves,
        "ivs": { "hp": hp_iv, "atk": 30, "def": 30, "spa": 30, "spd": 30, "spe": 30 },
        "evs": { "hp": hp_ev, "atk": 255, "def": 255, "spa": 255, "spd": 255, "spe": 255 }
    }))
    .unwrap()
}

fn sample_six(dex: &Dex) -> Vec<PokemonSet> {
    let _ = dex;
    vec![
        make_set("Snorlax", 55, "Leftovers", &["Body Slam", "Earthquake", "Curse", "Rest"]),
        make_set("Zapdos", 50, "Mint Berry", &["Thunderbolt", "Drill Peck", "Rest", "Sleep Talk"]),
        make_set("Starmie", 50, "Miracle Berry", &["Surf", "Psychic", "Recover", "Rapid Spin"]),
        make_set("Gengar", 50, "Gold Berry", &["Thunderbolt", "Ice Punch", "Destiny Bond", "Explosion"]),
        make_set("Raikou", 50, "Magnet", &["Thunderbolt", "Roar", "Rest", "Sleep Talk"]),
        make_set("Exeggutor", 50, "Pink Bow", &["Psychic", "Giga Drain", "Stun Spore", "Explosion"]),
    ]
}

#[test]
fn observer_castle_iceberry_dual_cure_and_bitterberry_inert() {
    let dex = load_dex();
    let team = vec![make_set("Snorlax", 50, "Leftovers", &["Body Slam", "Rest", "Curse", "Earthquake"])];

    // PS mode: confusion excludes bitterberry, NOT iceberry
    let b_ps = Battle::from_fixture_with_ruleset(&dex, "1,2,3,4", &team, &team, Ruleset::PsStadium2).unwrap();
    let mut obs_ps = Observer::new(&b_ps, 0);
    assert_eq!(obs_ps.ruleset(), Ruleset::PsStadium2);
    for line in [
        "|switch|p1a: Snorlax|Snorlax, L50, M|100/100",
        "|switch|p2a: Snorlax|Snorlax, L50, M|100/100",
        "|-start|p2a: Snorlax|confusion",
        "|turn|2",
    ] {
        obs_ps.ingest_line(line, &dex);
    }
    assert!(Observer::is_item_excluded_for_slot(&dex, obs_ps.mons(), 0, dex.items.id("bitterberry")));
    assert!(!Observer::is_item_excluded_for_slot(&dex, obs_ps.mons(), 0, dex.items.id("iceberry")));

    // Castle mode: confusion excludes iceberry (ID 81) and NEVER excludes bitterberry (ID 84)
    let b_castle = Battle::from_fixture_with_ruleset(
        &dex,
        "1,2,3,4",
        &team,
        &team,
        Ruleset::castle_tournament(),
    )
    .unwrap();
    let mut obs_castle = Observer::new(&b_castle, 0);
    assert!(obs_castle.ruleset().is_castle());
    for line in [
        "|switch|p1a: Snorlax|Snorlax, L50, M|266/266",
        "|switch|p2a: Snorlax|Snorlax, L50, M|266/266",
        "|-start|p2a: Snorlax|confusion",
        "|turn|2",
    ] {
        obs_castle.ingest_line(line, &dex);
    }
    assert!(Observer::is_item_excluded_for_slot(&dex, obs_castle.mons(), 0, dex.items.id("iceberry")));
    assert!(!Observer::is_item_excluded_for_slot(&dex, obs_castle.mons(), 0, dex.items.id("bitterberry")));

    // Castle mode: Burn ALSO excludes iceberry (ID 81) and NEVER excludes bitterberry
    let mut obs_brn = Observer::new(&b_castle, 0);
    for line in [
        "|switch|p1a: Snorlax|Snorlax, L50, M|266/266",
        "|switch|p2a: Snorlax|Snorlax, L50, M|266/266",
        "|-status|p2a: Snorlax|brn",
        "|turn|2",
    ] {
        obs_brn.ingest_line(line, &dex);
    }
    assert!(Observer::is_item_excluded_for_slot(&dex, obs_brn.mons(), 0, dex.items.id("iceberry")));
    assert!(!Observer::is_item_excluded_for_slot(&dex, obs_brn.mons(), 0, dex.items.id("bitterberry")));

    // Castle mode: curing confusion via "Bitter Berry" display disguise maps to iceberry!
    let mut obs_cure = Observer::new(&b_castle, 0);
    for line in [
        "|switch|p1a: Snorlax|Snorlax, L50, M|266/266",
        "|switch|p2a: Snorlax|Snorlax, L50, M|266/266",
        "|-start|p2a: Snorlax|confusion",
        "|-enditem|p2a: Snorlax|Bitter Berry",
        "|-end|p2a: Snorlax|confusion",
    ] {
        obs_cure.ingest_line(line, &dex);
    }
    assert_eq!(obs_cure.mons()[0].item.original, Some(dex.items.id("iceberry")));
    assert_eq!(obs_cure.mons()[0].item.current, Some(None));
}

#[test]
fn observer_castle_observed_maxhp_and_belief_exact_hp_filtering() {
    let dex = load_dex();
    let lax_full = make_set_evs("Snorlax", 50, "Leftovers", &["Body Slam", "Rest", "Curse", "Earthquake"], 255, 30);
    let lax_low = make_set_evs("Snorlax", 50, "Leftovers", &["Body Slam", "Rest", "Curse", "Earthquake"], 0, 0);

    let b_full = Battle::from_fixture_with_ruleset(
        &dex,
        "1,2,3,4",
        std::slice::from_ref(&lax_full),
        std::slice::from_ref(&lax_full),
        Ruleset::castle_tournament(),
    )
    .unwrap();
    let maxhp_full = b_full.sides[1].roster[0].base_maxhp;
    let b_low = Battle::from_fixture_with_ruleset(
        &dex,
        "1,2,3,4",
        std::slice::from_ref(&lax_low),
        std::slice::from_ref(&lax_low),
        Ruleset::castle_tournament(),
    )
    .unwrap();
    let maxhp_low = b_low.sides[1].roster[0].base_maxhp;
    assert_ne!(maxhp_full, maxhp_low);

    let pool = MetaPool {
        teams: vec![
            MetaTeam { id: "full".into(), sets: vec![lax_full.clone()], weight: None },
            MetaTeam { id: "low".into(), sets: vec![lax_low.clone()], weight: None },
        ],
    };

    let mut battle = b_full.clone();
    battle.sides[1].roster[0].is_active = true;
    battle.sides[1].roster[0].previously_switched_in = 1;
    // Pick an HP value that PS 100-step quantization would perturb:
    // For maxhp = 266, cur = 134 -> announce_hp(134, 266, 100) = 51 -> impute_hp(51, 100, 266) = 135 != 134.
    let target_hp = (1..maxhp_full)
        .find(|&h| {
            let pct = nc2000_bot::import::announce_hp(h, maxhp_full, 100);
            nc2000_bot::import::impute_hp(pct, 100, maxhp_full) != h
        })
        .expect("must have at least one quantized HP mismatch");
    battle.sides[1].roster[0].hp = target_hp;

    let mut obs = Observer::new(&battle, 0);
    assert_eq!(obs.mons()[0].observed_maxhp, None);
    let mut belief = Belief::new(&dex, &pool, &obs);
    // Before appearing, both candidates match preview (species, level, gender, item flag).
    assert_eq!(belief.alive(), &[0, 1]);

    // After observing in Castle mode, exact maxhp is revealed -> low-HP candidate rejected!
    obs.observe(&battle, &dex);
    assert_eq!(obs.mons()[0].observed_maxhp, Some(maxhp_full));
    belief.sync(&dex, &obs);
    assert_eq!(belief.alive(), &[0]);

    // Castle determinization preserves exact HP without 100-step quantization!
    let mut rng = SplitMix64::new(42);
    let det_castle = belief.determinize(&dex, &battle, &obs, &mut rng);
    assert_eq!(det_castle.sides[1].roster[0].hp, target_hp);

    // PS determinization still quantizes through 100-step percentages identically.
    let mut battle_ps = battle.clone();
    battle_ps.ruleset = Ruleset::PsStadium2;
    let mut obs_ps = Observer::new(&battle_ps, 0);
    obs_ps.observe(&battle_ps, &dex);
    assert_eq!(obs_ps.mons()[0].observed_maxhp, None);
    let mut belief_ps = Belief::new(&dex, &pool, &obs_ps);
    belief_ps.sync(&dex, &obs_ps);
    assert_eq!(belief_ps.alive(), &[0, 1]);
    let det_ps = belief_ps.determinize_with(&dex, &battle_ps, &obs_ps, Some(0), &mut rng);
    assert_ne!(det_ps.sides[1].roster[0].hp, target_hp);
}

#[test]
fn observer_castle_mystery_berry_pp_over_maxpp_and_turn_cut_upkeep() {
    let dex = load_dex();
    let team = vec![make_set("Smeargle", 50, "Mystery Berry", &["Sketch", "Spore", "Baton Pass", "Recover"])];
    let mut battle = Battle::from_fixture_with_ruleset(
        &dex,
        "1,2,3,4",
        &team,
        &team,
        Ruleset::castle_tournament(),
    )
    .unwrap();

    // Simulate Castle Mystery Berry restoring Sketch PP from 0 to 5 (> maxpp 1) without `used` flag
    battle.sides[1].roster[0].base_move_slots[0].maxpp = 1;
    battle.sides[1].roster[0].base_move_slots[0].pp = 5;
    battle.sides[1].roster[0].base_move_slots[0].used = false;
    battle.sides[1].roster[0].is_active = true;

    let mut obs = Observer::new(&battle, 0);
    obs.observe(&battle, &dex);
    let sketch_id = dex.moves.id("sketch").unwrap();
    assert!(
        obs.mons()[0].revealed_moves.contains(&sketch_id),
        "Castle observer must detect revealed move when Mystery Berry boosts pp > maxpp"
    );

    // Turn-Cut safety: mid-turn faint + upkeep must NOT exclude Leftovers or Gold Berry on active opp
    let mut obs_tc = Observer::new(&battle, 0);
    for line in [
        "|switch|p1a: Smeargle|Smeargle, L50, M|161/161",
        "|switch|p2a: Smeargle|Smeargle, L50, M|161/161",
        "|turn|1",
        "|-damage|p2a: Smeargle|50/161",
        "|faint|p1a: Smeargle",
        "|upkeep",
    ] {
        obs_tc.ingest_line(line, &dex);
    }
    assert!(!Observer::is_item_excluded_for_slot(&dex, obs_tc.mons(), 0, dex.items.id("leftovers")));
    assert!(!Observer::is_item_excluded_for_slot(&dex, obs_tc.mons(), 0, dex.items.id("goldberry")));

    // Normal Castle upkeep at exact floor(171/2) = 85/171: Leftovers excluded, Gold Berry NOT excluded
    let mut obs_b = Observer::new(&battle, 0);
    for line in [
        "|switch|p1a: Smeargle|Smeargle, L50, M|171/171",
        "|switch|p2a: Smeargle|Smeargle, L50, M|171/171",
        "|turn|1",
        "|-damage|p2a: Smeargle|85/171",
        "|upkeep",
    ] {
        obs_b.ingest_line(line, &dex);
    }
    assert!(Observer::is_item_excluded_for_slot(&dex, obs_b.mons(), 0, dex.items.id("leftovers")));
    assert!(!Observer::is_item_excluded_for_slot(&dex, obs_b.mons(), 0, dex.items.id("goldberry")));

    // Strict below half (84/171 < 85): Gold Berry IS excluded, Focus Band & King's Rock stay unexcluded
    obs_b.ingest_line("|turn|2", &dex);
    obs_b.ingest_line("|-damage|p2a: Smeargle|84/171", &dex);
    obs_b.ingest_line("|upkeep", &dex);
    assert!(Observer::is_item_excluded_for_slot(&dex, obs_b.mons(), 0, dex.items.id("goldberry")));
    assert!(!Observer::is_item_excluded_for_slot(&dex, obs_b.mons(), 0, dex.items.id("focusband")));
    assert!(!Observer::is_item_excluded_for_slot(&dex, obs_b.mons(), 0, dex.items.id("kingsrock")));
}

#[test]
fn blind_search_castle_hidden_timer_ranges() {
    let dex = load_dex();
    let team = vec![make_set("Snorlax", 50, "Leftovers", &["Body Slam", "Rest", "Curse", "Earthquake"])];

    let sample_ranges = |ruleset: Ruleset| {
        let mut base = Battle::from_fixture_with_ruleset(&dex, "1,2,3,4", &team, &team, ruleset).unwrap();
        let pid = nc2000_engine::state::PokeId { side: 1, slot: 0 };
        base.poke_mut(pid).status = Status::Slp;
        base.poke_mut(pid).last_move = dex.moves.id("bodyslam");
        base.add_volatile(&dex, pid, "confusion", None, nc2000_engine::battle::EffectHandle::None);
        base.add_volatile(&dex, pid, "disable", None, nc2000_engine::battle::EffectHandle::None);
        base.add_volatile(&dex, pid, "bide", None, nc2000_engine::battle::EffectHandle::None);

        let specs = [
            HiddenTimerSpec { side: 1, slot: 0, kind: HiddenTimerKind::Sleep { slept: 0 } },
            HiddenTimerSpec { side: 1, slot: 0, kind: HiddenTimerKind::Confusion { ticks: 0 } },
            HiddenTimerSpec {
                side: 1,
                slot: 0,
                kind: HiddenTimerKind::Disable { upkeeps: 0, acted_on_start: false },
            },
            HiddenTimerSpec { side: 1, slot: 0, kind: HiddenTimerKind::Bide { upkeeps: 0 } },
        ];
        let mut slp = BTreeSet::new();
        let mut conf = BTreeSet::new();
        let mut dis = BTreeSet::new();
        let mut bide = BTreeSet::new();
        let mut rng = SplitMix64::new(12345);
        for _ in 0..400 {
            let mut sim = base.clone();
            resample_hidden_timers(&dex, &mut sim, &specs, &mut rng);
            let p = sim.poke(pid);
            slp.insert(p.status_state.get_int(DK::Time));
            conf.insert(p.volatile(dex.conds_id("confusion").unwrap()).unwrap().get_int(DK::Time));
            dis.insert(p.volatile(dex.conds_id("disable").unwrap()).unwrap().duration.unwrap());
            bide.insert(p.volatile(dex.conds_id("bide").unwrap()).unwrap().duration.unwrap());
        }
        (slp, conf, dis, bide)
    };

    let (ps_slp, ps_conf, ps_dis, ps_bide) = sample_ranges(Ruleset::PsStadium2);
    assert_eq!(ps_slp, BTreeSet::from([2, 3, 4]));
    assert_eq!(ps_conf, BTreeSet::from([2, 3, 4, 5]));
    assert_eq!(ps_dis, BTreeSet::from([2, 3, 4, 5]));
    assert_eq!(ps_bide, BTreeSet::from([3, 4]));

    let (c_slp, c_conf, c_dis, c_bide) = sample_ranges(Ruleset::castle_tournament());
    assert_eq!(c_slp, BTreeSet::from([2, 3, 4, 5]));
    assert_eq!(c_conf, BTreeSet::from([2, 3, 4, 5]));
    assert_eq!(c_dis, BTreeSet::from([1, 2, 3, 4, 5, 6, 7]));
    assert_eq!(c_bide, BTreeSet::from([2, 3]));
}

#[test]
fn import_castle_den_100_exact_hp_and_ruleset_preservation() {
    let dex = load_dex();

    // Pure HP helper checks: Castle exact mode does NOT misread den==100 as percentage mode
    assert_eq!(impute_hp_mode(37, 100, 100, true), 37);
    assert_eq!(impute_hp_mode(49, 100, 200, true), 49);
    assert_eq!(impute_hp_mode(49, 100, 200, false), 98);
    assert_eq!(announce_hp_mode(37, 100, 100, true), 37);
    assert_eq!(announce_hp_mode(99, 266, 266, true), 99);

    // End-to-end ProtocolAgent with Castle ruleset and a real 100-HP opponent (Lv50 Diglett-like exact den==100)
    let own = sample_six(&dex);
    let mut foe = sample_six(&dex);
    // Diglett Lv50 with HP DV=10 (iv=20), HP EV=255 -> baseHP(10) + hpDV(10) + floor(63/2)(31) + 60 = 111;
    // With HP DV=0 (iv=0), HP EV=255 (effort 63 -> +31): 10 + 0 + 31 + 60 = 101; with HP EV=240 (effort 60 -> +30): 100!
    foe[0] = make_set_evs("Diglett", 50, "Focus Band", &["Earthquake", "Rock Slide", "Slash", "Substitute"], 240, 0);
    let check_b = Battle::from_fixture_with_ruleset(&dex, "1,2,3,4", &own, &foe, Ruleset::castle_tournament()).unwrap();
    assert_eq!(check_b.sides[1].roster[0].base_maxhp, 100);

    let pool = MetaPool { teams: vec![MetaTeam { id: "foe".into(), sets: foe.clone(), weight: None }] };
    let mut agent = ProtocolAgent::new_with_ruleset(
        &dex,
        0,
        pool,
        RmConfig::default(),
        7,
        Ruleset::castle_tournament(),
    );
    agent.set_own_team(own.clone());
    assert_eq!(agent.ruleset(), Ruleset::castle_tournament());

    let mut lines = Vec::new();
    for (p, team) in [("p1", &own), ("p2", &foe)] {
        for s in team {
            lines.push(format!("|poke|{p}|{}, L{}, M|item", s.species, s.level));
        }
    }
    lines.extend([
        "|teampreview|3".to_string(),
        "|start".to_string(),
        "|switch|p1a: Snorlax|Snorlax, L55, M|292/292".to_string(),
        "|switch|p2a: Diglett|Diglett, L50, M|100/100".to_string(),
        "|turn|1".to_string(),
        "|-damage|p2a: Diglett|37/100".to_string(),
    ]);
    for l in &lines {
        agent.push_line(&dex, l);
    }

    let req_json = serde_json::json!({
        "active": [{
            "moves": [
                { "move": "Body Slam", "id": "bodyslam", "pp": 24, "maxpp": 24, "disabled": false },
                { "move": "Earthquake", "id": "earthquake", "pp": 16, "maxpp": 16, "disabled": false },
                { "move": "Curse", "id": "curse", "pp": 16, "maxpp": 16, "disabled": false },
                { "move": "Rest", "id": "rest", "pp": 16, "maxpp": 16, "disabled": false }
            ]
        }],
        "side": {
            "name": "P1",
            "id": "p1",
            "pokemon": [
                { "ident": "p1: Snorlax", "details": "Snorlax, L55, M", "condition": "292/292", "active": true, "item": "leftovers" },
                { "ident": "p1: Zapdos", "details": "Zapdos, L50, M", "condition": "197/197", "active": false, "item": "mintberry" },
                { "ident": "p1: Starmie", "details": "Starmie, L50, M", "condition": "167/167", "active": false, "item": "miracleberry" }
            ]
        }
    })
    .to_string();
    assert!(agent.on_request(&dex, &req_json).unwrap());

    let synth = agent.battle().expect("synthesized battle");
    assert_eq!(synth.ruleset, Ruleset::castle_tournament());
    assert_eq!(synth.sides[1].roster[0].base_maxhp, 100);
    assert_eq!(synth.sides[1].roster[0].hp, 37);
    assert_eq!(agent.observer().unwrap().mons()[0].observed_maxhp, Some(100));

    // Castle switch-in faint Turn-Cut (Spikes/switch-in poison KO before moves) omits Residual from queue
    let mut tracker = ProtocolTracker::new_with_ruleset(0, Ruleset::castle_tournament());
    for l in [
        "|poke|p1|Snorlax, L55, M|item",
        "|poke|p1|Zapdos, L50, M|item",
        "|poke|p1|Starmie, L50, M|item",
        "|poke|p2|Snorlax, L55, M|item",
        "|poke|p2|Zapdos, L50, M|item",
        "|poke|p2|Starmie, L50, M|item",
        "|start",
        "|switch|p1a: Snorlax|Snorlax, L55, M|292/292",
        "|switch|p2a: Snorlax|Snorlax, L55, M|292/292",
        "|turn|1",
        "|switch|p1a: Zapdos|Zapdos, L50, M|10/197",
        "|-damage|p1a: Zapdos|0 fnt|[from] Spikes",
        "|faint|p1a: Zapdos",
    ] {
        tracker.push_line(&dex, l);
    }
    let fswitch_req = nc2000_bot::import::Request::parse(
        &dex,
        &serde_json::json!({
            "forceSwitch": [true],
            "side": {
                "name": "P1",
                "id": "p1",
                "pokemon": [
                    { "ident": "p1: Zapdos", "details": "Zapdos, L50, M", "condition": "0 fnt", "active": true, "item": "mintberry" },
                    { "ident": "p1: Snorlax", "details": "Snorlax, L55, M", "condition": "292/292", "active": false, "item": "leftovers" },
                    { "ident": "p1: Starmie", "details": "Starmie, L50, M", "condition": "167/167", "active": false, "item": "miracleberry" }
                ]
            }
        })
        .to_string(),
    )
    .unwrap();
    let three_own = own[..3].to_vec();
    let base_ref = Battle::from_fixture_with_ruleset(
        &dex,
        "1,2,3,4",
        &three_own,
        &three_own,
        Ruleset::castle_tournament(),
    )
    .unwrap();
    let obs = Observer::new(&base_ref, 0);
    let mut rng = SplitMix64::new(9);
    let tc_b = tracker
        .synthesize(&dex, &three_own, &base_ref.sides[1].roster, &obs, &fswitch_req, &mut rng)
        .unwrap();
    assert!(
        !tc_b.queue.iter().any(|a| matches!(a.choice, ActionKind::Residual)),
        "Castle switch-in Turn-Cut faint synthesis must not queue EOT Residual"
    );
}

#[test]
fn teamgen_castle_item_swaps_and_dummy_items() {
    let dex = load_dex();
    let ls_text = include_str!("../../../data/learnsets-gen2.json");
    let pool_text = include_str!("../../../data/meta-pool-v0/meta-pool.json");

    let gen_ps = TeamGen::new(&dex, ls_text, pool_text).unwrap();
    let gen_castle = TeamGen::new(&dex, ls_text, pool_text)
        .unwrap()
        .with_ruleset(Ruleset::castle_tournament());
    assert_eq!(gen_ps.ruleset(), Ruleset::PsStadium2);
    assert_eq!(gen_castle.ruleset(), Ruleset::castle_tournament());

    // Team with Dragonite (Outrage -> Dragon damaging move) and a teammate holding Miracle Berry
    let raw = vec![
        serde_json::json!({
            "species": "Dragonite",
            "level": 55,
            "item": "",
            "moves": ["Outrage", "Thunderbolt", "Fire Blast", "Wing Attack"]
        }),
        serde_json::json!({
            "species": "Snorlax",
            "level": 50,
            "item": "Miracle Berry",
            "moves": ["Body Slam", "Earthquake", "Curse", "Rest"]
        }),
    ];

    let ps_items: BTreeSet<String> = gen_ps
        .non_dominated_items(&dex, &raw, 0, true)
        .into_iter()
        .map(|s| toid(&s))
        .collect();
    assert!(ps_items.contains("dragonfang"));
    assert!(!ps_items.contains("dragonscale"));
    assert!(ps_items.contains("bitterberry"));
    assert!(!ps_items.contains("iceberry"));
    assert!(!ps_items.contains("mail"));
    assert!(!ps_items.contains("fastball"));
    assert!(!ps_items.contains("firestone"));

    let castle_items: BTreeSet<String> = gen_castle
        .non_dominated_items(&dex, &raw, 0, true)
        .into_iter()
        .map(|s| toid(&s))
        .collect();
    assert!(castle_items.contains("dragonscale"), "Castle must offer Dragon Scale for Dragon moves");
    assert!(!castle_items.contains("dragonfang"), "Castle must NOT offer inert Dragon Fang");
    assert!(castle_items.contains("iceberry"), "Castle must offer Ice Berry (ID 81) when Miracle Berry is held");
    assert!(!castle_items.contains("bitterberry"), "Castle must NOT offer inert Bitter Berry (ID 84)");
    assert!(castle_items.contains("mail"), "Castle must permit Mail as dummy Thief-bait item");
    assert!(castle_items.contains("fastball"), "Castle must permit Apricorn balls as dummy items");
    assert!(castle_items.contains("firestone"), "Castle must permit stones as dummy items");

    // Non-Dragon move holder must NOT receive Dragon Scale in Castle
    let castle_lax_items: BTreeSet<String> = gen_castle
        .non_dominated_items(&dex, &raw, 1, true)
        .into_iter()
        .map(|s| toid(&s))
        .collect();
    assert!(!castle_lax_items.contains("dragonscale"));
}

#[test]
fn runner_duel_preview_castle_turn_limit_and_probe_12a_judge() {
    let dex = load_dex();

    assert_eq!(effective_turn_cap(Ruleset::PsStadium2, 500), 500);
    assert_eq!(effective_turn_cap(Ruleset::PsStadium2, 1500), 1000);
    assert_eq!(effective_turn_cap(Ruleset::castle_tournament(), 500), 80);
    assert_eq!(effective_turn_cap(Ruleset::castle_free(), 500), 500);

    let spec = DuelSpec::for_ruleset(10, 1, Ruleset::castle_tournament());
    assert_eq!(spec.max_turns, 80);

    let bake = BakeCfg {
        screen_games: 1,
        refine_games: 1,
        support: 1,
        skuct_iters: 1,
        advisor_iters: 1,
        advisor_runs: 1,
        eps: 0.1,
        max_turns: 500,
        seed: 1,
    };
    assert_eq!(bake.effective_max_turns(Ruleset::castle_tournament()), 80);
    assert_eq!(bake.effective_max_turns(Ruleset::PsStadium2), 500);

    // Probe 12A1/12A2/12A3 exact judge verification
    let p1_team = vec![
        make_set("Blissey", 50, "Leftovers", &["Soft-Boiled"]),
        make_set("Alakazam", 50, "Miracle Berry", &["Psychic"]),
    ];
    let p2_team = vec![
        make_set("Snorlax", 50, "Leftovers", &["Body Slam"]),
        make_set("Snorlax", 50, "Mint Berry", &["Body Slam"]),
    ];
    let mut b = Battle::from_fixture_with_ruleset(
        &dex,
        "1,2,3,4",
        &p1_team,
        &p2_team,
        Ruleset::castle_tournament(),
    )
    .unwrap();
    // 12A1: P1 (91/361 + 126/126 = 1.252) vs P2 (133/266 + 133/266 = 1.000) -> P1Win
    b.sides[0].roster[0].maxhp = 361;
    b.sides[0].roster[0].hp = 91;
    b.sides[0].roster[1].maxhp = 126;
    b.sides[0].roster[1].hp = 126;
    b.sides[1].roster[0].maxhp = 266;
    b.sides[1].roster[0].hp = 133;
    b.sides[1].roster[1].maxhp = 266;
    b.sides[1].roster[1].hp = 133;
    assert_eq!(judge_castle_turn_limit(&b), Outcome::P1Win);

    // 12A2: seat swapped -> P2Win (no seat bias)
    let mut b_swap = Battle::from_fixture_with_ruleset(
        &dex,
        "1,2,3,4",
        &p2_team,
        &p1_team,
        Ruleset::castle_tournament(),
    )
    .unwrap();
    b_swap.sides[0].roster[0].maxhp = 266;
    b_swap.sides[0].roster[0].hp = 133;
    b_swap.sides[0].roster[1].maxhp = 266;
    b_swap.sides[0].roster[1].hp = 133;
    b_swap.sides[1].roster[0].maxhp = 361;
    b_swap.sides[1].roster[0].hp = 91;
    b_swap.sides[1].roster[1].maxhp = 126;
    b_swap.sides[1].roster[1].hp = 126;
    assert_eq!(judge_castle_turn_limit(&b_swap), Outcome::P2Win);

    // 12A3: 2 alive (0.505 total) beats 1 alive (1.000 total)
    b.sides[0].roster[0].hp = 1;
    b.sides[0].roster[1].hp = 63; // 1/361 + 63/126 = 0.5028
    b.sides[1].roster[0].hp = 266;
    b.sides[1].roster[1].hp = 0;
    b.sides[1].roster[1].fainted = true;
    assert_eq!(judge_castle_turn_limit(&b), Outcome::P1Win);
}
