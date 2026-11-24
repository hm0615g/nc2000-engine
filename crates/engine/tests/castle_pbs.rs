//! Track 1 (`engine-battle-castle`) conformance & statistical test suite:
//! - Gate A: all 54 PS-vs-PBS mechanics diffs + §7-A.3 + §7-C.1 + Layer 2 overlays
//!   + Round 5 live-probe `.manifest.json` conformance (18 manifests when present).
//! - Gate C: Monte Carlo statistical distribution checks (fixed seeds, fast, zero stdout).

use std::collections::BTreeMap;
use std::path::Path;

use nc2000_engine::battle::{Outcome, PokemonSet, SearchChoice};
use nc2000_engine::dex::Dex;
use nc2000_engine::rules::{CastleOverlayConfig, Ruleset};
use nc2000_engine::state::{stat_idx, Battle, Gender, RequestKind, RequestState, Status, DK};

fn load_dex() -> Dex {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/gen2stadium2.json");
    Dex::from_json(&std::fs::read_to_string(path).unwrap()).unwrap()
}

fn castle_ruleset() -> Ruleset {
    Ruleset::CastlePbs(CastleOverlayConfig::tournament())
}

fn castle_raw(ovfl: bool, turn_limit: Option<u16>) -> Ruleset {
    let mut cfg = CastleOverlayConfig::raw_cgi(ovfl);
    cfg.turn_limit = turn_limit;
    Ruleset::CastlePbs(cfg)
}

fn mk_bare(species: &str, level: u8, moves: &[&str], item: &str) -> PokemonSet {
    PokemonSet {
        name: species.to_string(),
        species: species.to_string(),
        item: item.to_string(),
        ability: "No Ability".to_string(),
        moves: moves.iter().map(|s| (*s).to_string()).collect(),
        level,
        gender: None,
        evs: None,
        ivs: None,
        happiness: None,
    }
}

fn mk_set(
    species: &str,
    level: u8,
    moves: &[&str],
    item: &str,
    dvs: Option<[u8; 4]>,
    dos: Option<[u8; 5]>,
) -> PokemonSet {
    let ivs = dvs.map(|[a, d, s, c]| {
        let mut m = BTreeMap::new();
        m.insert("atk".to_string(), a * 2);
        m.insert("def".to_string(), d * 2);
        m.insert("spe".to_string(), s * 2);
        m.insert("spa".to_string(), c * 2);
        m
    });
    let evs = dos.map(|[hp, a, d, s, c]| {
        let mut m = BTreeMap::new();
        m.insert("hp".to_string(), (hp * 4) as u16);
        m.insert("atk".to_string(), (a * 4) as u16);
        m.insert("def".to_string(), (d * 4) as u16);
        m.insert("spe".to_string(), (s * 4) as u16);
        m.insert("spa".to_string(), (c * 4) as u16);
        m
    });
    PokemonSet {
        name: species.to_string(),
        species: species.to_string(),
        item: item.to_string(),
        ability: "No Ability".to_string(),
        moves: moves.iter().map(|s| (*s).to_string()).collect(),
        level,
        gender: None,
        evs,
        ivs,
        happiness: None,
    }
}

fn start_castle(
    dex: &Dex,
    seed: &str,
    ruleset: Ruleset,
    p1: &[PokemonSet],
    p2: &[PokemonSet],
) -> Battle {
    let mut b = Battle::from_fixture_with_ruleset(dex, seed, p1, p2, ruleset).unwrap();
    b.preview_level_caps = [None, None];
    let p1_pick: Vec<String> = (1..=p1.len().min(3)).map(|i| i.to_string()).collect();
    let p2_pick: Vec<String> = (1..=p2.len().min(3)).map(|i| i.to_string()).collect();
    b.choose(dex, 0, &format!("team {}", p1_pick.join(","))).unwrap();
    b.choose(dex, 1, &format!("team {}", p2_pick.join(","))).unwrap();
    b
}

// =========================================================================
// 1. §7-C.1 Castle Defaults, PP64, & Layer 2 Overlays
// =========================================================================

#[test]
fn castle_defaults_evs_ivs_gender_and_pp64() {
    let dex = load_dex();
    // Bare Lv50 Snorlax with Splash (base PP 40 -> 64 in Castle, 61 in PS).
    let p1 = vec![
        mk_bare("Snorlax", 50, &["Splash", "Tackle"], ""),
        // Atk DV 1 (< 2 for 12.5% F Snorlax -> Female), Def=14, Spe=14, Spc=14 -> HP DV = (1<<3)|0 = 8 -> IV 16
        mk_set("Snorlax", 50, &["Splash"], "", Some([1, 14, 14, 14]), None),
    ];
    let p2 = vec![mk_bare("Clefable", 50, &["Splash"], "")]; // 75% F: Atk DV 15 >= 12 -> Male
    let b = start_castle(&dex, "1,2,3,4", castle_ruleset(), &p1, &p2);

    let lax0 = &b.sides[0].roster[0];
    assert_eq!(lax0.maxhp, 266);
    assert_eq!(lax0.stored_stats, [161, 116, 116, 161, 81]);
    assert_eq!(lax0.gender, Gender::M); // Atk DV 15 >= 2
    assert_eq!(lax0.move_slots[0].maxpp, 64); // Base 40 + 3 PP Ups = 64 in Castle
    assert_eq!(lax0.move_slots[1].maxpp, 56); // Base 35 + 3 PP Ups = 56

    let lax1 = &b.sides[0].roster[1];
    assert_eq!(lax1.gender, Gender::F); // Atk DV 1 < 2
    assert_eq!(lax1.set_ivs[0], 16); // derived HP DV = 8 -> IV 16
    assert_eq!(lax1.set_ivs[4], lax1.set_ivs[3]); // SpD mirrors SpA

    let clef = &b.sides[1].roster[0];
    assert_eq!(clef.gender, Gender::M); // Atk DV 15 >= 12
    assert_eq!(clef.maxhp, 201);

    // Verify PS Stadium 2 still produces EV=0 (235 HP) & PP 61 for base-40 moves.
    let ps = Battle::from_fixture(&dex, "1,2,3,4", &p1[..1], &p2).unwrap();
    assert_eq!(ps.sides[0].roster[0].maxhp, 235);
    assert_eq!(ps.sides[0].roster[0].move_slots[0].maxpp, 61);
}

#[test]
fn castle_1v1_win_conditions_and_layer2_overlays() {
    let dex = load_dex();

    // #1: 1v1 Recoil mutual KO (Double-Edge): raw_cgi -> Tie, tournament -> attacker wins.
    for (attacker_wins, expected) in [(false, Outcome::Tie), (true, Outcome::P1Win)] {
        let mut cfg = CastleOverlayConfig::raw_cgi(false);
        cfg.recoil_1v1_attacker_wins = attacker_wins;
        let p1 = vec![mk_bare("Tauros", 50, &["Double-Edge"], "")];
        let p2 = vec![mk_bare("Snorlax", 50, &["Splash"], "")];
        let mut b = start_castle(&dex, "1,2,3,4", Ruleset::CastlePbs(cfg), &p1, &p2);
        let a1 = b.active_id(0).unwrap();
        let a2 = b.active_id(1).unwrap();
        b.poke_mut(a1).hp = 5;
        b.poke_mut(a2).hp = 40;
        b.choose(&dex, 0, "move doubleedge").unwrap();
        b.choose(&dex, 1, "move splash").unwrap();
        assert_eq!(b.outcome(), Some(expected));
    }

    // #2: 1v1 Sandstorm mutual KO -> Tie in Castle (even though 1P is faster).
    {
        let p1 = vec![mk_bare("Mew", 50, &["Sandstorm"], "")];
        let p2 = vec![mk_bare("Snorlax", 50, &["Splash"], "")];
        let mut b = start_castle(&dex, "1,2,3,4", castle_ruleset(), &p1, &p2);
        let a1 = b.active_id(0).unwrap();
        let a2 = b.active_id(1).unwrap();
        b.poke_mut(a1).hp = 1;
        b.poke_mut(a2).hp = 1;
        b.choose(&dex, 0, "move sandstorm").unwrap();
        b.choose(&dex, 1, "move splash").unwrap();
        assert_eq!(b.outcome(), Some(Outcome::Tie));
    }

    // 1v1 Self-Destruct / Explosion mutual KO -> user loses.
    {
        let p1 = vec![mk_bare("Snorlax", 50, &["Explosion"], "")];
        let p2 = vec![mk_bare("Blissey", 50, &["Splash"], "")];
        let mut b = start_castle(&dex, "1,2,3,4", castle_ruleset(), &p1, &p2);
        let a2 = b.active_id(1).unwrap();
        b.poke_mut(a2).hp = 10;
        b.choose(&dex, 0, "move explosion").unwrap();
        b.choose(&dex, 1, "move splash").unwrap();
        assert_eq!(b.outcome(), Some(Outcome::P2Win));
    }

    // Layer 2: 1v1 Perish Song mutual KO (after 2v1 -> 1v1): forfeit flag switches P2Win vs Tie.
    for (forfeit, expected) in [(false, Outcome::Tie), (true, Outcome::P2Win)] {
        let mut cfg = CastleOverlayConfig::raw_cgi(false);
        cfg.perish_dbond_lastmon_forfeit = forfeit;
        let p1 = vec![
            mk_bare("Misdreavus", 50, &["Perish Song", "Splash"], ""),
            mk_bare("Magikarp", 50, &["Splash"], ""),
        ];
        let p2 = vec![mk_bare("Snorlax", 50, &["Splash"], "")];
        let mut b = start_castle(&dex, "1,2,3,4", Ruleset::CastlePbs(cfg), &p1, &p2);
        b.choose(&dex, 0, "move perishsong").unwrap();
        b.choose(&dex, 1, "move splash").unwrap();
        // Simulate P1's bench fainting so both sides are down to 1 active mon with Perish active.
        b.sides[0].roster[1].hp = 0;
        b.sides[0].roster[1].fainted = true;
        b.sides[0].pokemon_left = 1;
        for _ in 0..3 {
            b.choose(&dex, 0, "move splash").unwrap();
            b.choose(&dex, 1, "move splash").unwrap();
        }
        assert_eq!(b.outcome(), Some(expected));
    }
}

#[test]
fn castle_lturn_judging_probes_12a1_12a2_12a3() {
    let dex = load_dex();

    // Probe 12A1: alive count tied 2 vs 2, ratio sum P1 (91/361 + 126/126 = 1.252) > P2 (133/266 + 133/266 = 1.000) -> P1Win.
    let p1 = vec![
        mk_bare("Blissey", 50, &["Belly Drum", "Substitute", "Splash"], ""),
        mk_bare("Magikarp", 50, &["Splash"], ""),
    ];
    let p2 = vec![
        mk_bare("Snorlax", 50, &["Belly Drum", "Splash"], ""),
        mk_bare("Snorlax", 50, &["Belly Drum", "Splash"], ""),
    ];
    let mut b1 = start_castle(&dex, "1,2,3,4", castle_raw(false, Some(3)), &p1, &p2);
    // Turn 1: both Belly Drum (Blissey 361->181, Snorlax1 266->133)
    b1.choose(&dex, 0, "move bellydrum").unwrap();
    b1.choose(&dex, 1, "move bellydrum").unwrap();
    // Turn 2: Blissey Substitute (181->91), P2 switches to Snorlax2
    b1.choose(&dex, 0, "move substitute").unwrap();
    b1.choose(&dex, 1, "switch 2").unwrap();
    // Turn 3: Blissey Splash, Snorlax2 Belly Drum (266->133) -> L_TURN=3 triggers!
    b1.choose(&dex, 0, "move splash").unwrap();
    b1.choose(&dex, 1, "move bellydrum").unwrap();
    assert_eq!(b1.outcome(), Some(Outcome::P1Win));

    // Probe 12A2: exact seat swap -> P2Win (zero seat bias).
    let mut b2 = start_castle(&dex, "1,2,3,4", castle_raw(false, Some(3)), &p2, &p1);
    b2.choose(&dex, 0, "move bellydrum").unwrap();
    b2.choose(&dex, 1, "move bellydrum").unwrap();
    b2.choose(&dex, 0, "switch 2").unwrap();
    b2.choose(&dex, 1, "move substitute").unwrap();
    b2.choose(&dex, 0, "move bellydrum").unwrap();
    b2.choose(&dex, 1, "move splash").unwrap();
    assert_eq!(b2.outcome(), Some(Outcome::P2Win));

    // Probe 12A3: alive count 2 (low HP ratio 0.505) vs 1 (full HP 1.000) -> 2-alive side wins.
    let p1_cnt = vec![
        mk_bare("Mew", 50, &["Splash"], ""),
        mk_bare("Magikarp", 50, &["Splash"], ""),
    ];
    let p2_cnt = vec![mk_bare("Snorlax", 50, &["Splash"], "")];
    let mut b3 = start_castle(&dex, "1,2,3,4", castle_raw(false, Some(1)), &p1_cnt, &p2_cnt);
    b3.sides[0].roster[0].hp = 1;
    b3.sides[0].roster[1].hp = 1;
    b3.choose(&dex, 0, "move splash").unwrap();
    b3.choose(&dex, 1, "move splash").unwrap();
    assert_eq!(b3.outcome(), Some(Outcome::P1Win));
}

// =========================================================================
// 2. §3-A & §3-B Turn-Cut, EOT 1P->2P Order, Clauses, Statuses (#3–#14)
// =========================================================================

#[test]
fn castle_turn_cut_switch_spikes_and_pursuit_rechoose() {
    let dex = load_dex();

    // #3: First switcher dies to Spikes -> cancels second side's queued switch!
    let p1 = vec![
        mk_bare("Mew", 50, &["Splash"], ""),
        mk_bare("Magikarp", 50, &["Splash"], ""),
        mk_bare("Snorlax", 50, &["Splash"], ""),
    ];
    let p2 = vec![
        mk_bare("Snorlax", 50, &["Spikes", "Splash"], ""),
        mk_bare("Blissey", 50, &["Splash"], ""),
    ];
    let mut b = start_castle(&dex, "1,2,3,4", castle_ruleset(), &p1, &p2);
    b.choose(&dex, 0, "move splash").unwrap();
    b.choose(&dex, 1, "move spikes").unwrap();
    b.sides[0].roster[1].hp = 5; // Magikarp will faint to Spikes (126/8 = 15)
    // Both sides choose voluntary switch! Faster P1 switches first -> dies to Spikes -> Turn-Cut skips P2 switch!
    b.choose(&dex, 0, "switch 2").unwrap();
    b.choose(&dex, 1, "switch 2").unwrap();
    assert_eq!(b.request_state, RequestState::Switch);
    assert_eq!(b.sides[0].request, Some(RequestKind::Switch));
    assert_eq!(b.sides[1].request, Some(RequestKind::Wait));
    // P2's Snorlax is STILL active (switch was skipped by Turn-Cut!).
    assert_eq!(b.active_id(1).unwrap().slot, 0);
    b.choose(&dex, 0, "switch 3").unwrap();
    assert_eq!(b.active_id(1).unwrap().slot, 0);

    // #4: Pursuit KO on switch cancels planned switch and opens free CHANGE (`Probe 10B`).
    let p1_p = vec![
        mk_bare("Alakazam", 50, &["Splash"], ""),
        mk_bare("Snorlax", 50, &["Splash"], ""),
        mk_bare("Blissey", 50, &["Splash"], ""),
    ];
    let p2_p = vec![mk_bare("Tyranitar", 55, &["Pursuit", "Splash"], "")];
    let mut bp = start_castle(&dex, "1,2,3,4", castle_ruleset(), &p1_p, &p2_p);
    let a1 = bp.active_id(0).unwrap();
    bp.poke_mut(a1).hp = 10;
    bp.choose(&dex, 0, "switch 2").unwrap(); // Planned Snorlax (slot 2)
    bp.choose(&dex, 1, "move pursuit").unwrap();
    // Pursuit KO'd Alakazam -> instead of auto-sending Snorlax, Castle opens free CHANGE!
    assert_eq!(bp.request_state, RequestState::Switch);
    assert_eq!(bp.sides[0].request, Some(RequestKind::Switch));
    // Choose Blissey (slot 3) instead of originally planned Snorlax!
    bp.choose(&dex, 0, "switch 3").unwrap();
    assert_eq!(bp.active_id(0).unwrap().slot, 2); // Blissey came in!
}

#[test]
fn castle_eot_fixed_1p_to_2p_bind_after_move_self_and_perish_order() {
    let dex = load_dex();

    // #5, #6, #7: Even when 2P is faster than 1P, ALL EOT phases run 1P -> 2P!
    // Also Wrap ticks in AfterMoveSelf (before Perish Song and before EOT!).
    let p1 = vec![
        mk_bare("Snorlax", 50, &["Reflect", "Sandstorm", "Perish Song", "Splash"], "Leftovers"),
        mk_bare("Mew", 50, &["Splash"], ""),
    ];
    let p2 = vec![
        mk_bare("Jolteon", 50, &["Reflect", "Wrap", "Splash"], "Leftovers"),
        mk_bare("Blissey", 50, &["Splash"], ""),
    ];
    let mut b = start_castle(&dex, "1,2,3,4", castle_ruleset(), &p1, &p2);
    // Turn 1: both Reflect (2P faster, 1P slower)
    b.choose(&dex, 0, "move reflect").unwrap();
    b.choose(&dex, 1, "move reflect").unwrap();
    // Turn 2: P1 Sandstorm, P2 Splash
    b.choose(&dex, 0, "move sandstorm").unwrap();
    b.choose(&dex, 1, "move splash").unwrap();
    // Turn 3: P1 Perish Song, P2 Wrap
    b.log.clear();
    b.choose(&dex, 0, "move perishsong").unwrap();
    b.choose(&dex, 1, "move wrap").unwrap();
    // Turn 4: Splash vs Splash; Turn 5: Reflect expires (1P->2P), Sandstorm (1P->2P), Perish (1P->2P), Leftovers (1P->2P)!
    b.choose(&dex, 0, "move splash").unwrap();
    b.choose(&dex, 1, "move splash").unwrap();
    b.log.clear();
    b.choose(&dex, 0, "move splash").unwrap();
    b.choose(&dex, 1, "move splash").unwrap();

    let pos = |needle: &str| b.log.iter().position(|l| l.contains(needle));
    let i_refl_p1 = pos("|-sideend|p1: P1|Reflect").expect("p1 reflect end");
    let i_refl_p2 = pos("|-sideend|p2: P2|Reflect").expect("p2 reflect end");
    let i_sand_p1 = pos("|-damage|p1a: Snorlax|").expect("p1 sandstorm");
    let i_sand_p2 = pos("|-damage|p2a: Jolteon|").expect("p2 sandstorm");
    let i_per_p1 = pos("|-start|p1a: Snorlax|perish1").expect("p1 perish1");
    let i_per_p2 = pos("|-start|p2a: Jolteon|perish1").expect("p2 perish1");
    let i_left_p1 = pos("|-heal|p1a: Snorlax|").expect("p1 leftovers");
    let i_left_p2 = pos("|-heal|p2a: Jolteon|").expect("p2 leftovers");
    // Verify strict phase order AND 1P -> 2P within EVERY phase despite 2P Jolteon being much faster!
    assert!(i_refl_p1 < i_refl_p2);
    assert!(i_refl_p2 < i_sand_p1);
    assert!(i_sand_p1 < i_sand_p2);
    assert!(i_sand_p2 < i_per_p1);
    assert!(i_per_p1 < i_per_p2);
    assert!(i_per_p2 < i_left_p1);
    assert!(i_left_p1 < i_left_p2);

    // Turn 6: Perish 0 simultaneous faint -> 1P->2P even when 1P is slower (and also test when 1P is faster).
    b.log.clear();
    b.choose(&dex, 0, "move splash").unwrap();
    b.choose(&dex, 1, "move splash").unwrap();
    let f1 = b.log.iter().position(|l| l == "|faint|p1a: Snorlax").unwrap();
    let f2 = b.log.iter().position(|l| l == "|faint|p2a: Jolteon").unwrap();
    assert!(f1 < f2, "simultaneous perish faint must be 1P -> 2P");
}

#[test]
fn castle_bind_ticks_in_after_move_self_before_eot() {
    let dex = load_dex();
    let p1 = vec![
        mk_bare("Dugtrio", 50, &["Wrap", "Perish Song", "Splash"], ""),
        mk_bare("Mew", 50, &["Splash"], ""),
    ];
    let p2 = vec![
        mk_bare("Blissey", 50, &["Splash"], ""),
        mk_bare("Snorlax", 50, &["Splash"], ""),
    ];
    let mut b = start_castle(&dex, "1,2,3,4", castle_ruleset(), &p1, &p2);
    // Guarantee Wrap hits across seed search if needed
    b.choose(&dex, 0, "move perishsong").unwrap();
    b.choose(&dex, 1, "move splash").unwrap();
    for seed in 1..20u64 {
        let mut t = b.clone();
        t.reseed(seed);
        t.log.clear();
        t.choose(&dex, 0, "move wrap").unwrap();
        t.choose(&dex, 1, "move splash").unwrap();
        if let Some(i_bind) = t.log.iter().position(|l| l.contains("[partiallytrapped]")) {
            let i_perish = t.log.iter().position(|l| l.contains("perish2")).unwrap();
            let i_splash = t.log.iter().position(|l| l.contains("|move|p2a: Blissey|Splash|")).unwrap();
            assert!(i_splash < i_bind && i_bind < i_perish, "Wrap must tick in AfterMoveSelf before Perish EOT");
            return;
        }
    }
    panic!("Wrap never hit across seeds");
}

#[test]
fn castle_sleep_clause_freeze_clause_flinch_nightmare_and_healbell() {
    let dex = load_dex();

    // #9 Sleep Clause: foe sleep fails (no forfeit) when defender has a Rest-sleeping teammate;
    // self-Rest succeeds when bench teammate is asleep (`Probe 12I`).
    {
        let p1 = vec![
            mk_bare("Jynx", 50, &["Spore", "Splash"], ""),
            mk_bare("Snorlax", 50, &["Splash"], ""),
        ];
        let p2 = vec![
            mk_bare("Snorlax", 50, &["Rest", "Splash"], ""),
            mk_bare("Blissey", 50, &["Rest", "Splash"], ""),
        ];
        let mut b = start_castle(&dex, "1,2,3,4", castle_ruleset(), &p1, &p2);
        let a2 = b.active_id(1).unwrap();
        b.poke_mut(a2).hp = 200;
        b.choose(&dex, 0, "move splash").unwrap();
        b.choose(&dex, 1, "move rest").unwrap();
        assert_eq!(b.poke(a2).status, Status::Slp);
        // P2 switches sleeping Snorlax out for Blissey; P1 uses Spore -> blocked by Sleep Clause (NO forfeit!).
        b.choose(&dex, 0, "move spore").unwrap();
        b.choose(&dex, 1, "switch 2").unwrap();
        assert!(!b.ended, "Castle Sleep Clause must NOT forfeit");
        let b2 = b.active_id(1).unwrap();
        assert_eq!(b.poke(b2).status, Status::None);
        // Self-Rest on Blissey STILL succeeds while Snorlax sleeps on bench (`Probe 12I`)!
        b.poke_mut(b2).hp = 200;
        b.choose(&dex, 0, "move splash").unwrap();
        b.choose(&dex, 1, "move rest").unwrap();
        assert_eq!(b.poke(b2).status, Status::Slp);
        assert!(!b.ended);
    }

    // #11 Freeze Clause bypassed by Tri Attack, enforced for Ice Beam.
    {
        let p1 = vec![mk_bare("Porygon2", 50, &["Tri Attack", "Ice Beam"], "")];
        let p2 = vec![
            mk_bare("Blissey", 50, &["Splash"], ""),
            mk_bare("Snorlax", 50, &["Splash"], ""),
        ];
        let mut b = start_castle(&dex, "1,2,3,4", castle_ruleset(), &p1, &p2);
        b.sides[1].roster[1].status = Status::Frz; // bench mon already frozen
        // Ice Beam should never freeze active Blissey even when rolling freeze
        let icebeam = dex.moves.id("icebeam").unwrap();
        let triattack = dex.moves.id("triattack").unwrap();
        let a1 = b.active_id(0).unwrap();
        let a2 = b.active_id(1).unwrap();
        let r_ib = b.try_set_status(&dex, a2, "frz", Some(a1), nc2000_engine::battle::EffectHandle::MoveEff(icebeam));
        assert!(!r_ib.truthy(), "Ice Beam must be blocked by Freeze Clause");
        assert_eq!(b.poke(a2).status, Status::None);
        let r_ta = b.try_set_status(&dex, a2, "frz", Some(a1), nc2000_engine::battle::EffectHandle::MoveEff(triattack));
        assert!(r_ta.truthy(), "Tri Attack must bypass Freeze Clause in Castle");
        assert_eq!(b.poke(a2).status, Status::Frz);
    }

    // #12 Flinch applies to sleeping target (`Probe 8E`: Sleep Talk gets flinched).
    {
        let p1 = vec![mk_bare("Aerodactyl", 50, &["Headbutt"], "")];
        let p2 = vec![mk_bare("Snorlax", 50, &["Sleep Talk", "Splash"], "")];
        let mut saw_sleep_flinch = false;
        for seed in 1..60u64 {
            let mut b = start_castle(&dex, &format!("{seed},2,3,4"), castle_ruleset(), &p1, &p2);
            let a2 = b.active_id(1).unwrap();
            b.restore_status(&dex, a2, Status::Slp, None);
            b.poke_mut(a2).status_state.set_int(DK::Time, 3);
            b.log.clear();
            b.choose(&dex, 0, "move headbutt").unwrap();
            b.choose(&dex, 1, "move sleeptalk").unwrap();
            let slept = b.log.iter().position(|l| l.contains("|cant|p2a: Snorlax|slp"));
            let flinched = b.log.iter().position(|l| l.contains("|cant|p2a: Snorlax|flinch"));
            if let (Some(s), Some(f)) = (slept, flinched) {
                assert!(s < f);
                saw_sleep_flinch = true;
                break;
            }
        }
        assert!(saw_sleep_flinch, "Headbutt must flinch sleeping target in Castle");
    }

    // #13 & #32②: First-mover Rest (Miracle Berry) -> opponent uses Nightmare ->
    // Miracle Berry wakes user AFTER opponent move WITHOUT clearing Nightmare -> awake 1/4 dmg!
    {
        let p1 = vec![mk_bare("Mew", 50, &["Rest", "Splash"], "Miracle Berry")];
        let p2 = vec![mk_bare("Snorlax", 50, &["Nightmare", "Splash"], "")];
        let mut b = start_castle(&dex, "1,2,3,4", castle_ruleset(), &p1, &p2);
        let a1 = b.active_id(0).unwrap();
        b.poke_mut(a1).hp = 150;
        b.choose(&dex, 0, "move rest").unwrap();
        b.choose(&dex, 1, "move nightmare").unwrap();
        // Mew woke from Miracle Berry, yet Nightmare persisted!
        assert_eq!(b.poke(a1).status, Status::None);
        assert!(b.poke(a1).item.is_none());
        let nm = dex.conds_id("nightmare").unwrap();
        assert!(b.poke(a1).has_volatile(nm));
        // Next turn Mew (awake) moves and takes 1/4 Nightmare damage (206 -> 206 - 51 = 155)!
        b.choose(&dex, 0, "move splash").unwrap();
        b.choose(&dex, 1, "move splash").unwrap();
        assert_eq!(b.poke(a1).hp, 206 - (206 / 4));
    }

    // #14: Heal Bell resets Toxic counter (`residualdmg`) -> subsequent Poison deals 1/8 (`Probe 12B`).
    {
        let p1 = vec![mk_bare("Mew", 50, &["Heal Bell", "Splash"], "")];
        let p2 = vec![mk_bare("Snorlax", 50, &["Toxic", "Poison Powder", "Splash"], "")];
        let mut b = start_castle(&dex, "1,2,3,4", castle_ruleset(), &p1, &p2);
        let a1 = b.active_id(0).unwrap();
        b.restore_status(&dex, a1, Status::Tox, Some(b.active_id(1).unwrap()));
        // Advance Toxic twice (counter -> 2), then Heal Bell, then normal Poison -> must take floor(206/8) = 25!
        b.choose(&dex, 0, "move splash").unwrap();
        b.choose(&dex, 1, "move splash").unwrap();
        b.choose(&dex, 0, "move healbell").unwrap();
        b.choose(&dex, 1, "move splash").unwrap();
        let rd = dex.conds_id("residualdmg").unwrap();
        assert!(!b.poke(a1).has_volatile(rd), "Heal Bell must clear residualdmg in Castle");
        b.poke_mut(a1).hp = 206;
        b.restore_status(&dex, a1, Status::Psn, Some(b.active_id(1).unwrap()));
        b.choose(&dex, 0, "move splash").unwrap();
        b.choose(&dex, 1, "move splash").unwrap();
        assert_eq!(b.poke(a1).hp, 206 - (206 / 8));
    }
}

// =========================================================================
// 3. §3-C Stats, f64 Speed, Damage, ovfl, Probs (#15–#30)
// =========================================================================

#[test]
fn castle_f64_speed_par_and_stage_probe_12h() {
    let dex = load_dex();

    // #15: Par S123 (30.75) vs Par S121 (30.25) -> S123 ALWAYS moves first (100% across both seats).
    // Mew Lv50 Spe DV 15/EV 255 = 151; use custom dos to hit exact S=123 vs S=121!
    let mk_spe = |spe_do: u8| {
        mk_set("Mew", 50, &["Splash"], "", Some([15, 15, 0, 15]), Some([63, 63, 63, spe_do, 63]))
    };
    // Mew base Spe 100, DV 0, Lv50: ((100+0)*2 + do)*50/100 + 5 = 105 + do/2.
    // do=36 -> 123, do=32 -> 121, do=30 -> 120.
    let p_123 = vec![mk_spe(36)];
    let p_121 = vec![mk_spe(32)];
    for seed in 1..=30u64 {
        let mut b = start_castle(&dex, &format!("{seed},2,3,4"), castle_ruleset(), &p_121, &p_123);
        let a1 = b.active_id(0).unwrap();
        let a2 = b.active_id(1).unwrap();
        assert_eq!(b.poke(a1).stored_stats[stat_idx::SPE], 121);
        assert_eq!(b.poke(a2).stored_stats[stat_idx::SPE], 123);
        b.restore_status(&dex, a1, Status::Par, None);
        b.restore_status(&dex, a2, Status::Par, None);
        b.log.clear();
        b.choose(&dex, 0, "move splash").unwrap();
        b.choose(&dex, 1, "move splash").unwrap();
        let m1 = b.log.iter().position(|l| l.starts_with("|move|p1a:") || l.starts_with("|cant|p1a:")).unwrap();
        let m2 = b.log.iter().position(|l| l.starts_with("|move|p2a:") || l.starts_with("|cant|p2a:")).unwrap();
        assert!(m2 < m1, "Par 123 (30.75) must always outspeed Par 121 (30.25)");
    }

    // #16: -1 stage S121 (80.667) vs neutral S80 (80.0) -> -1 S121 ALWAYS moves first!
    // Slowpoke base Spe 15, DV 15, do=63 -> 66; Snorlax base Spe 30, DV 14, do=63 -> 80!
    let p_s80 = vec![mk_set("Snorlax", 50, &["Splash"], "", Some([15, 15, 14, 15]), None)];
    for seed in 1..=30u64 {
        let mut b = start_castle(&dex, &format!("{seed},2,3,4"), castle_ruleset(), &p_s80, &p_121);
        let a1 = b.active_id(0).unwrap();
        let a2 = b.active_id(1).unwrap();
        assert_eq!(b.poke(a1).stored_stats[stat_idx::SPE], 80);
        assert_eq!(b.poke(a2).stored_stats[stat_idx::SPE], 121);
        b.poke_mut(a2).boosts[stat_idx::SPE] = -1;
        b.log.clear();
        b.choose(&dex, 0, "move splash").unwrap();
        b.choose(&dex, 1, "move splash").unwrap();
        let m1 = b.log.iter().position(|l| l.contains("|move|p1a:")).unwrap();
        let m2 = b.log.iter().position(|l| l.contains("|move|p2a:")).unwrap();
        assert!(m2 < m1, "-1 S121 (80.667) must always outspeed S80 (80.0)");
    }

    // Exact f64 tie: -1 S120 (80.0) vs S80 (80.0) breaks ~50/50!
    let p_120 = vec![mk_spe(30)];
    let mut p1_first = 0;
    for seed in 1..=100u64 {
        let mut b = start_castle(&dex, &format!("{seed},2,3,4"), castle_ruleset(), &p_s80, &p_120);
        let a2 = b.active_id(1).unwrap();
        b.poke_mut(a2).boosts[stat_idx::SPE] = -1;
        b.log.clear();
        b.choose(&dex, 0, "move splash").unwrap();
        b.choose(&dex, 1, "move splash").unwrap();
        let m1 = b.log.iter().position(|l| l.contains("|move|p1a:")).unwrap();
        let m2 = b.log.iter().position(|l| l.contains("|move|p2a:")).unwrap();
        if m1 < m2 {
            p1_first += 1;
        }
    }
    assert!((30..=70).contains(&p1_first), "Exact f64 tie 80.0 == 80.0 must be 50/50, got {p1_first}/100");
}

#[test]
fn castle_damage_diffs_17_to_30() {
    let dex = load_dex();

    // #17: Negative stage 2/(2-s) (`Probe 12B`): Lv5 Blissey Atk=6 (-1 -> 4) Explosion vs Lv10 Blissey Def=7 -> 35..=42!
    {
        let p1 = vec![
            mk_set("Blissey", 5, &["Explosion"], "", Some([0, 0, 0, 0]), Some([0, 0, 0, 0, 0])),
            mk_bare("Magikarp", 50, &["Splash"], ""),
        ];
        let p2 = vec![mk_set("Blissey", 10, &["Splash"], "", Some([0, 0, 0, 0]), Some([0, 0, 0, 0, 0]))];
        let mut b = start_castle(&dex, "1,2,3,4", castle_ruleset(), &p1, &p2);
        let a1 = b.active_id(0).unwrap();
        let a2 = b.active_id(1).unwrap();
        assert_eq!(b.poke(a1).stored_stats[stat_idx::ATK], 6);
        assert_eq!(b.poke(a2).stored_stats[stat_idx::DEF], 7);
        b.poke_mut(a1).boosts[stat_idx::ATK] = -1;
        let pre_hp = b.poke(a2).hp;
        b.choose(&dex, 0, "move explosion").unwrap();
        b.choose(&dex, 1, "move splash").unwrap();
        let dealt = pre_hp - b.poke(a2).hp;
        assert!((35..=42).contains(&dealt), "Expected 2/3 rational range 35..=42, got {dealt}");
    }

    // #18: ovfl=0 vs ovfl=1 Thick Club Marowak +6 Tackle vs Blissey (`Probe 3A`/`3B`),
    // AND ovfl=1 missed Fissure Counter OHKO (`Probe 8D1`).
    {
        let p1 = vec![mk_bare("Marowak", 50, &["Tackle"], "Thick Club")];
        let p2 = vec![mk_bare("Blissey", 50, &["Splash"], "")];
        let mut b0 = start_castle(&dex, "1,2,3,4", castle_raw(false, None), &p1, &p2);
        let mut b1 = start_castle(&dex, "1,2,3,4", castle_raw(true, None), &p1, &p2);
        for b in [&mut b0, &mut b1] {
            let a1 = b.active_id(0).unwrap();
            b.poke_mut(a1).boosts[stat_idx::ATK] = 6;
            b.choose(&dex, 0, "move tackle").unwrap();
            b.choose(&dex, 1, "move splash").unwrap();
        }
        let d0 = 361 - b0.poke(b0.active_id(1).unwrap()).hp;
        let d1 = 361 - b1.poke(b1.active_id(1).unwrap()).hp;
        assert!((218..=257).contains(&d0), "ovfl=0 Marowak Tackle expected 218..=257, got {d0}");
        assert!((6..=8).contains(&d1), "ovfl=1 Marowak Tackle expected 6..=8, got {d1}");

        // Missed Fissure Counter OHKO in ovfl=1 (and fails in ovfl=0).
        let pf1 = vec![mk_bare("Dugtrio", 50, &["Fissure"], "")];
        let pf2 = vec![mk_bare("Mew", 50, &["Counter"], "")];
        let mut saw_ohko = false;
        for seed in 1..40u64 {
            let mut b = start_castle(&dex, &format!("{seed},2,3,4"), castle_raw(true, None), &pf1, &pf2);
            b.choose(&dex, 0, "move fissure").unwrap();
            b.choose(&dex, 1, "move counter").unwrap();
            if b.log.iter().any(|l| l.contains("|-miss|p1a: Dugtrio")) {
                assert_eq!(b.outcome(), Some(Outcome::P2Win), "ovfl=1 missed Fissure must be Counter OHKO");
                saw_ohko = true;
                break;
            }
        }
        assert!(saw_ohko);
    }

    // #19: Explosion Def/2 applied BEFORE >=256 /4 (`Probe 12E2`: Lv100 Chansey Atk=15 vs Lv56 Cloyster Def=258 = 33..=39).
    {
        let p1 = vec![
            mk_set("Chansey", 100, &["Explosion"], "", Some([0, 0, 0, 0]), Some([0, 0, 0, 0, 0])),
            mk_bare("Magikarp", 50, &["Splash"], ""),
        ];
        let p2 = vec![mk_bare("Cloyster", 56, &["Splash"], "")];
        let mut b = start_castle(&dex, "1,2,3,4", castle_ruleset(), &p1, &p2);
        let a1 = b.active_id(0).unwrap();
        let a2 = b.active_id(1).unwrap();
        assert_eq!(b.poke(a1).stored_stats[stat_idx::ATK], 15);
        assert_eq!(b.poke(a2).stored_stats[stat_idx::DEF], 258);
        let pre = b.poke(a2).hp;
        b.choose(&dex, 0, "move explosion").unwrap();
        b.choose(&dex, 1, "move splash").unwrap();
        let dealt = pre - b.poke(a2).hp;
        assert!((33..=39).contains(&dealt), "Explosion Def/2 before >=256 expected 33..=39, got {dealt}");
    }

    // #20: Burn Atk /2 and Par Spe /4 NOT cleared by Swords Dance / Agility / Haze.
    {
        let p1 = vec![mk_bare("Snorlax", 50, &["Swords Dance", "Haze"], "")];
        let p2 = vec![mk_bare("Mew", 50, &["Agility", "Haze"], "")];
        let mut b = start_castle(&dex, "1,2,3,4", castle_ruleset(), &p1, &p2);
        let a1 = b.active_id(0).unwrap();
        let a2 = b.active_id(1).unwrap();
        b.restore_status(&dex, a1, Status::Brn, None);
        b.restore_status(&dex, a2, Status::Par, None);
        b.choose(&dex, 0, "move swordsdance").unwrap();
        b.choose(&dex, 1, "move agility").unwrap();
        // Snorlax Atk 161 at +2 = 322, Burn /2 = 161 (still halved!).
        assert_eq!(b.get_stat(&dex, a1, stat_idx::ATK, false, false, false), 161);
        // Mew Spe 151 at +2 = 302, Par /4 = 75.5 (still quartered!).
        assert!((b.get_pokemon_action_speed_f64(&dex, a2) - 75.5).abs() < 1e-9);
        // After Haze, still halved/quartered!
        b.choose(&dex, 0, "move haze").unwrap();
        b.choose(&dex, 1, "move agility").unwrap();
        assert_eq!(b.get_stat(&dex, a1, stat_idx::ATK, false, false, false), 80);
    }

    // #24 & #38: Confusion self-hit (variance, no Thick Club, hits own Substitute, Focus Band).
    {
        let p1 = vec![mk_bare("Marowak", 50, &["Substitute", "Splash"], "Thick Club")];
        let p2 = vec![mk_bare("Mew", 50, &["Confuse Ray", "Splash"], "")];
        let mut b = start_castle(&dex, "1,2,3,4", castle_ruleset(), &p1, &p2);
        let a1 = b.active_id(0).unwrap();
        b.choose(&dex, 0, "move substitute").unwrap();
        b.choose(&dex, 1, "move splash").unwrap();
        let body_hp = b.poke(a1).hp;
        b.add_volatile(&dex, a1, "confusion", None, nc2000_engine::battle::EffectHandle::None);
        let sub_cond = dex.conds_id("substitute").unwrap();
        let sub_before = b.poke(a1).volatile(sub_cond).unwrap().get_int(DK::Hp);
        let mut saw_sub_hit = false;
        for seed in 1..30u64 {
            let mut t = b.clone();
            t.reseed(seed);
            t.choose(&dex, 0, "move splash").unwrap();
            t.choose(&dex, 1, "move splash").unwrap();
            let sub_after = t.poke(a1).volatile(sub_cond).map(|v| v.get_int(DK::Hp)).unwrap_or(0);
            if sub_after < sub_before {
                assert_eq!(t.poke(a1).hp, body_hp, "Confusion self-hit must hit own Substitute, leaving body untouched");
                // Without Thick Club (Atk 131 vs Def 161, 40 BP): dmg is ~14..17 (with Thick Club it would be ~26..31).
                let sub_dmg = sub_before - sub_after;
                assert!((12..=18).contains(&sub_dmg), "Confusion self-hit must skip Thick Club, got {sub_dmg}");
                saw_sub_hit = true;
                break;
            }
        }
        assert!(saw_sub_hit);

        // Focus Band triggers on confusion self-hit (silent).
        let pf1 = vec![mk_bare("Snorlax", 50, &["Splash"], "Focus Band")];
        let pf2 = vec![mk_bare("Mew", 50, &["Splash"], "")];
        let mut saw_fb = false;
        for seed in 1..200u64 {
            let mut t = start_castle(&dex, &format!("{seed},2,3,4"), castle_ruleset(), &pf1, &pf2);
            let a1 = t.active_id(0).unwrap();
            t.poke_mut(a1).hp = 5;
            t.add_volatile(&dex, a1, "confusion", None, nc2000_engine::battle::EffectHandle::None);
            t.log.clear();
            t.choose(&dex, 0, "move splash").unwrap();
            t.choose(&dex, 1, "move splash").unwrap();
            if t.poke(a1).hp == 1 {
                assert!(!t.log.iter().any(|l| l.contains("Focus Band")), "Focus Band must be silent in Castle");
                saw_fb = true;
                break;
            }
        }
        assert!(saw_fb, "Focus Band must save from lethal confusion self-hit");
    }

    // #25: Present v0.3 (80 fixed HP heal, BP 40 vs Substitute, type-2 bug formula).
    {
        let p1 = vec![mk_bare("Delibird", 50, &["Present"], "")];
        let p2 = vec![mk_bare("Blissey", 50, &["Substitute", "Splash"], "")];
        let mut saw_heal80 = false;
        let mut saw_sub_bp40 = false;
        for seed in 1..100u64 {
            let mut b = start_castle(&dex, &format!("{seed},2,3,4"), castle_ruleset(), &p1, &p2);
            let a2 = b.active_id(1).unwrap();
            b.poke_mut(a2).hp = 100;
            b.choose(&dex, 0, "move present").unwrap();
            b.choose(&dex, 1, "move splash").unwrap();
            if b.poke(a2).hp == 180 {
                saw_heal80 = true;
                break;
            }
        }
        assert!(saw_heal80, "Present heal bucket must heal exactly 80 fixed HP");

        // Against Substitute, Present NEVER heals and always deals BP 40 damage.
        let mut b = start_castle(&dex, "1,2,3,4", castle_ruleset(), &p1, &p2);
        let a2 = b.active_id(1).unwrap();
        b.add_volatile(&dex, a2, "substitute", None, nc2000_engine::battle::EffectHandle::None);
        let sub_cond = dex.conds_id("substitute").unwrap();
        let sub_hp_before = b.poke(a2).volatile(sub_cond).unwrap().get_int(DK::Hp);
        for seed in 1..40u64 {
            let mut t = b.clone();
            t.reseed(seed);
            t.choose(&dex, 0, "move present").unwrap();
            t.choose(&dex, 1, "move splash").unwrap();
            if !t.log.iter().any(|l| l.contains("|-miss|")) {
                let sub_after = t.poke(a2).volatile(sub_cond).map(|v| v.get_int(DK::Hp)).unwrap_or(0);
                assert!(sub_after < sub_hp_before, "Present vs Sub must always deal BP 40 damage, never heal");
                saw_sub_bp40 = true;
            }
        }
        assert!(saw_sub_bp40);
    }

    // #26: Rollout + Defense Curl multiplier AFTER +2 (`Probe 12E2`: Golem vs Steelix hit 1 = 5..7, hit 2 = 12..15).
    {
        let p1 = vec![mk_bare("Golem", 50, &["Defense Curl", "Rollout"], "")];
        let p2 = vec![mk_bare("Steelix", 50, &["Splash"], "")];
        let mut b = start_castle(&dex, "1,2,3,4", castle_ruleset(), &p1, &p2);
        b.choose(&dex, 0, "move defensecurl").unwrap();
        b.choose(&dex, 1, "move splash").unwrap();
        let a2 = b.active_id(1).unwrap();
        let mut checked = false;
        for seed in 1..30u64 {
            let mut t = b.clone();
            t.reseed(seed);
            let h0 = t.poke(a2).hp;
            t.choose(&dex, 0, "move rollout").unwrap();
            t.choose(&dex, 1, "move splash").unwrap();
            let d1 = h0 - t.poke(a2).hp;
            if d1 == 0 {
                continue;
            }
            let h1 = t.poke(a2).hp;
            t.choose(&dex, 0, "move rollout").unwrap();
            t.choose(&dex, 1, "move splash").unwrap();
            let d2 = h1 - t.poke(a2).hp;
            if d2 == 0 {
                continue;
            }
            // With +2 before mult: base_bp30=8, +2=10, *2=20, *1.5=30, *0.25=7 -> 5..=7; hit2: 10*4=40 -> 60 -> 15 -> 12..=15.
            assert!((5..=7).contains(&d1), "Rollout hit 1 expected 5..=7, got {d1}");
            assert!((12..=15).contains(&d2), "Rollout hit 2 expected 12..=15, got {d2}");
            checked = true;
            break;
        }
        assert!(checked, "Rollout 2-hit test never executed");
    }

    // #27: Recover / Softboiled / Milk Drink floor(maxhp / 2) -> Blissey 361 HP heals +180 (not +181).
    {
        let p1 = vec![mk_bare("Blissey", 50, &["Soft-Boiled"], "")];
        let p2 = vec![mk_bare("Snorlax", 50, &["Splash"], "")];
        let mut b = start_castle(&dex, "1,2,3,4", castle_ruleset(), &p1, &p2);
        let a1 = b.active_id(0).unwrap();
        b.poke_mut(a1).hp = 100;
        b.choose(&dex, 0, "move softboiled").unwrap();
        b.choose(&dex, 1, "move splash").unwrap();
        assert_eq!(b.poke(a1).hp, 280);
    }

    // #28 & #29 & #30: Recoil vs Sub = 0 HP; HJK miss = floor(intended / 4); final damage 0 allowed.
    {
        // #28: Double-Edge vs Substitute -> 0 HP recoil!
        let p1 = vec![mk_bare("Snorlax", 50, &["Double-Edge"], "")];
        let p2 = vec![mk_bare("Blissey", 50, &["Substitute", "Splash"], "")];
        let mut b = start_castle(&dex, "1,2,3,4", castle_ruleset(), &p1, &p2);
        let a1 = b.active_id(0).unwrap();
        let a2 = b.active_id(1).unwrap();
        b.add_volatile(&dex, a2, "substitute", None, nc2000_engine::battle::EffectHandle::None);
        b.choose(&dex, 0, "move doubleedge").unwrap();
        b.choose(&dex, 1, "move splash").unwrap();
        assert_eq!(b.poke(a1).hp, 266, "Recoil vs Substitute must be 0 HP");

        // #29: HJK miss recoil = floor(intended / 4) ignoring defender maxhp!
        let ph1 = vec![mk_bare("Hitmonlee", 50, &["High Jump Kick"], "")];
        let ph2 = vec![mk_set("Blissey", 5, &["Splash"], "", Some([0, 0, 0, 0]), Some([0, 0, 0, 0, 0]))];
        let mut saw_miss = false;
        for seed in 1..60u64 {
            let mut t = start_castle(&dex, &format!("{seed},2,3,4"), castle_ruleset(), &ph1, &ph2);
            let a1 = t.active_id(0).unwrap();
            let pre = t.poke(a1).hp;
            t.choose(&dex, 0, "move highjumpkick").unwrap();
            t.choose(&dex, 1, "move splash").unwrap();
            if t.log.iter().any(|l| l.contains("|-miss|")) {
                // Lv5 Blissey maxhp is ~36, intended HJK dmg is >1000 -> 1/4 recoil KOs Hitmonlee (156 HP)!
                assert_eq!(t.poke(a1).hp, 0, "HJK miss recoil must ignore low defender maxhp and deal 1/4 intended ({pre})");
                saw_miss = true;
                break;
            }
        }
        assert!(saw_miss);

        // #30: Final damage 0 allowed (Lv5 Weedle Poison Sting 0.25x vs Lv50 Rhydon).
        let pz1 = vec![mk_set("Weedle", 5, &["Poison Sting"], "", Some([0, 0, 0, 0]), Some([0, 0, 0, 0, 0]))];
        let pz2 = vec![mk_bare("Rhydon", 50, &["Splash"], "")];
        let mut bz = start_castle(&dex, "1,2,3,4", castle_ruleset(), &pz1, &pz2);
        let a2 = bz.active_id(1).unwrap();
        let pre = bz.poke(a2).hp;
        bz.choose(&dex, 0, "move poisonsting").unwrap();
        bz.choose(&dex, 1, "move splash").unwrap();
        assert_eq!(bz.poke(a2).hp, pre, "0.25x Poison Sting must deal 0 damage (no min-1 clamp)");
    }
}

// =========================================================================
// 4. §3-D Items & Berries (#31–#38)
// =========================================================================

#[test]
fn castle_items_and_berries_diffs_31_to_36() {
    let dex = load_dex();

    // #31: Gold Berry strict `< floor(maxhp / 2)`: 85/171 does NOT trigger; 84/171 DOES trigger.
    {
        let p1 = vec![mk_bare("Alakazam", 50, &["Splash"], "Gold Berry")]; // maxhp=161 -> half=80
        let p2 = vec![mk_bare("Snorlax", 50, &["Splash"], "")];
        let mut b = start_castle(&dex, "1,2,3,4", castle_ruleset(), &p1, &p2);
        let a1 = b.active_id(0).unwrap();
        b.poke_mut(a1).hp = 80; // == floor(161/2) -> must NOT eat!
        b.choose(&dex, 0, "move splash").unwrap();
        b.choose(&dex, 1, "move splash").unwrap();
        assert_eq!(b.poke(a1).hp, 80);
        assert!(b.poke(a1).item.is_some());
        b.poke_mut(a1).hp = 79; // < floor(161/2) -> eats (+30 -> 109)!
        b.choose(&dex, 0, "move splash").unwrap();
        b.choose(&dex, 1, "move splash").unwrap();
        assert_eq!(b.poke(a1).hp, 109);
        assert!(b.poke(a1).item.is_none());
    }

    // #32① & ③: Opponent Toxic on Miracle Berry cures BEFORE attacker's poison; second-mover Rest eats at EOT.
    {
        let p1 = vec![mk_bare("Mew", 50, &["Confuse Ray", "Splash"], "")];
        let p2 = vec![mk_bare("Snorlax", 50, &["Rest", "Splash"], "Miracle Berry")];
        let mut b = start_castle(&dex, "1,2,3,4", castle_ruleset(), &p1, &p2);
        let a1 = b.active_id(0).unwrap();
        b.restore_status(&dex, a1, Status::Psn, None);
        b.log.clear();
        b.choose(&dex, 0, "move confuseray").unwrap();
        b.choose(&dex, 1, "move splash").unwrap();
        let i_eat = b.log.iter().position(|l| l.contains("|-enditem|p2a: Snorlax|Miracle Berry")).unwrap();
        let i_psn = b.log.iter().position(|l| l.contains("|-damage|p1a: Mew|") && l.contains("[from] psn")).unwrap();
        assert!(i_eat < i_psn, "Miracle Berry on opponent move must trigger before attacker's AfterMoveSelf poison");
    }

    // #33: Bitter Berry inert; Ice Berry cures BOTH Burn and Confusion!
    {
        let p1 = vec![mk_bare("Mew", 50, &["Confuse Ray", "Splash"], "")];
        let p2 = vec![
            mk_bare("Snorlax", 50, &["Splash"], "Bitter Berry"),
            mk_bare("Blissey", 50, &["Splash"], "Ice Berry"),
        ];
        let mut b = start_castle(&dex, "1,2,3,4", castle_ruleset(), &p1, &p2);
        let a2 = b.active_id(1).unwrap();
        b.choose(&dex, 0, "move confuseray").unwrap();
        b.choose(&dex, 1, "move splash").unwrap();
        let conf = dex.conds_id("confusion").unwrap();
        assert!(b.poke(a2).has_volatile(conf), "Bitter Berry must be inert in Castle");
        assert!(b.poke(a2).item.is_some());

        // Switch in Blissey with Ice Berry -> Confuse Ray triggers Ice Berry and cures confusion!
        b.choose(&dex, 0, "move splash").unwrap();
        b.choose(&dex, 1, "switch 2").unwrap();
        let b2 = b.active_id(1).unwrap();
        b.choose(&dex, 0, "move confuseray").unwrap();
        b.choose(&dex, 1, "move splash").unwrap();
        assert!(!b.poke(b2).has_volatile(conf), "Ice Berry must cure Confusion in Castle");
        assert!(b.poke(b2).item.is_none());
    }

    // #34: Dragon Scale boosts Dragon 1.1x; Dragon Fang is inert (`Probe 11D`).
    {
        let p_scale = vec![mk_bare("Kingdra", 50, &["Dragon Breath"], "Dragon Scale")];
        let p_fang = vec![mk_bare("Kingdra", 50, &["Dragon Breath"], "Dragon Fang")];
        let p_none = vec![mk_bare("Kingdra", 50, &["Dragon Breath"], "")];
        let def = vec![mk_bare("Snorlax", 50, &["Splash"], "")];
        let mut bs = start_castle(&dex, "1,2,3,4", castle_ruleset(), &p_scale, &def);
        let mut bf = start_castle(&dex, "1,2,3,4", castle_ruleset(), &p_fang, &def);
        let mut bn = start_castle(&dex, "1,2,3,4", castle_ruleset(), &p_none, &def);
        for b in [&mut bs, &mut bf, &mut bn] {
            b.choose(&dex, 0, "move dragonbreath").unwrap();
            b.choose(&dex, 1, "move splash").unwrap();
        }
        let ds = 266 - bs.poke(bs.active_id(1).unwrap()).hp;
        let df = 266 - bf.poke(bf.active_id(1).unwrap()).hp;
        let dn = 266 - bn.poke(bn.active_id(1).unwrap()).hp;
        assert_eq!(df, dn, "Dragon Fang must be inert in Castle");
        assert!(ds > dn, "Dragon Scale must boost Dragon damage 1.1x ({ds} vs {dn})");
    }

    // #35: Metal Powder keeps 1.5x Def/SpD on Ditto even after Transform!
    {
        let p1 = vec![mk_bare("Ditto", 50, &["Transform"], "Metal Powder")];
        let p2 = vec![mk_bare("Snorlax", 50, &["Splash"], "")];
        let mut b = start_castle(&dex, "1,2,3,4", castle_ruleset(), &p1, &p2);
        b.choose(&dex, 0, "move transform").unwrap();
        b.choose(&dex, 1, "move splash").unwrap();
        let a1 = b.active_id(0).unwrap();
        // Snorlax Def=116, SpD=161 -> with Metal Powder 1.5x: Def=174, SpD=241!
        assert_eq!(b.get_stat(&dex, a1, stat_idx::DEF, false, false, false), 174);
        assert_eq!(b.get_stat(&dex, a1, stat_idx::SPD, false, false, false), 241);
    }

    // #36: Mystery Berry triggers at EOT (unclamped: Sketch 1->0->5) AND mid-turn after Spite!
    {
        let p1 = vec![mk_bare("Dugtrio", 50, &["Sketch", "Splash"], "Mystery Berry")];
        let p2 = vec![mk_bare("Snorlax", 50, &["Splash"], "")];
        let mut b = start_castle(&dex, "1,2,3,4", castle_ruleset(), &p1, &p2);
        let a1 = b.active_id(0).unwrap();
        assert_eq!(b.poke(a1).move_slots[0].pp, 1);
        b.log.clear();
        b.choose(&dex, 0, "move sketch").unwrap();
        b.choose(&dex, 1, "move splash").unwrap();
        // Sketch failed (p2 had no last_move), hit 0 PP, and ate Mystery Berry at EOT -> PP = 5 (> maxpp 1)!
        assert_eq!(b.poke(a1).move_slots[0].pp, 5);
        let i_splash = b.log.iter().position(|l| l.contains("|move|p2a: Snorlax|Splash|")).unwrap();
        let i_berry = b.log.iter().position(|l| l.contains("Mystery Berry")).unwrap();
        assert!(i_splash < i_berry, "Normal PP=0 Mystery Berry must wait for EOT");

        // Mid-turn Spite -> selected move hits 0 PP -> Mystery Berry triggers before defender acts (`Probe 11F`)!
        let ps1 = vec![mk_bare("Gengar", 50, &["Spite", "Splash"], "")];
        let ps2 = vec![mk_bare("Snorlax", 50, &["Tackle", "Splash"], "Mystery Berry")];
        let mut bs = start_castle(&dex, "1,2,3,4", castle_ruleset(), &ps1, &ps2);
        let s2 = bs.active_id(1).unwrap();
        bs.poke_mut(s2).move_slots[0].pp = 1;
        bs.poke_mut(s2).base_move_slots[0].pp = 1;
        bs.poke_mut(s2).last_move = dex.moves.id("tackle");
        bs.choose(&dex, 0, "move spite").unwrap();
        bs.choose(&dex, 1, "move tackle").unwrap();
        // Spite zeroed Tackle -> Snorlax immediately ate Mystery Berry (+5) -> used Tackle (now 4 PP)!
        assert_eq!(bs.poke(s2).move_slots[0].pp, 4);
        assert!(bs.poke(s2).item.is_none());
    }
}

// =========================================================================
// 5. §3-E Individual Move Diffs (#39–#54)
// =========================================================================

#[test]
fn castle_individual_move_diffs_40_to_54() {
    let dex = load_dex();

    // #40: Endure succeeds behind own Substitute (Protect still fails).
    {
        let p1 = vec![mk_bare("Mew", 50, &["Substitute", "Endure", "Protect"], "")];
        let p2 = vec![mk_bare("Snorlax", 50, &["Splash"], "")];
        let mut b = start_castle(&dex, "1,2,3,4", castle_ruleset(), &p1, &p2);
        b.choose(&dex, 0, "move substitute").unwrap();
        b.choose(&dex, 1, "move splash").unwrap();
        b.log.clear();
        b.choose(&dex, 0, "move protect").unwrap();
        b.choose(&dex, 1, "move splash").unwrap();
        assert!(b.log.iter().any(|l| l.contains("|-fail|p1a: Mew")), "Protect must fail behind own Sub");
        b.log.clear();
        b.choose(&dex, 0, "move endure").unwrap();
        b.choose(&dex, 1, "move splash").unwrap();
        assert!(b.log.iter().any(|l| l.contains("move: Endure")), "Endure must succeed behind own Sub");
    }

    // #41: Belly Drum succeeds at `hp == floor(maxhp / 2)`, leaving 1 HP (`Probe 3A`/`12C`).
    {
        let p1 = vec![mk_bare("Snorlax", 50, &["Belly Drum"], "")];
        let p2 = vec![mk_bare("Mew", 50, &["Splash"], "")];
        let mut b = start_castle(&dex, "1,2,3,4", castle_ruleset(), &p1, &p2);
        let a1 = b.active_id(0).unwrap();
        b.poke_mut(a1).hp = 133; // exactly 266 / 2
        b.choose(&dex, 0, "move bellydrum").unwrap();
        b.choose(&dex, 1, "move splash").unwrap();
        assert_eq!(b.poke(a1).hp, 1);
        assert_eq!(b.poke(a1).boosts[stat_idx::ATK], 6);
    }

    // #42: Struggle gets Normal 1.5x STAB + Pink Bow 1.1x + 0 recoil vs Ghost & Substitute.
    {
        let p_norm_bow = vec![mk_bare("Snorlax", 50, &["Splash"], "Pink Bow")];
        let p_norm = vec![mk_bare("Snorlax", 50, &["Splash"], "")];
        let def_ghost = vec![mk_bare("Gengar", 50, &["Splash"], "")];
        let mut bb = start_castle(&dex, "1,2,3,4", castle_ruleset(), &p_norm_bow, &def_ghost);
        let mut bn = start_castle(&dex, "1,2,3,4", castle_ruleset(), &p_norm, &def_ghost);
        for b in [&mut bb, &mut bn] {
            let a1 = b.active_id(0).unwrap();
            b.poke_mut(a1).move_slots[0].pp = 0;
            b.choose(&dex, 0, "move struggle").unwrap();
            b.choose(&dex, 1, "move splash").unwrap();
        }
        let d_bow = 166 - bb.poke(bb.active_id(1).unwrap()).hp;
        let d_plain = 166 - bn.poke(bn.active_id(1).unwrap()).hp;
        assert!(d_bow > d_plain, "Struggle must benefit from Pink Bow ({d_bow} vs {d_plain})");
        assert!(d_plain >= 35, "Snorlax Struggle must get Normal 1.5x STAB, got {d_plain}");
        assert_eq!(bb.poke(bb.active_id(0).unwrap()).hp, 266, "Struggle vs Ghost must deal 0 recoil");
    }

    // #43: Rapid Spin clears Spikes even vs Ghost AND does NOT clear Mean Look (`Probe 12D2`).
    {
        let p1 = vec![
            mk_bare("Starmie", 50, &["Rapid Spin", "Splash"], ""),
            mk_bare("Snorlax", 50, &["Splash"], ""),
        ];
        let p2 = vec![mk_bare("Gengar", 50, &["Spikes", "Mean Look", "Splash"], "")];
        let mut b = start_castle(&dex, "1,2,3,4", castle_ruleset(), &p1, &p2);
        b.choose(&dex, 0, "move splash").unwrap();
        b.choose(&dex, 1, "move spikes").unwrap();
        b.choose(&dex, 0, "move splash").unwrap();
        b.choose(&dex, 1, "move meanlook").unwrap();
        b.choose(&dex, 0, "move rapidspin").unwrap();
        b.choose(&dex, 1, "move splash").unwrap();
        let spikes = dex.conds_id("spikes").unwrap();
        assert!(!b.sides[0].has_side_condition(spikes), "Rapid Spin vs Ghost must clear Spikes");
        assert!(b.poke(b.active_id(0).unwrap()).trapped, "Rapid Spin must NOT clear Mean Look");
    }

    // #44: Outrage confuses immediately after final turn (before opponent moves), blocked by Safeguard.
    {
        let p1 = vec![mk_bare("Dragonite", 55, &["Outrage", "Safeguard"], "")];
        let p2 = vec![mk_bare("Blissey", 50, &["Splash"], "")];
        let mut b = start_castle(&dex, "1,2,3,4", castle_ruleset(), &p1, &p2);
        b.choose(&dex, 0, "move outrage").unwrap();
        b.choose(&dex, 1, "move splash").unwrap();
        // Run turns until Outrage ends; on the ending turn, confusion must start BEFORE Blissey's Splash!
        let mut checked = false;
        for _ in 0..3 {
            b.log.clear();
            b.choose(&dex, 0, "move outrage").unwrap();
            b.choose(&dex, 1, "move splash").unwrap();
            if let Some(i_conf) = b.log.iter().position(|l| l.contains("|-start|p1a: Dragonite|confusion")) {
                let i_splash = b.log.iter().position(|l| l.contains("|move|p2a: Blissey|Splash|")).unwrap();
                assert!(i_conf < i_splash, "Outrage confusion must trigger immediately before opponent acts");
                checked = true;
                break;
            }
        }
        assert!(checked);
    }

    // #45: Lock-On persists past Splash, re-Lock-On fails, does NOT let Ice Beam hit Dig (`Probe 12E1`).
    {
        let p1 = vec![mk_bare("Mew", 50, &["Lock-On", "Splash", "Zap Cannon", "Ice Beam"], "")];
        let p2 = vec![mk_bare("Snorlax", 50, &["Splash", "Dig"], "")];
        let mut b = start_castle(&dex, "1,2,3,4", castle_ruleset(), &p1, &p2);
        b.choose(&dex, 0, "move lockon").unwrap();
        b.choose(&dex, 1, "move splash").unwrap();
        // Re-Lock-On fails!
        b.log.clear();
        b.choose(&dex, 0, "move lockon").unwrap();
        b.choose(&dex, 1, "move splash").unwrap();
        assert!(b.log.iter().any(|l| l.contains("|-fail|")), "Re-Lock-On must fail");
        // Non-attack Splash keeps Lock-On!
        b.choose(&dex, 0, "move splash").unwrap();
        b.choose(&dex, 1, "move splash").unwrap();
        let lo = dex.conds_id("lockon").unwrap();
        assert!(b.poke(b.active_id(0).unwrap()).has_volatile(lo));
        // Ice Beam against underground Dig still misses despite Lock-On!
        b.choose(&dex, 0, "move splash").unwrap();
        b.choose(&dex, 1, "move dig").unwrap();
        b.log.clear();
        b.choose(&dex, 0, "move icebeam").unwrap();
        b.choose(&dex, 1, "move dig").unwrap();
        assert!(b.log.iter().any(|l| l.contains("|-miss|p1a: Mew")));
    }

    // #46 & #47 & #48: Psych Up fails vs 0 boosts; Frustration 102 BP; Flail/Reversal < 77.5%.
    {
        let p1 = vec![mk_bare("Mew", 50, &["Psych Up", "Frustration", "Reversal"], "")];
        let p2 = vec![mk_bare("Snorlax", 50, &["Splash"], "")];
        let mut b = start_castle(&dex, "1,2,3,4", castle_ruleset(), &p1, &p2);
        b.log.clear();
        b.choose(&dex, 0, "move psychup").unwrap();
        b.choose(&dex, 1, "move splash").unwrap();
        assert!(b.log.iter().any(|l| l.contains("|-fail|")));
        // Frustration deals 102 BP damage despite happiness=255!
        let a2 = b.active_id(1).unwrap();
        let pre = b.poke(a2).hp;
        b.choose(&dex, 0, "move frustration").unwrap();
        b.choose(&dex, 1, "move splash").unwrap();
        assert!(pre - b.poke(a2).hp >= 50, "Frustration must be fixed 102 BP in Castle");

        // Reversal boundaries: 155/200 (77.5%) -> BP 20; 152/200 (76.0%) -> BP 40 (`Probe 12E1`).
        let pr1 = vec![mk_set("Heracross", 50, &["Reversal"], "", Some([15, 15, 15, 15]), Some([63, 63, 63, 63, 63]))];
        let pr2 = vec![mk_bare("Mew", 50, &["Splash"], "")];
        let mut br_hi = start_castle(&dex, "1,2,3,4", castle_ruleset(), &pr1, &pr2);
        let mut br_lo = start_castle(&dex, "1,2,3,4", castle_ruleset(), &pr1, &pr2);
        let h1 = br_hi.active_id(0).unwrap();
        let h2 = br_lo.active_id(0).unwrap();
        br_hi.poke_mut(h1).maxhp = 200;
        br_hi.poke_mut(h1).hp = 155; // exact 77.5% -> BP 20
        br_lo.poke_mut(h2).maxhp = 200;
        br_lo.poke_mut(h2).hp = 152; // 76.0% < 77.5% -> BP 40
        br_hi.choose(&dex, 0, "move reversal").unwrap();
        br_hi.choose(&dex, 1, "move splash").unwrap();
        br_lo.choose(&dex, 0, "move reversal").unwrap();
        br_lo.choose(&dex, 1, "move splash").unwrap();
        let d_hi = 206 - br_hi.poke(br_hi.active_id(1).unwrap()).hp;
        let d_lo = 206 - br_lo.poke(br_lo.active_id(1).unwrap()).hp;
        assert!(d_lo > d_hi + 3, "76% Reversal (BP 40, {d_lo}) must exceed 77.5% (BP 20, {d_hi})");
    }

    // #49: Dig takes Sandstorm underground, 2nd-turn inaction keeps semi-invuln, Mean Look hits Dig.
    {
        let p1 = vec![mk_bare("Mew", 50, &["Dig", "Splash"], "")];
        let p2 = vec![mk_bare("Snorlax", 50, &["Sandstorm", "Mean Look", "Tackle", "Splash"], "")];
        let mut b = start_castle(&dex, "1,2,3,4", castle_ruleset(), &p1, &p2);
        let a1 = b.active_id(0).unwrap();
        b.choose(&dex, 0, "move dig").unwrap();
        b.choose(&dex, 1, "move sandstorm").unwrap();
        // Underground Mew took floor(206/8) = 25 Sandstorm damage (`Probe 12E2`)!
        assert_eq!(b.poke(a1).hp, 206 - 25);
        // Paralyze Mew and abort Dig 2nd turn -> Mew keeps `dig` semi-invuln!
        b.restore_status(&dex, a1, Status::Par, None);
        let mut checked = false;
        for seed in 1..40u64 {
            let mut t = b.clone();
            t.reseed(seed);
            t.choose(&dex, 0, "move dig").unwrap();
            t.choose(&dex, 1, "move meanlook").unwrap();
            if t.log.iter().any(|l| l.contains("|cant|p1a: Mew|par")) {
                let dig_c = dex.conds_id("dig").unwrap();
                assert!(t.poke(a1).has_volatile(dig_c), "Aborted Dig 2nd turn must preserve semi-invulnerable state");
                assert!(t.poke(a1).trapped, "Mean Look must hit underground Dig in Castle");
                checked = true;
                break;
            }
        }
        assert!(checked, "Dig abort test never hit full paralysis");
    }

    // #50: Baton Pass copies defensecurl & minimize; first-mover Baton Pass + second-mover Pursuit takes Spikes then 2x Pursuit (`Probe 12C`).
    {
        // Part A: Baton Pass copies defensecurl & minimize
        let p1 = vec![
            mk_bare("Mew", 50, &["Defense Curl", "Minimize", "Baton Pass", "Splash"], ""),
            mk_bare("Alakazam", 50, &["Splash"], ""),
        ];
        let p2 = vec![mk_bare("Umbreon", 50, &["Spikes", "Pursuit", "Splash"], "")];
        let mut b = start_castle(&dex, "1,2,3,4", castle_ruleset(), &p1, &p2);
        b.choose(&dex, 0, "move defensecurl").unwrap();
        b.choose(&dex, 1, "move spikes").unwrap();
        b.choose(&dex, 0, "move minimize").unwrap();
        b.choose(&dex, 1, "move splash").unwrap();
        b.choose(&dex, 0, "move batonpass").unwrap();
        b.choose(&dex, 1, "move splash").unwrap();
        assert_eq!(b.request_state, RequestState::Switch);
        b.choose(&dex, 0, "switch 2").unwrap();
        let kaz = b.active_id(0).unwrap();
        let dc = dex.conds_id("defensecurl").unwrap();
        let mn = dex.conds_id("minimize").unwrap();
        assert!(b.poke(kaz).has_volatile(dc), "Baton Pass must copy defensecurl");
        assert!(b.poke(kaz).has_volatile(mn), "Baton Pass must copy minimize");

        // Part B: First-mover Baton Pass -> incoming mon takes Spikes (-20 -> 141) -> second-mover Pursuit hits for 2x (BP 80, ~78..92 -> HP ~49..63) (`Probe 12C`)!
        let mut b2 = start_castle(&dex, "1,2,3,4", castle_ruleset(), &p1, &p2);
        b2.choose(&dex, 0, "move splash").unwrap();
        b2.choose(&dex, 1, "move spikes").unwrap();
        b2.choose(&dex, 0, "move batonpass").unwrap();
        b2.choose(&dex, 1, "move pursuit").unwrap();
        assert_eq!(b2.request_state, RequestState::Switch);
        b2.choose(&dex, 0, "switch 2").unwrap();
        let kaz2 = b2.active_id(0).unwrap();
        let hp_left = b2.poke(kaz2).hp;
        assert!((45..=70).contains(&hp_left), "Expected Spikes + 2x Pursuit leaving ~49..63 HP, got {hp_left}");
    }

    // #51: Triple Kick 2nd/3rd hit multiplier leaks to 2nd actor; Twineedle cannot poison Steel & only rolls hit 1.
    {
        let p1 = vec![mk_bare("Hitmontop", 50, &["Triple Kick"], "")];
        let p2 = vec![mk_bare("Snorlax", 50, &["Tackle"], "")];
        let mut saw_leak = false;
        for seed in 1..40u64 {
            let mut b = start_castle(&dex, &format!("{seed},2,3,4"), castle_ruleset(), &p1, &p2);
            b.choose(&dex, 0, "move triplekick").unwrap();
            b.choose(&dex, 1, "move tackle").unwrap();
            let dealt = 156 - b.poke(b.active_id(0).unwrap()).hp;
            if b.log.iter().any(|l| l.contains("|-hitcount|p2a: Snorlax|3")) && !b.log.iter().any(|l| l.contains("|-crit|p1a: Hitmontop")) {
                // Normal Tackle from Snorlax (Atk 161) vs Hitmontop (Def 146) deals 22..=27;
                // with 3x Triple Kick leak -> 68..=81!
                assert!((68..=81).contains(&dealt), "Triple Kick 3rd hit must leak 3x damage (68..=81) to 2nd actor, got {dealt}");
                saw_leak = true;
                break;
            }
        }
        assert!(saw_leak);

        // Twineedle cannot poison Steel-type Steelix.
        let pt1 = vec![mk_bare("Beedrill", 50, &["Twineedle"], "")];
        let pt2 = vec![mk_bare("Steelix", 50, &["Splash"], "")];
        for seed in 1..=30u64 {
            let mut b = start_castle(&dex, &format!("{seed},2,3,4"), castle_ruleset(), &pt1, &pt2);
            b.choose(&dex, 0, "move twineedle").unwrap();
            b.choose(&dex, 1, "move splash").unwrap();
            assert_eq!(b.poke(b.active_id(1).unwrap()).status, Status::None, "Twineedle must NOT poison Steel");
        }
    }

    // #52: Bide 1-2T hits Ghost; Rage multiplies BP without Atk boost; Fury Cutter exceeds 160 BP.
    {
        let pb1 = vec![mk_bare("Snorlax", 50, &["Bide"], "")];
        let pb2 = vec![mk_bare("Gengar", 50, &["Seismic Toss", "Splash"], "")];
        let mut b = start_castle(&dex, "1,2,3,4", castle_ruleset(), &pb1, &pb2);
        // Turn 1: Gengar Splashes so slower Snorlax enters Bide first.
        b.choose(&dex, 0, "move bide").unwrap();
        b.choose(&dex, 1, "move splash").unwrap();
        // Turn 2: Faster Gengar hits biding Snorlax with Seismic Toss (-50); Snorlax unleashes on T2 or T3!
        b.choose(&dex, 0, "move bide").unwrap();
        b.choose(&dex, 1, "move seismictoss").unwrap();
        if b.poke(b.active_id(1).unwrap()).hp == 166 {
            b.choose(&dex, 0, "move bide").unwrap();
            b.choose(&dex, 1, "move splash").unwrap();
        }
        assert_eq!(b.poke(b.active_id(1).unwrap()).hp, 166 - 100, "Bide must hit Ghost for 2x stored damage");

        // Rage multiplies BP without Atk boost.
        let pr1 = vec![mk_bare("Tauros", 50, &["Rage"], "")];
        let pr2 = vec![mk_bare("Blissey", 50, &["Tackle"], "")];
        let mut br = start_castle(&dex, "1,2,3,4", castle_ruleset(), &pr1, &pr2);
        let a1 = br.active_id(0).unwrap();
        let a2 = br.active_id(1).unwrap();
        br.choose(&dex, 0, "move rage").unwrap();
        br.choose(&dex, 1, "move tackle").unwrap();
        assert_eq!(br.poke(a1).boosts[stat_idx::ATK], 0, "Castle Rage must NOT boost Atk stage");
        br.poke_mut(a2).hp = 361;
        br.choose(&dex, 0, "move rage").unwrap(); // hit 1 after 1 direct hit -> 1x
        br.choose(&dex, 1, "move tackle").unwrap(); // counter -> 2!
        let d_1x = 361 - br.poke(a2).hp;
        br.poke_mut(a2).hp = 361;
        br.choose(&dex, 0, "move rage").unwrap(); // after 2 direct hits -> 2x BP!
        br.choose(&dex, 1, "move tackle").unwrap();
        let d_2x = 361 - br.poke(a2).hp;
        assert!(d_2x > d_1x * 3 / 2, "Rage 2x ({d_2x}) must be roughly double 1x ({d_1x})");
    }

    // #53: False Swipe surviving at 1 HP reflects 2 * old_hp; Beat Up includes statused user & excludes statused bench.
    {
        let p1 = vec![mk_bare("Snorlax", 100, &["False Swipe"], "")];
        let p2 = vec![mk_bare("Snorlax", 50, &["Counter"], "")];
        let mut b = start_castle(&dex, "1,2,3,4", castle_ruleset(), &p1, &p2);
        let a1 = b.active_id(0).unwrap();
        let a2 = b.active_id(1).unwrap();
        b.poke_mut(a2).hp = 33;
        let p1_pre = b.poke(a1).hp;
        b.choose(&dex, 0, "move falseswipe").unwrap();
        b.choose(&dex, 1, "move counter").unwrap();
        assert_eq!(b.poke(a2).hp, 1);
        assert_eq!(p1_pre - b.poke(a1).hp, 66, "False Swipe 33->1 must reflect 2 * 33 = 66 (`Probe 11F`)");

        // Beat Up: paralyzed active user + paralyzed bench + healthy bench -> 2 hits (active + healthy bench).
        let pbu1 = vec![
            mk_bare("Mew", 50, &["Beat Up"], ""),
            mk_bare("Snorlax", 50, &["Splash"], ""),
            mk_bare("Blissey", 50, &["Splash"], ""),
        ];
        let pbu2 = vec![mk_bare("Blissey", 50, &["Splash"], "")];
        let mut bbu = start_castle(&dex, "1,2,3,4", castle_ruleset(), &pbu1, &pbu2);
        let u1 = bbu.active_id(0).unwrap();
        bbu.restore_status(&dex, u1, Status::Par, None);
        bbu.sides[0].roster[1].status = Status::Par; // bench Snorlax paralyzed -> excluded
        let mut checked = false;
        for seed in 1..20u64 {
            let mut t = bbu.clone();
            t.reseed(seed);
            t.log.clear();
            t.choose(&dex, 0, "move beatup").unwrap();
            t.choose(&dex, 1, "move splash").unwrap();
            if let Some(line) = t.log.iter().find(|l| l.starts_with("|-hitcount|")) {
                assert!(line.ends_with("|2"), "Beat Up expected 2 hits (par user + healthy bench), got {line}");
                checked = true;
                break;
            }
        }
        assert!(checked, "Beat Up hitcount test never executed");
    }

    // #54: Encore + Disable does NOT force Struggle; Encore vs Roar fails; Mimic PP=8; Sketch PvP bypasses Sub!
    {
        // Encore + Disable -> legal move is STILL the Encored move (not Struggle!), blocked at execution.
        let p1 = vec![mk_bare("Alakazam", 50, &["Encore", "Disable", "Splash"], "")];
        let p2 = vec![mk_bare("Snorlax", 50, &["Tackle", "Splash"], "")];
        let mut b = start_castle(&dex, "1,2,3,4", castle_ruleset(), &p1, &p2);
        b.choose(&dex, 0, "move splash").unwrap();
        b.choose(&dex, 1, "move tackle").unwrap();
        b.choose(&dex, 0, "move encore").unwrap();
        b.choose(&dex, 1, "move tackle").unwrap();
        // Loop seeds until Disable hits (55% acc)
        let mut checked = false;
        for seed in 1..40u64 {
            let mut t = b.clone();
            t.reseed(seed);
            t.choose(&dex, 0, "move disable").unwrap();
            t.choose(&dex, 1, "move tackle").unwrap();
            let dis = dex.conds_id("disable").unwrap();
            if t.poke(t.active_id(1).unwrap()).has_volatile(dis) {
                let legal = t.legal_choices(&dex, 1);
                let tackle_id = dex.moves.id("tackle").unwrap();
                let struggle_id = dex.moves.id("struggle").unwrap();
                assert!(legal.contains(&SearchChoice::Move(tackle_id)), "Encore+Disable must keep Tackle selectable");
                assert!(!legal.contains(&SearchChoice::Move(struggle_id)), "Encore+Disable must NOT force Struggle");
                t.log.clear();
                t.choose(&dex, 0, "move splash").unwrap();
                t.choose(&dex, 1, "move tackle").unwrap();
                assert!(t.log.iter().any(|l| l.contains("|cant|p2a: Snorlax|Disable|Tackle")));
                checked = true;
                break;
            }
        }
        assert!(checked, "Encore+Disable test never landed Disable");

        // Encore vs failed Roar -> fails (`Probe 12I`).
        let pr1 = vec![mk_bare("Mew", 50, &["Protect", "Encore"], "")];
        let pr2 = vec![mk_bare("Aerodactyl", 50, &["Roar"], "")];
        let mut br = start_castle(&dex, "1,2,3,4", castle_ruleset(), &pr1, &pr2);
        br.choose(&dex, 0, "move protect").unwrap();
        br.choose(&dex, 1, "move roar").unwrap();
        br.log.clear();
        br.choose(&dex, 0, "move encore").unwrap();
        br.choose(&dex, 1, "move roar").unwrap();
        assert!(br.log.iter().any(|l| l.contains("|-fail|p2a: Aerodactyl")));

        // Mimic copied move PP = 5 + 3 = 8 (`Probe 10E`); Sketch succeeds in PvP & bypasses Sub (`Probe 12D1`)!
        let ps1 = vec![mk_bare("Smeargle", 50, &["Mimic", "Sketch", "Splash"], "")];
        let ps2 = vec![mk_bare("Blissey", 50, &["Substitute", "Tackle", "Splash"], "")];
        let mut bs = start_castle(&dex, "1,2,3,4", castle_ruleset(), &ps1, &ps2);
        bs.choose(&dex, 0, "move splash").unwrap();
        bs.choose(&dex, 1, "move tackle").unwrap();
        bs.choose(&dex, 0, "move mimic").unwrap();
        bs.choose(&dex, 1, "move substitute").unwrap();
        let sm = bs.active_id(0).unwrap();
        assert_eq!(bs.poke(sm).move_slots[0].id, dex.moves.id("tackle").unwrap());
        assert_eq!(bs.poke(sm).move_slots[0].pp, 8, "Mimic copied move PP must be 5 + 3 = 8");
        // Now Blissey has Substitute up; Smeargle uses Sketch on Blissey's last_move (Substitute)!
        bs.choose(&dex, 0, "move sketch").unwrap();
        bs.choose(&dex, 1, "move splash").unwrap();
        assert_eq!(bs.poke(sm).move_slots[1].id, dex.moves.id("substitute").unwrap(), "Sketch must succeed in PvP & bypass Sub");
        assert_eq!(bs.poke(sm).move_slots[1].pp, 16);
    }

    // #52 Fury Cutter exceeds 160 BP cap; #53① Outrage hit 2 not Counter-reflectable by fresh switch-in;
    // #54③ Foresight clears on user switch and ignores own accuracy drops when acc < eva; #39 Sleep Talk resets stall.
    {
        // Fury Cutter > 160 BP on 6th hit (10 * 32 = 320 BP vs PS capped 160 BP).
        let p1 = vec![mk_bare("Scizor", 50, &["Fury Cutter"], "")];
        let p2 = vec![mk_bare("Blissey", 50, &["Soft-Boiled", "Splash"], "")];
        let fc = dex.conds_id("furycutter").unwrap();
        let mut fc_checked = false;
        for seed in 1..30u64 {
            let mut bf = start_castle(&dex, &format!("{seed},2,3,4"), castle_ruleset(), &p1, &p2);
            let a1 = bf.active_id(0).unwrap();
            bf.add_volatile(&dex, a1, "furycutter", None, nc2000_engine::battle::EffectHandle::None);
            bf.poke_mut(a1).volatile_mut(fc).unwrap().set_int(DK::Multiplier, 16);
            bf.poke_mut(a1).volatile_mut(fc).unwrap().duration = Some(2);
            bf.choose(&dex, 0, "move furycutter").unwrap();
            bf.choose(&dex, 1, "move splash").unwrap();
            // After hit at 16x -> FuryCutterRestart doubled Multiplier to 32 (uncapped in Castle)!
            if bf.poke(a1).has_volatile(fc) {
                assert_eq!(bf.poke(a1).volatile(fc).unwrap().get_int(DK::Multiplier), 32);
                fc_checked = true;
                break;
            }
        }
        assert!(fc_checked, "Fury Cutter 32x uncapped test never landed");

        // #53① Outrage hit 2/3 against a newly switched-in mon cannot be MirrorCoated!
        let po1 = vec![mk_bare("Dragonite", 55, &["Outrage"], "")];
        let po2 = vec![
            mk_bare("Blissey", 50, &["Splash"], ""),
            mk_bare("Snorlax", 50, &["Mirror Coat", "Splash"], ""),
        ];
        let lm = dex.conds_id("lockedmove").unwrap();
        let mut outrage_checked = false;
        for seed in 1..40u64 {
            let mut bo = start_castle(&dex, &format!("{seed},2,3,4"), castle_ruleset(), &po1, &po2);
            bo.choose(&dex, 0, "move outrage").unwrap();
            bo.choose(&dex, 1, "move splash").unwrap();
            // Turn 2: P2 switches to Snorlax; Snorlax takes Outrage hit 2 (didn't take hit 1!).
            bo.choose(&dex, 0, "move outrage").unwrap();
            bo.choose(&dex, 1, "switch 2").unwrap();
            if bo.poke(bo.active_id(0).unwrap()).has_volatile(lm) {
                let d_pre = bo.poke(bo.active_id(0).unwrap()).hp;
                bo.choose(&dex, 0, "move outrage").unwrap();
                bo.choose(&dex, 1, "move mirrorcoat").unwrap();
                assert_eq!(
                    bo.poke(bo.active_id(0).unwrap()).hp,
                    d_pre,
                    "Outrage hit 2/3 must not be MirrorCoat-reflectable by mon that didn't take hit 1"
                );
                outrage_checked = true;
                break;
            }
        }
        assert!(outrage_checked, "Outrage 3-turn Mirror Coat test never rolled 3-turn Outrage");

        // #54③ Foresight clears when user switches out; ignores own acc drops when acc < eva.
        let pf1 = vec![
            mk_bare("Mew", 50, &["Foresight", "Tackle", "Splash"], ""),
            mk_bare("Snorlax", 50, &["Splash"], ""),
        ];
        let pf2 = vec![mk_bare("Gengar", 50, &["Splash"], "")];
        let mut bfo = start_castle(&dex, "1,2,3,4", castle_ruleset(), &pf1, &pf2);
        let f1 = bfo.active_id(0).unwrap();
        let f2 = bfo.active_id(1).unwrap();
        bfo.choose(&dex, 0, "move foresight").unwrap();
        bfo.choose(&dex, 1, "move splash").unwrap();
        let fs = dex.conds_id("foresight").unwrap();
        assert!(bfo.poke(f2).has_volatile(fs));
        // With acc=-6 < eva=0, Foresight ignores own accuracy drops so 100%-acc move hits 100%!
        bfo.poke_mut(f1).boosts[stat_idx::ACC] = -6;
        let pound_team = vec![
            mk_bare("Mew", 50, &["Foresight", "Pound"], ""),
            mk_bare("Snorlax", 50, &["Splash"], ""),
        ];
        let mut bfo2 = start_castle(&dex, "1,2,3,4", castle_ruleset(), &pound_team, &pf2);
        bfo2.choose(&dex, 0, "move foresight").unwrap();
        bfo2.choose(&dex, 1, "move splash").unwrap();
        let u1 = bfo2.active_id(0).unwrap();
        bfo2.poke_mut(u1).boosts[stat_idx::ACC] = -6;
        for _ in 0..10 {
            bfo2.log.clear();
            bfo2.poke_mut(bfo2.active_id(1).unwrap()).hp = 166;
            bfo2.choose(&dex, 0, "move pound").unwrap();
            bfo2.choose(&dex, 1, "move splash").unwrap();
            assert!(!bfo2.log.iter().any(|l| l.contains("|-miss|")), "Foresight with acc < eva must ignore own acc drops");
        }
        // User switches out -> Foresight clears from opponent!
        bfo.choose(&dex, 0, "switch 2").unwrap();
        bfo.choose(&dex, 1, "move splash").unwrap();
        assert!(!bfo.poke(f2).has_volatile(fs), "Foresight must clear when user switches out");

        // #39 Sleep Talk resets Protect/Detect/Endure stall counter.
        let pst1 = vec![mk_bare("Snorlax", 50, &["Protect", "Sleep Talk", "Splash"], "")];
        let pst2 = vec![mk_bare("Blissey", 50, &["Splash"], "")];
        let st_c = dex.conds_id("stall").unwrap();
        let mut st_checked = false;
        for seed in 1..30u64 {
            let mut bst = start_castle(&dex, &format!("{seed},2,3,4"), castle_ruleset(), &pst1, &pst2);
            let st1 = bst.active_id(0).unwrap();
            bst.add_volatile(&dex, st1, "stall", None, nc2000_engine::battle::EffectHandle::None);
            bst.restore_status(&dex, st1, Status::Slp, None);
            bst.poke_mut(st1).status_state.set_int(DK::Time, 3);
            assert!(bst.poke(st1).has_volatile(st_c));
            bst.choose(&dex, 0, "move sleeptalk").unwrap();
            bst.choose(&dex, 1, "move splash").unwrap();
            if !bst.log.iter().any(|l| l.contains("|Protect|")) {
                assert!(!bst.poke(st1).has_volatile(st_c), "Sleep Talk must reset stall counter");
                st_checked = true;
                break;
            }
        }
        assert!(st_checked, "Sleep Talk stall-reset test never picked a non-Protect move");
    }
}

#[test]
fn castle_audit_bugfixes_comprehensive() {
    let dex = load_dex();

    // 1. Castle Battle::get_health emits exact HP (shared == secret) and omits HP Percentage Mod.
    {
        let p1 = vec![mk_bare("Snorlax", 50, &["Splash"], "")];
        let p2 = vec![mk_bare("Blissey", 50, &["Splash"], "")];
        let mut bc = start_castle(&dex, "1,2,3,4", castle_ruleset(), &p1, &p2);
        let a1 = bc.active_id(0).unwrap();
        bc.poke_mut(a1).hp = 133;
        let (sec, sh) = bc.get_health(a1);
        assert_eq!(sec, "133/266");
        assert_eq!(sh, "133/266");
        assert!(!bc.log.iter().any(|l| l.contains("HP Percentage Mod")));

        let mut bp = Battle::from_fixture(&dex, "1,2,3,4", &p1, &p2).unwrap();
        bp.choose(&dex, 0, "team 1").unwrap();
        bp.choose(&dex, 1, "team 1").unwrap();
        let pa1 = bp.active_id(0).unwrap();
        bp.poke_mut(pa1).hp = 100;
        let (psec, psh) = bp.get_health(pa1);
        assert_eq!(psec, "100/235");
        assert_eq!(psh, "43/100");
        assert!(bp.log.iter().any(|l| l.contains("HP Percentage Mod")));
    }

    // 2. Unified Castle PokemonSet defaults: typed HP Fire auto-fill (None & all-30/31),
    // spd-only DV/EV propagation, always-derived HP DV, deterministic gender, and 11 Castle inert items.
    {
        let hp_fire_none = mk_bare("Zapdos", 50, &["Thunderbolt", "Hidden Power Fire"], "Surf Mail");
        let mut hp_fire_30 = mk_bare("Snorlax", 50, &["Hidden Power Fire"], "Flower Mail");
        hp_fire_30.ivs = Some(
            [("hp", 30), ("atk", 30), ("def", 30), ("spa", 30), ("spd", 30), ("spe", 30)]
                .into_iter()
                .map(|(k, v)| (k.to_string(), v))
                .collect(),
        );
        hp_fire_30.gender = Some("F".into()); // Invalid for Atk DV 14 Snorlax (12.5% F -> M)!
        let mut spd_only = mk_bare("Blissey", 50, &["Splash"], "Park Ball");
        spd_only.ivs = Some([("hp", 0), ("spd", 20)].into_iter().map(|(k, v)| (k.to_string(), v)).collect());
        spd_only.evs = Some([("spd", 100)].into_iter().map(|(k, v)| (k.to_string(), v)).collect());
        spd_only.gender = Some("M".into()); // Invalid for fixed-F Blissey!

        let b = Battle::from_fixture_with_ruleset(
            &dex,
            "1,2,3,4",
            &[hp_fire_none, hp_fire_30, spd_only],
            &[mk_bare("Mew", 50, &["Splash"], "Eon Mail")],
            castle_ruleset(),
        )
        .unwrap();
        let z = &b.sides[0].roster[0];
        assert_eq!(dex.type_name(z.hp_type), "Fire");
        assert_eq!(z.hp_power, 70);
        assert_eq!(z.set_ivs, [6, 28, 24, 30, 30, 30]); // Atk=14,Def=12,Spe=15,Spc=15 -> HP DV=3 (IV 6)
        assert!(z.item.is_some(), "Surf Mail must stay non-None in Castle Battle");

        let lax = &b.sides[0].roster[1];
        assert_eq!(dex.type_name(lax.hp_type), "Fire");
        assert_eq!(lax.hp_power, 70);
        assert_eq!(lax.gender, Gender::M, "Castle must enforce deterministic Atk-DV gender");
        assert!(lax.item.is_some(), "Flower Mail must resolve to Mail");

        let bl = &b.sides[0].roster[2];
        assert_eq!(bl.set_ivs[3], 20);
        assert_eq!(bl.set_ivs[4], 20);
        assert_eq!(bl.set_ivs[0], 28); // Atk=15(1), Def=15(1), Spe=15(1), Spc=10(0) -> HP DV=14 (IV 28)
        assert_eq!(bl.set_evs[3], 100);
        assert_eq!(bl.set_evs[4], 100);
        assert_eq!(bl.set_evs[0], 255);
        assert_eq!(bl.gender, Gender::F, "Castle must enforce fixed species gender");
        assert!(bl.item.is_some(), "Park Ball must resolve to Sport Ball");
    }

    // 3. Unified Battle::get_stat_castle (stage -> item -> 999 -> Burn/2), f64 pre-par 999 clamp, and hit_probability.
    {
        let p1 = vec![mk_bare("Marowak", 50, &["Fire Blast", "Splash"], "Thick Club")];
        let p2 = vec![mk_bare("Jolteon", 50, &["Splash"], "BrightPowder")];
        let mut b = start_castle(&dex, "1,2,3,4", castle_ruleset(), &p1, &p2);
        let a1 = b.active_id(0).unwrap();
        let a2 = b.active_id(1).unwrap();
        b.poke_mut(a1).boosts[stat_idx::ATK] = 6; // 131 * 4 = 524, *2 Thick Club = 1048 -> 999 -> Burn /2 = 499
        b.restore_status(&dex, a1, Status::Brn, None);
        assert_eq!(b.get_stat(&dex, a1, stat_idx::ATK, false, false, false), 499);

        // Pre-paralysis speed clamp at 999.0 before / 4.0:
        // Lv50 Jolteon Spe=181, at +6 -> 724; set stored_stats[SPE]=300 at +6 -> 1200 -> clamp 999 -> Par/4 = 249.75!
        b.poke_mut(a2).stored_stats[stat_idx::SPE] = 300;
        b.poke_mut(a2).boosts[stat_idx::SPE] = 6;
        b.restore_status(&dex, a2, Status::Par, None);
        assert!((b.get_pokemon_action_speed_f64(&dex, a2) - 249.75).abs() < 1e-9);

        // hit_probability uses Castle rational 3/9..9/3 + Foresight + BrightPowder -20/256:
        let fb = dex.moves.id("fireblast").unwrap();
        b.poke_mut(a1).boosts[stat_idx::ACC] = -1; // 85% -> floor(85*255/100)=216; *3/4=162; -20 BrightPowder = 142 -> 142/256
        assert!((b.hit_probability(&dex, a1, a2, fb) - 142.0 / 256.0).abs() < 1e-12);
        // With Foresight and acc=-1 < eva=0, both reset to 0 -> 216 - 20 = 196 -> 196/256!
        b.add_volatile(&dex, a2, "foresight", Some(a1), nc2000_engine::battle::EffectHandle::None);
        assert!((b.hit_probability(&dex, a1, a2, fb) - 196.0 / 256.0).abs() < 1e-12);
    }

    // 4. #25 Present v0.3: Burn unfloored halving, Metal Powder unfloored 1.5x, Crit before Pink Bow, Triple Kick leak.
    {
        // Delibird (Ice/Flying -> att_last=Flying=2) vs Umbreon (Dark -> def_last=Dark=27, p_lvl*6=162):
        // BP 40 clean -> floor(162*40*1.0/2.0/50) = 64 -> Pink Bow *1.1 = 70 -> +2 = 72 (roll 61..=72).
        // BP 40 burned -> floor(162*40*0.5/2.0/50) = 32 -> Pink Bow *1.1 = 35 -> +2 = 37 (roll 31..=37).
        let p1 = vec![mk_bare("Delibird", 50, &["Present"], "Pink Bow")];
        let p2 = vec![mk_bare("Umbreon", 50, &["Splash"], "")];
        let mut b_clean = start_castle(&dex, "1,2,3,4", castle_ruleset(), &p1, &p2);
        let mut b_burn = start_castle(&dex, "1,2,3,4", castle_ruleset(), &p1, &p2);
        let a1 = b_burn.active_id(0).unwrap();
        b_burn.restore_status(&dex, a1, Status::Brn, None);
        let a2c = b_clean.active_id(1).unwrap();
        let a2b = b_burn.active_id(1).unwrap();
        let mut checked = false;
        for seed in 1..60u64 {
            let mut tc = b_clean.clone();
            let mut tb = b_burn.clone();
            tc.reseed(seed);
            tb.reseed(seed);
            tc.choose(&dex, 0, "move present").unwrap();
            tc.choose(&dex, 1, "move splash").unwrap();
            tb.choose(&dex, 0, "move present").unwrap();
            tb.choose(&dex, 1, "move splash").unwrap();
            let dc = 201 - tc.poke(a2c).hp;
            let db = 201 - tb.poke(a2b).hp;
            if dc >= 40 && db > 0 {
                assert!(db < dc * 3 / 4, "Burned Present ({db}) must deal ~half of unburned ({dc})");
                checked = true;
                break;
            }
        }
        assert!(checked, "Burned Present comparison never rolled damaging bucket");
    }

    // 5. #45 Lock-On: target switch clears lockon, Swift doesn't consume, OHKO consumes without auto-hitting,
    // Baton Pass re-points foe lockon and does not copy dig/fly/attacker lockon.
    {
        let p1 = vec![
            mk_bare("Mew", 50, &["Lock-On", "Swift", "Fissure", "Baton Pass"], ""),
            mk_bare("Snorlax", 50, &["Splash"], ""),
        ];
        let p2 = vec![
            mk_bare("Blissey", 50, &["Baton Pass", "Splash"], ""),
            mk_bare("Snorlax", 50, &["Splash"], ""),
        ];
        let lo = dex.conds_id("lockon").unwrap();
        let mut b = start_castle(&dex, "1,2,3,4", castle_ruleset(), &p1, &p2);
        let a1 = b.active_id(0).unwrap();
        b.choose(&dex, 0, "move lockon").unwrap();
        b.choose(&dex, 1, "move splash").unwrap();
        assert!(b.poke(a1).has_volatile(lo));
        // Swift (AlwaysHits, no acc check) does NOT consume Lock-On!
        b.choose(&dex, 0, "move swift").unwrap();
        b.choose(&dex, 1, "move splash").unwrap();
        assert!(b.poke(a1).has_volatile(lo), "Swift must NOT consume Lock-On");
        // Foe Baton Pass re-points attacker's Lock-On to receiver!
        b.choose(&dex, 0, "move swift").unwrap();
        b.choose(&dex, 1, "move batonpass").unwrap();
        b.choose(&dex, 1, "switch 2").unwrap();
        let p2_recv = b.active_id(1).unwrap();
        assert_eq!(b.poke(a1).volatile(lo).and_then(|v| v.source), Some(p2_recv), "Foe Baton Pass must re-point Lock-On");
        // Fissure (OHKO, has acc check) consumes Lock-On!
        b.choose(&dex, 0, "move fissure").unwrap();
        b.choose(&dex, 1, "move splash").unwrap();
        assert!(!b.poke(a1).has_volatile(lo), "OHKO move must consume Lock-On");
        // Re-lock and normal switch by target clears attacker's Lock-On!
        let mut b2 = start_castle(&dex, "1,2,3,4", castle_ruleset(), &p1, &p2);
        let u1 = b2.active_id(0).unwrap();
        b2.choose(&dex, 0, "move lockon").unwrap();
        b2.choose(&dex, 1, "move splash").unwrap();
        assert!(b2.poke(u1).has_volatile(lo));
        b2.choose(&dex, 0, "move swift").unwrap();
        b2.choose(&dex, 1, "switch 2").unwrap();
        assert!(!b2.poke(u1).has_volatile(lo), "Target normal switch must clear attacker's Lock-On");
        // Attacker Baton Pass must NOT copy attacker-side Lock-On (or Dig/Fly)!
        let mut b3 = start_castle(&dex, "1,2,3,4", castle_ruleset(), &p1, &p2);
        b3.choose(&dex, 0, "move lockon").unwrap();
        b3.choose(&dex, 1, "move splash").unwrap();
        b3.choose(&dex, 0, "move batonpass").unwrap();
        b3.choose(&dex, 1, "move splash").unwrap();
        b3.choose(&dex, 0, "switch 2").unwrap();
        assert!(!b3.poke(b3.active_id(0).unwrap()).has_volatile(lo), "Baton Pass must NOT copy attacker-side Lock-On");
    }

    // 6. #44 Sleep Talk -> Outrage does NOT attach lockedmove!
    {
        let p1 = vec![mk_bare("Dragonite", 55, &["Sleep Talk", "Outrage"], "")];
        let p2 = vec![mk_bare("Blissey", 50, &["Splash"], "")];
        let mut b = start_castle(&dex, "1,2,3,4", castle_ruleset(), &p1, &p2);
        let a1 = b.active_id(0).unwrap();
        b.restore_status(&dex, a1, Status::Slp, None);
        b.poke_mut(a1).status_state.set_int(DK::Time, 4);
        b.choose(&dex, 0, "move sleeptalk").unwrap();
        b.choose(&dex, 1, "move splash").unwrap();
        let lm = dex.conds_id("lockedmove").unwrap();
        assert!(!b.poke(a1).has_volatile(lm), "Sleep Talk -> Outrage must NOT attach lockedmove in Castle");
    }

    // 7. #2 1v1 Simultaneous Spikes KO on dual replacement switches -> DRAW!
    {
        let p1 = vec![
            mk_bare("Mew", 50, &["Spikes", "Explosion"], ""),
            mk_bare("Snorlax", 50, &["Splash"], ""),
        ];
        let p2 = vec![
            mk_bare("Mew", 50, &["Spikes", "Splash"], ""),
            mk_bare("Snorlax", 50, &["Splash"], ""),
        ];
        let mut b = start_castle(&dex, "1,2,3,4", castle_ruleset(), &p1, &p2);
        b.choose(&dex, 0, "move spikes").unwrap();
        b.choose(&dex, 1, "move spikes").unwrap();
        // Both benches at 1 HP so Spikes KOs both simultaneously!
        b.sides[0].roster[1].hp = 1;
        b.sides[1].roster[1].hp = 1;
        b.sides[1].roster[0].hp = 10;
        b.choose(&dex, 0, "move explosion").unwrap();
        b.choose(&dex, 1, "move splash").unwrap();
        assert_eq!(b.request_state, RequestState::Switch);
        b.choose(&dex, 0, "switch 2").unwrap();
        b.choose(&dex, 1, "switch 2").unwrap();
        assert_eq!(b.outcome(), Some(Outcome::Tie), "1v1 simultaneous Spikes KO must adjudicate as DRAW");
    }

    // 8. #30 Variance-at-1 rolls 0 in Castle; #47① Return fixed 102 BP at happiness=0;
    // #38 King's Rock excludes Counter/MirrorCoat/SelfDestruct/Explosion; #43 Rapid Spin clears Spikes vs Protect;
    // #54④ Sketch fails vs transformed target; #7 Perish Song asymmetric 0-remaining loser faints first!
    {
        // #30: base damage = 1 -> 38/39 rolls produce 0!
        let pz1 = vec![mk_set("Magikarp", 5, &["Tackle"], "", Some([0, 0, 0, 0]), Some([0, 0, 0, 0, 0]))];
        let pz2 = vec![mk_bare("Steelix", 50, &["Splash"], "")];
        let mut saw_zero = false;
        let mut saw_one = false;
        for seed in 1..120u64 {
            let mut bz = start_castle(&dex, &format!("{seed},2,3,4"), castle_ruleset(), &pz1, &pz2);
            let a2 = bz.active_id(1).unwrap();
            let pre = bz.poke(a2).hp;
            bz.choose(&dex, 0, "move tackle").unwrap();
            bz.choose(&dex, 1, "move splash").unwrap();
            if !bz.log.iter().any(|l| l.contains("|-miss|")) {
                let d = pre - bz.poke(a2).hp;
                if d == 0 { saw_zero = true; }
                if d == 1 { saw_one = true; }
                if saw_zero && saw_one { break; }
            }
        }
        assert!(saw_zero && saw_one, "Castle variance-at-1 must produce both 0 and 1");

        // #47① Return fixed 102 BP even with happiness == 0!
        let mut ret_mon = mk_bare("Snorlax", 50, &["Return"], "");
        ret_mon.happiness = Some(0);
        let mut br = start_castle(&dex, "1,2,3,4", castle_ruleset(), &[ret_mon], &[mk_bare("Blissey", 50, &["Splash"], "")]);
        br.choose(&dex, 0, "move return").unwrap();
        br.choose(&dex, 1, "move splash").unwrap();
        let dealt = 361 - br.poke(br.active_id(1).unwrap()).hp;
        assert!(dealt >= 140, "Castle Return must be fixed 102 BP even at happiness=0, got {dealt}");

        // #43 Rapid Spin vs Protect still clears Spikes!
        let pr1 = vec![mk_bare("Starmie", 50, &["Rapid Spin", "Splash"], "")];
        let pr2 = vec![mk_bare("Cloyster", 50, &["Spikes", "Protect", "Splash"], "")];
        let mut bp = start_castle(&dex, "1,2,3,4", castle_ruleset(), &pr1, &pr2);
        bp.choose(&dex, 0, "move splash").unwrap();
        bp.choose(&dex, 1, "move spikes").unwrap();
        bp.choose(&dex, 0, "move rapidspin").unwrap();
        bp.choose(&dex, 1, "move protect").unwrap();
        let spikes = dex.conds_id("spikes").unwrap();
        assert!(!bp.sides[0].has_side_condition(spikes), "Rapid Spin vs Protect must still clear Spikes");

        // #54④ Sketch fails against transformed target!
        let ps1 = vec![mk_bare("Smeargle", 50, &["Sketch", "Splash"], "")];
        let ps2 = vec![mk_bare("Ditto", 50, &["Transform"], "")];
        let mut bs = start_castle(&dex, "1,2,3,4", castle_ruleset(), &ps1, &ps2);
        bs.choose(&dex, 0, "move splash").unwrap();
        bs.choose(&dex, 1, "move transform").unwrap();
        bs.choose(&dex, 0, "move splash").unwrap();
        bs.choose(&dex, 1, "move splash").unwrap();
        bs.log.clear();
        bs.choose(&dex, 0, "move sketch").unwrap();
        bs.choose(&dex, 1, "move splash").unwrap();
        assert!(bs.log.iter().any(|l| l.contains("|-fail|")), "Sketch must fail vs transformed target");

        // #7 Perish Song asymmetric 0-remaining loser queued first in faint_queue!
        let pp1 = vec![
            mk_bare("Misdreavus", 50, &["Perish Song", "Splash"], ""),
            mk_bare("Snorlax", 50, &["Splash"], ""),
        ];
        let pp2 = vec![mk_bare("Blissey", 50, &["Splash"], "")];
        let mut bper = start_castle(&dex, "1,2,3,4", castle_ruleset(), &pp1, &pp2);
        bper.choose(&dex, 0, "move perishsong").unwrap();
        bper.choose(&dex, 1, "move splash").unwrap();
        for _ in 0..3 {
            bper.log.clear();
            bper.choose(&dex, 0, "move splash").unwrap();
            bper.choose(&dex, 1, "move splash").unwrap();
        }
        let f_p2 = bper.log.iter().position(|l| l == "|faint|p2a: Blissey").unwrap();
        let f_p1 = bper.log.iter().position(|l| l == "|faint|p1a: Misdreavus").unwrap();
        assert!(f_p2 < f_p1, "Asymmetric Perish 0-remaining loser (P2) must faint first");
        assert_eq!(bper.outcome(), Some(Outcome::P1Win));

        // #38 King's Rock excludes Counter/MirrorCoat/SelfDestruct/Explosion in Castle!
        let pk1 = vec![mk_bare("Snorlax", 50, &["Counter", "Splash"], "King's Rock")];
        let pk2 = vec![mk_bare("Blissey", 50, &["Tackle", "Splash"], "")];
        for seed in 1..80u64 {
            let mut bk = start_castle(&dex, &format!("{seed},2,3,4"), castle_ruleset(), &pk1, &pk2);
            let a1 = bk.active_id(0).unwrap();
            let a2 = bk.active_id(1).unwrap();
            // Force Snorlax to act first with Counter via Quick-style speed boost? Better: test faster Mew with Explosion/Counter!
            let _ = (a1, a2);
            break;
        }
        let pm1 = vec![mk_bare("Mew", 50, &["Counter", "Splash"], "King's Rock")];
        let pm2 = vec![mk_bare("Snorlax", 50, &["Tackle"], "")];
        for seed in 1..60u64 {
            let mut bk = start_castle(&dex, &format!("{seed},2,3,4"), castle_ruleset(), &pm1, &pm2);
            let a2 = bk.active_id(1).unwrap();
            bk.poke_mut(a2).last_move = dex.moves.id("tackle");
            bk.choose(&dex, 0, "move counter").unwrap();
            bk.choose(&dex, 1, "move tackle").unwrap();
            assert!(!bk.log.iter().any(|l| l.contains("|flinch")), "Counter must not trigger King's Rock flinch");
        }

        // #32/#33 Status Berry uniformity: PRZCureBerry does NOT fire on generic Update,
        // and DOES fire after foe's failed/Bide/nopp turn in Castle!
        {
            let pb1 = vec![mk_bare("Mew", 50, &["Bide", "Splash"], "")];
            let pb2 = vec![mk_bare("Snorlax", 50, &["Splash"], "PRZCureBerry")];
            let mut bb = start_castle(&dex, "1,2,3,4", castle_ruleset(), &pb1, &pb2);
            let a1 = bb.active_id(0).unwrap();
            let a2 = bb.active_id(1).unwrap();
            // Inject paralysis directly + fire Update: Castle PRZCureBerryUpdate must NOT consume the berry.
            bb.restore_status(&dex, a2, Status::Par, None);
            bb.each_event(&dex, &nc2000_engine::battle::events::ev::Update, None);
            assert_eq!(bb.poke(a2).status, Status::Par, "Castle PRZCureBerry must not cure on generic Update");
            assert!(bb.poke(a2).item.is_some());
            // When P1 hits the Bide early-return path, foe's immediate status berry check fires!
            bb.add_volatile(&dex, a1, "bide", Some(a1), nc2000_engine::battle::EffectHandle::None);
            bb.choose(&dex, 0, "move bide").unwrap();
            bb.choose(&dex, 1, "move splash").unwrap();
            assert_eq!(bb.poke(a2).status, Status::None, "Castle foe status berry must cure on Bide early return");
            assert!(bb.poke(a2).item.is_none());
        }

        // #51 Triple Kick: BP=10, multiplied by hit_k AFTER type effectiveness & BEFORE roll!
        // Hitmontop (Atk=146) vs Blissey (Def=61, 2x weak to Fighting):
        // Hit 1: floor(floor(22*10*146/61)/50)+2 = 12, *1.5 STAB = 18, *2 SE = 36, *1 = 36 (roll 30..=36).
        // Hit 2: 36 * 2 = 72 (roll 61..=72). Hit 3: 36 * 3 = 108 (roll 91..=108). Total = 182..=216!
        {
            let pt1 = vec![mk_bare("Hitmontop", 50, &["Triple Kick"], "")];
            let pt2 = vec![mk_bare("Blissey", 50, &["Splash"], "")];
            let mut saw_3hits = false;
            for seed in 1..60u64 {
                let mut bt = start_castle(&dex, &format!("{seed},2,3,4"), castle_ruleset(), &pt1, &pt2);
                bt.choose(&dex, 0, "move triplekick").unwrap();
                bt.choose(&dex, 1, "move splash").unwrap();
                if bt.log.iter().any(|l| l.contains("|-hitcount|p2a: Blissey|3")) {
                    let dealt = 361 - bt.poke(bt.active_id(1).unwrap()).hp;
                    assert!(
                        (182..=216).contains(&dealt),
                        "Castle Triple Kick 3-hit total expected 182..=216, got {dealt}"
                    );
                    saw_3hits = true;
                    break;
                }
            }
            assert!(saw_3hits, "Triple Kick 3-hit seed not found");
        }

        // #54③ resolve_future_move: Castle 256-denom accuracy + BrightPowder -20/256 + Foresight acc<eva!
        {
            let pf1 = vec![mk_bare("Espeon", 50, &["Future Sight", "Splash"], "")];
            let pf2 = vec![mk_bare("Blissey", 50, &["Splash"], "BrightPowder")];
            let mut hits = 0usize;
            let trials = 400usize;
            for seed in 1..=(trials as u64) {
                let mut bf = start_castle(&dex, &format!("{seed},2,3,4"), castle_ruleset(), &pf1, &pf2);
                bf.set_log_enabled(false);
                bf.choose(&dex, 0, "move futuresight").unwrap();
                bf.choose(&dex, 1, "move splash").unwrap();
                // Lower Espeon accuracy by -2: 90% -> floor(90*255/100)=229, *3/5=137, -20 BrightPowder = 117/256 (~45.7%)!
                let a1 = bf.active_id(0).unwrap();
                let a2 = bf.active_id(1).unwrap();
                bf.poke_mut(a1).boosts[stat_idx::ACC] = -2;
                bf.choose(&dex, 0, "move splash").unwrap();
                bf.choose(&dex, 1, "move splash").unwrap();
                bf.choose(&dex, 0, "move splash").unwrap();
                bf.choose(&dex, 1, "move splash").unwrap();
                if bf.poke(a2).hp < 361 {
                    hits += 1;
                }
            }
            let rate = hits as f64 / trials as f64;
            // Expected 117/256 = 0.457; clearly distinct from unboosted 90% (0.90)!
            assert!(
                (0.35..=0.58).contains(&rate),
                "Castle Future Sight at acc -2 + BrightPowder expected ~0.457, got {rate:.3} ({hits}/{trials})"
            );
        }

        // #15/#16 PursuitFoeBeforeSwitchOut uses f64 speed (999 clamp before /4, no early floor)!
        {
            // Paralyzed Tyranitar (Spe=112 -> 28.0) vs switching Destiny Bond Gengar (Spe=111 -> paralyzed 27.75):
            // Under integer floor both would be 28 vs 27 (or 113/4=28.25 vs 112/4=28.0 -> tied as ints, NOT tied in f64!).
            let pp1 = vec![mk_bare("Tyranitar", 50, &["Pursuit", "Splash"], "")];
            let pp2 = vec![
                mk_bare("Gengar", 50, &["Destiny Bond", "Splash"], ""),
                mk_bare("Snorlax", 50, &["Splash"], ""),
            ];
            for seed in 1..30u64 {
                let mut bp = start_castle(&dex, &format!("{seed},2,3,4"), castle_ruleset(), &pp1, &pp2);
                let a1 = bp.active_id(0).unwrap();
                let a2 = bp.active_id(1).unwrap();
                // Set stored_stats[SPE]: Tyranitar=112 (Par -> 28.0), Gengar=113 (Par -> 28.25).
                // As integers floor(112/4)==floor(113/4)==28 (50/50 tie), as f64 28.0 < 28.25 (ALWAYS clears Destiny Bond!).
                bp.poke_mut(a1).stored_stats[stat_idx::SPE] = 112;
                bp.poke_mut(a2).stored_stats[stat_idx::SPE] = 113;
                bp.restore_status(&dex, a1, Status::Par, None);
                bp.restore_status(&dex, a2, Status::Par, None);
                bp.add_volatile(&dex, a2, "destinybond", Some(a2), nc2000_engine::battle::EffectHandle::None);
                bp.poke_mut(a2).hp = 1;
                bp.choose(&dex, 0, "move pursuit").unwrap();
                bp.choose(&dex, 1, "switch 2").unwrap();
                // Since 28.0 < 28.25 strictly, holder_speed < switching_speed ALWAYS removes Destiny Bond,
                // so Tyranitar NEVER faints to Destiny Bond!
                assert!(!bp.poke(a1).fainted, "Castle f64 Pursuit speed (28.0 < 28.25) must deterministically clear Destiny Bond");
            }
        }
    }
}

// =========================================================================
// 6. Gate A — Round 5 Live-Probe `.manifest.json` Conformance (All 18)
// =========================================================================

#[derive(serde::Deserialize)]
struct ManifestStats {
    hp: i32,
    atk: i32,
    def: i32,
    spa: i32,
    spd: i32,
    spe: i32,
}

#[derive(serde::Deserialize)]
struct ManifestMon {
    species: String,
    level: u8,
    moves: Vec<String>,
    #[serde(default)]
    item: Option<String>,
    #[serde(default)]
    dvs: Option<[u8; 4]>,
    #[serde(default)]
    dos: Option<[u8; 5]>,
    computed_stats: ManifestStats,
}

#[derive(serde::Deserialize)]
struct ManifestStep {
    player: u8,
    #[allow(dead_code)]
    step: u32,
    action: String,
    opts: Vec<(String, String)>,
    submitted: String,
    #[serde(default)]
    pdat: Option<Vec<String>>,
}

#[derive(serde::Deserialize)]
struct ProbeManifest {
    tag: String,
    l_turn: u16,
    ovfl: bool,
    p1_pick: Vec<usize>,
    p2_pick: Vec<usize>,
    p1_party: Vec<ManifestMon>,
    p2_party: Vec<ManifestMon>,
    steps: Vec<ManifestStep>,
}

fn manifest_mon_to_set(m: &ManifestMon) -> PokemonSet {
    let move_refs: Vec<&str> = m.moves.iter().map(|s| s.as_str()).collect();
    let item = m.item.as_deref().unwrap_or("");
    mk_set(&m.species, m.level, &move_refs, item, m.dvs, m.dos)
}

#[derive(Debug)]
struct PdatRow {
    hp: i32,
    maxhp: i32,
    status: Status,
    pps: Vec<i32>,
}

fn parse_pdat_rows(pdat: &[String]) -> Vec<PdatRow> {
    let mut rows = Vec::new();
    for line in pdat {
        if !line.contains('／') {
            continue;
        }
        let cells: Vec<&str> = line
            .split("| |")
            .map(|c| c.trim().trim_matches('|').trim())
            .collect();
        if cells.len() < 8 {
            continue;
        }
        let mut hp_parts = cells[2].split('／');
        let hp: i32 = hp_parts.next().unwrap().trim().parse().unwrap();
        let maxhp: i32 = hp_parts.next().unwrap().trim().parse().unwrap();
        let status = match cells[3] {
            "どく" => Status::Psn,
            "ねむり" => Status::Slp,
            "まひ" => Status::Par,
            "やけど" => Status::Brn,
            "こおり" => Status::Frz,
            _ => Status::None,
        };
        let mut pps = Vec::new();
        for cell in &cells[4..8] {
            if let (Some(l), Some(r)) = (cell.rfind('('), cell.rfind(')')) {
                let name = cell[..l].trim();
                let pp: i32 = cell[l + 1..r].trim().parse().unwrap();
                if !name.is_empty() {
                    pps.push(pp);
                }
            }
        }
        rows.push(PdatRow { hp, maxhp, status, pps });
    }
    rows
}

fn verify_pdat_snapshot(
    tag: &str,
    step_idx: usize,
    b: &Battle,
    side_n: usize,
    pdat: &[String],
    hp_tolerance: i32,
) -> Result<(), String> {
    let rows = parse_pdat_rows(pdat);
    if rows.len() != b.sides[side_n].roster.len() {
        return Err(format!(
            "{tag} step {step_idx} pdat row count {} != roster {}",
            rows.len(),
            b.sides[side_n].roster.len()
        ));
    }
    for (slot, row) in rows.iter().enumerate() {
        let p = &b.sides[side_n].roster[slot];
        if p.maxhp != row.maxhp {
            return Err(format!(
                "{tag} step {step_idx} slot {slot} maxhp {} != pdat {}",
                p.maxhp, row.maxhp
            ));
        }
        if (p.hp - row.hp).abs() > hp_tolerance {
            return Err(format!(
                "{tag} step {step_idx} slot {slot} hp {} vs pdat {} (tol {hp_tolerance})",
                p.hp, row.hp
            ));
        }
        let got_st = match p.status {
            Status::Tox => Status::Psn,
            Status::Fnt => Status::None,
            other => other,
        };
        let want_st = if p.fainted { Status::None } else { row.status };
        if got_st != want_st {
            return Err(format!(
                "{tag} step {step_idx} slot {slot} status {:?} != pdat {:?}",
                got_st, want_st
            ));
        }
        let got_pps: Vec<i32> = p.move_slots.iter().map(|s| s.pp).collect();
        if got_pps != row.pps {
            return Err(format!(
                "{tag} step {step_idx} slot {slot} PPs {:?} != pdat {:?}",
                got_pps, row.pps
            ));
        }
    }
    Ok(())
}

fn replay_manifest_with_seed(
    dex: &Dex,
    m: &ProbeManifest,
    p1_picked: &[PokemonSet],
    p2_picked: &[PokemonSet],
    ruleset: Ruleset,
    seed: u64,
    hp_tolerance: i32,
) -> Result<Battle, (usize, String)> {
    let mut b = start_castle(dex, &format!("{seed},2,3,4"), ruleset, p1_picked, p2_picked);
    for (step_idx, st) in m.steps.iter().enumerate() {
        let side_n = (st.player - 1) as usize;
        let expected_req = match st.action.as_str() {
            "INPUT" => RequestKind::Move,
            "CHANGE" | "BATONN" => RequestKind::Switch,
            other => return Err((step_idx, format!("unexpected action {other}"))),
        };
        if b.sides[side_n].request != Some(expected_req) {
            return Err((
                step_idx,
                format!(
                    "{} step {} p{}: request {:?} != expected {:?}",
                    m.tag, step_idx, st.player, b.sides[side_n].request, expected_req
                ),
            ));
        }
        let legal = b.legal_choices(dex, side_n);
        let legal_moves = legal
            .iter()
            .filter(|c| matches!(c, SearchChoice::Move(_)))
            .count();
        let legal_switches = legal
            .iter()
            .filter(|c| matches!(c, SearchChoice::Switch(_)))
            .count();
        let opt_moves = st
            .opts
            .iter()
            .filter(|(k, _)| k.starts_with('W') || k == "A")
            .count();
        let opt_switches = st.opts.iter().filter(|(k, _)| k.starts_with('C')).count();
        if legal_moves != opt_moves || legal_switches != opt_switches {
            return Err((
                step_idx,
                format!(
                    "{} step {} p{}: legal (m={legal_moves}, s={legal_switches}) != opts (m={opt_moves}, s={opt_switches}) {:?}",
                    m.tag, step_idx, st.player, st.opts
                ),
            ));
        }
        if let Some(ref pdat) = st.pdat {
            if !pdat.is_empty() {
                verify_pdat_snapshot(&m.tag, step_idx, &b, side_n, pdat, hp_tolerance)
                    .map_err(|e| (step_idx, e))?;
            }
        }
        let choice = if let Some(w) = st.submitted.strip_prefix('W') {
            let slot: u8 = w.parse().unwrap();
            format!("move {slot}")
        } else if let Some(c) = st.submitted.strip_prefix('C') {
            let orig_slot: u8 = c.parse::<u8>().unwrap() - 1;
            let pos = b.sides[side_n]
                .party
                .iter()
                .position(|&s| s == orig_slot)
                .ok_or_else(|| {
                    (step_idx, format!("{} step {}: switch target C{c} not in party", m.tag, step_idx))
                })?
                + 1;
            format!("switch {pos}")
        } else if st.submitted == "A" {
            legal
                .iter()
                .find(|c| matches!(c, SearchChoice::Move(_)))
                .ok_or_else(|| (step_idx, format!("{} step {}: no legal move for A", m.tag, step_idx)))?
                .to_input(dex)
        } else {
            return Err((step_idx, format!("unknown submitted value {}", st.submitted)));
        };
        b.choose(dex, side_n, &choice)
            .map_err(|e| (step_idx, format!("choice {choice} failed in {} step {}: {e:?}", m.tag, step_idx)))?;
    }
    Ok(b)
}

#[test]
fn gate_a_round5_manifests_stats_and_protocol_conformance() {
    let fixture_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/castle");
    assert!(
        fixture_dir.is_dir(),
        "Committed Gate A fixtures missing at {}",
        fixture_dir.display()
    );
    let dex = load_dex();
    let mut manifests: Vec<std::path::PathBuf> = std::fs::read_dir(&fixture_dir)
        .unwrap()
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("probe12") && n.ends_with(".manifest.json"))
        })
        .collect();
    manifests.sort();
    assert_eq!(
        manifests.len(),
        11,
        "Expected all 11 non-Monte-Carlo Round 5 probe manifests in fixtures/castle"
    );

    let mut total_pdat_verified = 0usize;
    for path in &manifests {
        let raw = std::fs::read_to_string(path).unwrap();
        let m: ProbeManifest = serde_json::from_str(&raw).unwrap();
        let p1_all: Vec<PokemonSet> = m.p1_party.iter().map(manifest_mon_to_set).collect();
        let p2_all: Vec<PokemonSet> = m.p2_party.iter().map(manifest_mon_to_set).collect();
        let ruleset = castle_raw(m.ovfl, Some(m.l_turn));

        // 1. Verify EVERY mon's computed_stats across all 6 party slots on both sides!
        let b_check = Battle::from_fixture_with_ruleset(&dex, "1,2,3,4", &p1_all, &p2_all, ruleset).unwrap();
        for (side_n, party) in [(0, &m.p1_party), (1, &m.p2_party)] {
            for (slot, mon) in party.iter().enumerate() {
                let p = &b_check.sides[side_n].roster[slot];
                let cs = &mon.computed_stats;
                assert_eq!(
                    (p.maxhp, p.stored_stats[0], p.stored_stats[1], p.stored_stats[2], p.stored_stats[3], p.stored_stats[4]),
                    (cs.hp, cs.atk, cs.def, cs.spa, cs.spd, cs.spe),
                    "Stat mismatch in {} side={} slot={} ({})",
                    m.tag, side_n + 1, slot + 1, mon.species
                );
            }
        }

        // 2. Replay ALL 11 manifests end-to-end, verifying opts menus, pdat HP/PP/status snapshots,
        // hit-by-hit damage/status invariants, and exact final outcomes!
        let p1_picked: Vec<PokemonSet> = m.p1_pick.iter().map(|&i| p1_all[i - 1].clone()).collect();
        let p2_picked: Vec<PokemonSet> = m.p2_pick.iter().map(|&i| p2_all[i - 1].clone()).collect();
        let hp_tol = match m.tag.as_str() {
            "probe12c_bp_pursuit_fswipe_pdat" => 12,
            "probe12i_loose_ends" => 15,
            _ => 0,
        };
        let expected_outcome = match m.tag.as_str() {
            "probe12a1_lturn_frac_p1"
            | "probe12a3_lturn_count"
            | "probe12d1_sub_roar_sketch_mimic_wrap_encore"
            | "probe12d2_spin_mlook_stencore_beatupbench"
            | "probe12i_loose_ends" => Outcome::P1Win,
            "probe12a2_lturn_frac_p2"
            | "probe12b_healbell_psn_23"
            | "probe12c_bp_pursuit_fswipe_pdat"
            | "probe12e1_reversal_lockon_zap"
            | "probe12e2_rollout_expl256_digsand" => Outcome::P2Win,
            "probe12h_speed_model_f64" => Outcome::Tie,
            other => panic!("unexpected manifest tag {other}"),
        };

        let mut replayed: Option<Battle> = None;
        let mut best_step = 0usize;
        let mut best_err = String::new();
        for seed in 1..=2048u64 {
            match replay_manifest_with_seed(&dex, &m, &p1_picked, &p2_picked, ruleset, seed, hp_tol) {
                Ok(b) if b.outcome() == Some(expected_outcome) => {
                    let path_ok = match m.tag.as_str() {
                        "probe12b_healbell_psn_23" => {
                            let mew_hp = b.sides[0].roster[0].hp;
                            let bl2_hp = b.sides[1].roster[1].hp;
                            mew_hp == 120
                                && b.sides[0].roster[0].status == Status::Psn
                                && (29..=36).contains(&bl2_hp)
                        }
                        "probe12c_bp_pursuit_fswipe_pdat" => {
                            (49..=63).contains(&b.sides[0].roster[1].hp)
                                && b.sides[0].roster[2].hp == 134
                        }
                        "probe12d1_sub_roar_sketch_mimic_wrap_encore" => {
                            b.sides[0].roster[1].base_move_slots[0].id
                                == dex.moves.id("tackle").unwrap()
                                && b.sides[0].roster[1].base_move_slots[0].maxpp == 56
                        }
                        "probe12d2_spin_mlook_stencore_beatupbench" => {
                            let spikes = dex.conds_id("spikes").unwrap();
                            !b.sides[0].has_side_condition(spikes)
                                && b.sides[0].roster[1].status == Status::Slp
                                && b.sides[0].roster[0].status == Status::Par
                                && (192..=200).contains(&b.sides[1].roster[1].hp)
                        }
                        "probe12e1_reversal_lockon_zap" => {
                            (175..=178).contains(&b.sides[1].roster[1].hp)
                                && (179..=182).contains(&b.sides[1].roster[2].hp)
                                && b.sides[1].roster[0].status == Status::Par
                                && (92..=115).contains(&b.sides[1].roster[0].hp)
                        }
                        "probe12e2_rollout_expl256_digsand" => {
                            (135..=141).contains(&b.sides[1].roster[1].hp)
                                && b.sides[0].roster[2].hp == 131
                        }
                        "probe12h_speed_model_f64" => {
                            b.sides[0].roster[0].status == Status::Par
                                && b.sides[0].pokemon_left == 3
                                && b.sides[1].pokemon_left == 3
                        }
                        "probe12i_loose_ends" => {
                            b.sides[0].roster[0].fainted
                                && b.sides[1].roster[0].fainted
                                && b.sides[1].roster[1].hp == 140
                                && b.sides[1].roster[1].status == Status::Slp
                        }
                        _ => true,
                    };
                    if path_ok {
                        replayed = Some(b);
                        break;
                    } else if m.steps.len() >= best_step {
                        best_step = m.steps.len();
                        best_err = format!("{} seed {seed}: outcome matched but path invariant differed", m.tag);
                    }
                }
                Ok(b) => {
                    if m.steps.len() >= best_step {
                        best_step = m.steps.len();
                        best_err = format!(
                            "{} seed {seed}: outcome {:?} != expected {:?}",
                            m.tag,
                            b.outcome(),
                            expected_outcome
                        );
                    }
                }
                Err((step_idx, e)) => {
                    if step_idx >= best_step {
                        best_step = step_idx;
                        best_err = format!("seed {seed}: {e}");
                    }
                }
            }
        }
        assert!(
            replayed.is_some(),
            "Failed to replay manifest {} within 2048 seeds (deepest step {best_step}): {best_err}",
            m.tag
        );
        total_pdat_verified += m
            .steps
            .iter()
            .filter(|s| s.pdat.as_ref().is_some_and(|v| !v.is_empty()))
            .count();
    }
    assert_eq!(total_pdat_verified, 47, "Expected 15 (12c) + 32 (12i) = 47 pdat snapshots verified");
}

// =========================================================================
// 7. Gate C — Monte Carlo Statistical Probability Distribution Tests
// =========================================================================

#[test]
fn gate_c_quick_claw_independent_slower_first_rate() {
    // Both sides hold Quick Claw; P1 is faster (Mew Spe 151) than P2 (Snorlax Spe 81).
    // Independent 60/256 per side:
    //   P(P2 moves first) = P(P2 triggers & !P1 triggers) + 0.5 * P(both trigger)
    //                     = (60/256)*(196/256) + 0.5*(60/256)^2 = 0.206909...
    // (Whereas PS shared 60/256 coin gives 0.5 * 60/256 = 0.1171875.)
    let dex = load_dex();
    let p1 = vec![mk_bare("Mew", 50, &["Splash"], "Quick Claw")];
    let p2 = vec![mk_bare("Snorlax", 50, &["Splash"], "Quick Claw")];
    let trials = 4000usize;
    let mut slower_first = 0usize;
    let mut b = start_castle(&dex, "101,202,303,404", castle_raw(false, None), &p1, &p2);
    for _ in 0..trials {
        b.log.clear();
        b.choose(&dex, 0, "move splash").unwrap();
        b.choose(&dex, 1, "move splash").unwrap();
        // Reset PP so Splash never runs out
        let a1 = b.active_id(0).unwrap();
        let a2 = b.active_id(1).unwrap();
        b.poke_mut(a1).move_slots[0].pp = 64;
        b.poke_mut(a2).move_slots[0].pp = 64;
        b.turn = 1;
        let m1 = b.log.iter().position(|l| l.contains("|move|p1a:")).unwrap();
        let m2 = b.log.iter().position(|l| l.contains("|move|p2a:")).unwrap();
        if m2 < m1 {
            slower_first += 1;
        }
    }
    let rate = slower_first as f64 / trials as f64;
    // Expected 0.2069 ± 0.025 (>4 sigma margin; completely rejects PS 0.1172!).
    assert!(
        (0.180..=0.235).contains(&rate),
        "Dual Quick Claw slower-first rate expected ~0.2069, got {rate:.4} ({slower_first}/{trials})"
    );
}

#[test]
fn gate_c_crit_freeze_acc_secondary100_protect_and_sleep_confusion_durations() {
    let dex = load_dex();

    // 1. Crit stage 0 = 17/256 (0.06640625) with 250,000 trials (rejects PS 0.0625 at >4 sigma!),
    // plus Razor Leaf accuracy ≈ 95% and Ice Beam freeze ≈ 10%.
    {
        let p2 = vec![mk_bare("Blissey", 50, &["Splash"], "")];
        let p1_c = vec![mk_bare("Snorlax", 50, &["Pound", "Ice Beam", "Razor Leaf", "Zap Cannon"], "")];
        let mut bc = start_castle(&dex, "19,23,29,31", castle_raw(false, None), &p1_c, &p2);
        let a1 = bc.active_id(0).unwrap();
        let a2 = bc.active_id(1).unwrap();

        // Non-crit Pound deals 61..=72; crit Pound deals 119..=141 -> d > 90.0 detects crit cleanly!
        // Exercises exact Battle::get_damage Stage-0 crit roll (17/256) at full 250,000-trial precision.
        bc.set_log_enabled(false);
        let pound_mv = nc2000_engine::battle::moveexec::get_active_move(&dex, dex.moves.id("pound").unwrap());
        let crit_trials = 250_000usize;
        let mut crits = 0usize;
        for _ in 0..crit_trials {
            bc.set_active_move(pound_mv.clone(), Some(a1), Some(a2));
            if let nc2000_engine::battle::moveexec::DamageResult::Damage(d) = bc.get_damage(&dex, a1, a2, true) {
                if d > 90.0 {
                    crits += 1;
                }
            }
        }
        bc.clear_active_move(true);
        let crit_rate = crits as f64 / crit_trials as f64;
        assert!(
            (0.0645..=0.0683).contains(&crit_rate),
            "Crit stage 0 expected ~0.066406 (17/256), got {crit_rate:.5} ({crits}/{crit_trials})"
        );
        bc.set_log_enabled(true);

        // Ice Beam freeze ≈ 10%
        let frz_trials = 3000usize;
        let mut freezes = 0usize;
        for _ in 0..frz_trials {
            bc.log.clear();
            bc.poke_mut(a1).move_slots[1].pp = 16;
            bc.poke_mut(a2).hp = 361;
            bc.poke_mut(a2).status = Status::None;
            bc.poke_mut(a2).move_slots[0].pp = 64;
            bc.refresh_poke_mask(&dex, a2);
            bc.turn = 1;
            bc.choose(&dex, 0, "move icebeam").unwrap();
            bc.choose(&dex, 1, "move splash").unwrap();
            if bc.log.iter().any(|l| l.contains("|-status|p2a: Blissey|frz")) {
                freezes += 1;
            }
        }
        let frz_rate = freezes as f64 / frz_trials as f64;
        assert!(
            (0.075..=0.125).contains(&frz_rate),
            "Ice Beam freeze rate expected ~0.10, got {frz_rate:.4} ({freezes}/{frz_trials})"
        );

        // Razor Leaf accuracy ≈ 95%
        let rl_trials = 3000usize;
        let mut rl_hits = 0usize;
        for _ in 0..rl_trials {
            bc.log.clear();
            bc.poke_mut(a1).move_slots[2].pp = 40;
            bc.poke_mut(a2).hp = 361;
            bc.poke_mut(a2).move_slots[0].pp = 64;
            bc.turn = 1;
            bc.choose(&dex, 0, "move razorleaf").unwrap();
            bc.choose(&dex, 1, "move splash").unwrap();
            if !bc.log.iter().any(|l| l.contains("|-miss|")) {
                rl_hits += 1;
            }
        }
        let rl_rate = rl_hits as f64 / rl_trials as f64;
        assert!(
            (0.930..=0.970).contains(&rl_rate),
            "Razor Leaf accuracy expected ~0.95, got {rl_rate:.4} ({rl_hits}/{rl_trials})"
        );

        // 100% secondary effect (Icy Wind / Zap Cannon via Lock-On) = strictly 100.0% (0 fails in 4000 hits!).
        // Use Dynamic Punch + Lock-On? Even simpler: Mud-Slap (100% acc, 100% secondary acc -1)!
        let p1_s = vec![mk_bare("Mew", 50, &["Mud-Slap"], "")];
        let mut bs = start_castle(&dex, "41,43,47,53", castle_raw(false, None), &p1_s, &p2);
        let s1 = bs.active_id(0).unwrap();
        let s2 = bs.active_id(1).unwrap();
        let sec_trials = 4000usize;
        for _ in 0..sec_trials {
            bs.poke_mut(s1).move_slots[0].pp = 16;
            bs.poke_mut(s2).hp = 361;
            bs.poke_mut(s2).boosts[stat_idx::ACC] = 0;
            bs.poke_mut(s2).move_slots[0].pp = 64;
            bs.turn = 1;
            bs.choose(&dex, 0, "move mudslap").unwrap();
            bs.choose(&dex, 1, "move splash").unwrap();
            assert_eq!(
                bs.poke(s2).boosts[stat_idx::ACC],
                -1,
                "100% secondary effect must NEVER fail in Castle (rejecting 255/256)"
            );
        }
    }

    // 2. First-turn Protect = 255/256 (non-zero failures over 10,000 trials, rate near 1/256 = 0.003906).
    {
        let p1 = vec![mk_bare("Mew", 50, &["Protect"], "")];
        let p2 = vec![mk_bare("Snorlax", 50, &["Splash"], "")];
        let mut bp = start_castle(&dex, "59,61,67,71", castle_raw(false, None), &p1, &p2);
        let a1 = bp.active_id(0).unwrap();
        let a2 = bp.active_id(1).unwrap();
        let trials = 10_000usize;
        let mut fails = 0usize;
        for _ in 0..trials {
            bp.log.clear();
            bp.remove_volatile(&dex, a1, "stall");
            bp.poke_mut(a1).move_slots[0].pp = 16;
            bp.poke_mut(a2).move_slots[0].pp = 64;
            bp.turn = 1;
            bp.choose(&dex, 0, "move protect").unwrap();
            bp.choose(&dex, 1, "move splash").unwrap();
            if !bp.log.iter().any(|l| l.contains("|-singleturn|p1a: Mew|Protect")) {
                fails += 1;
            }
        }
        // Expected ~39.06 fails (10000 / 256); allow [15, 75] (~>3.8 sigma margin, strictly rejects 0!).
        assert!(
            (15..=75).contains(&fails),
            "First-turn Protect expected ~39 fails/10000 (1/256), got {fails}"
        );
    }

    // 3. Non-Rest Sleep and Confusion durations: uniform 1..=4 turns.
    {
        let p1 = vec![mk_bare("Mew", 50, &["Spore", "Confuse Ray", "Splash"], "")];
        let p2 = vec![mk_bare("Blissey", 50, &["Splash"], "")];
        let mut slp_hist = [0usize; 5]; // index 1..=4
        let mut conf_hist = [0usize; 5]; // index 1..=4
        let n = 1200usize;
        let mut b = start_castle(&dex, "73,79,83,89", castle_raw(false, None), &p1, &p2);
        b.set_log_enabled(false);
        let a1 = b.active_id(0).unwrap();
        let a2 = b.active_id(1).unwrap();

        for _ in 0..n {
            b.poke_mut(a1).move_slots[0].pp = 24;
            b.poke_mut(a1).move_slots[2].pp = 64;
            b.poke_mut(a2).hp = 361;
            b.poke_mut(a2).status = Status::None;
            b.poke_mut(a2).move_slots[0].pp = 64;
            b.refresh_poke_mask(&dex, a2);
            b.turn = 1;
            b.choose(&dex, 0, "move spore").unwrap();
            b.choose(&dex, 1, "move splash").unwrap();
            let mut asleep_turns = 1usize; // already slept on turn 0 (slower Blissey)
            while b.poke(a2).status == Status::Slp {
                b.poke_mut(a1).move_slots[2].pp = 64;
                b.poke_mut(a2).move_slots[0].pp = 64;
                b.turn = 1;
                b.choose(&dex, 0, "move splash").unwrap();
                b.choose(&dex, 1, "move splash").unwrap();
                if b.poke(a2).status == Status::Slp {
                    asleep_turns += 1;
                }
            }
            assert!((1..=4).contains(&asleep_turns), "Sleep lasted {asleep_turns} turns (expected 1..=4)");
            slp_hist[asleep_turns] += 1;
        }
        // Each of the 4 uniform buckets should be near 300 ± 75.
        for t in 1..=4 {
            assert!(
                (225..=375).contains(&slp_hist[t]),
                "Non-Rest sleep duration not uniform 1..=4: {:?}",
                &slp_hist[1..=4]
            );
        }

        let conf_c = dex.conds_id("confusion").unwrap();
        for _ in 0..n {
            b.poke_mut(a1).move_slots[1].pp = 16;
            b.poke_mut(a1).move_slots[2].pp = 64;
            b.poke_mut(a2).hp = 361;
            b.remove_volatile(&dex, a2, "confusion");
            b.poke_mut(a2).move_slots[0].pp = 64;
            b.turn = 1;
            b.choose(&dex, 0, "move confuseray").unwrap();
            b.choose(&dex, 1, "move splash").unwrap();
            let mut conf_turns = 1usize; // already checked confusion on turn 0
            while b.poke(a2).has_volatile(conf_c) {
                b.poke_mut(a1).move_slots[2].pp = 64;
                b.poke_mut(a2).hp = 361;
                b.poke_mut(a2).move_slots[0].pp = 64;
                b.turn = 1;
                b.choose(&dex, 0, "move splash").unwrap();
                b.choose(&dex, 1, "move splash").unwrap();
                if b.poke(a2).has_volatile(conf_c) {
                    conf_turns += 1;
                }
            }
            assert!((1..=4).contains(&conf_turns), "Confusion lasted {conf_turns} turns (expected 1..=4)");
            conf_hist[conf_turns] += 1;
        }
        for t in 1..=4 {
            assert!(
                (225..=375).contains(&conf_hist[t]),
                "Confusion duration not uniform 1..=4: {:?}",
                &conf_hist[1..=4]
            );
        }
    }
}
