use conformance::load_dex;
use nc2000_bot::smmcts::dominated_actions;
use nc2000_engine::{
    battle::{enumerate::enumerate_step_partial, PokemonSet, SearchChoice},
    dex::Dex,
    state::Battle,
};

fn position(dex: &Dex, side: usize, species: &str, attack: &str, setup: &str) -> Battle {
    let set = |species: &str, moves: &[&str], defense: u8| -> PokemonSet {
        serde_json::from_value(serde_json::json!({
            "species":species,"name":species,"level":50,"gender":"M","moves":moves,
            "evs":{"hp":255,"atk":255,"def":defense,"spa":255,"spd":255,"spe":255},
            "ivs":{"hp":30,"atk":30,"def":30,"spa":30,"spd":30,"spe":30}
        })).unwrap()
    };
    let own = [set("Jolteon", &["Substitute", "Thunderbolt", "Rest", "Agility"], 0)];
    let foe = [set(species, &[attack, setup], 255)];
    let teams = if side == 0 { [&own, &foe] } else { [&foe, &own] };
    let mut b = Battle::from_fixture(dex, "1,2,3,4", teams[0], teams[1]).unwrap();
    b.choose(dex, 0, "team 1").unwrap();
    b.choose(dex, 1, "team 1").unwrap();
    b.choose(dex, side, "move substitute").unwrap();
    b.choose(dex, 1 - side, &format!("move {setup}")).unwrap();
    let own = b.active_id(side).unwrap();
    let foe = b.active_id(1 - side).unwrap();
    assert!(b.get_pokemon_action_speed(dex, own) > b.get_pokemon_action_speed(dex, foe));
    assert!(b.poke(own).has_volatile(dex.conds_id("substitute").unwrap()));
    b
}

fn masked(b: &Battle, dex: &Dex, side: usize) -> bool {
    dominated_actions(b, dex, side).iter()
        .any(|(action, _)| *action == SearchChoice::Move(dex.moves.id("substitute").unwrap()))
}

#[test]
fn priority_damage_can_break_and_replace_an_existing_substitute() {
    let dex = load_dex();
    let mut incorrectly_masked = Vec::new();
    for side in 0..2 {
        for (species, attack, setup) in [
            ("Scizor", "quickattack", "swordsdance"),
            ("Hitmonchan", "machpunch", "detect"),
            ("Arcanine", "extremespeed", "leer"),
        ] {
            let b = position(&dex, side, species, attack, setup);
            let own = b.active_id(side).unwrap();
            let hp = b.poke(own).hp;
            let cost = b.poke(own).maxhp / 4;
            let mut joint = [None; 2];
            joint[side] = Some(SearchChoice::Move(dex.moves.id("substitute").unwrap()));
            joint[1 - side] = Some(SearchChoice::Move(dex.moves.id(attack).unwrap()));
            let step = enumerate_step_partial(&dex, &b, joint, 4096);
            assert!(!step.exhausted && step.pending.is_empty());
            let mut rebuilt = 0.0;
            let mut unchanged = 0.0;
            for leaf in &step.leaves {
                assert!(leaf.battle.poke(own).has_volatile(dex.conds_id("substitute").unwrap()));
                match leaf.battle.poke(own).hp {
                    after if after == hp - cost => rebuilt += leaf.prob,
                    after if after == hp => unchanged += leaf.prob,
                    after => panic!("unexpected HP {after}, before {hp}, cost {cost}"),
                }
            }
            assert!((rebuilt + unchanged - 1.0).abs() < 1e-9);
            assert!(rebuilt > 0.99, "{species}: {rebuilt}");
            let excluded = masked(&b, &dex, side);
            println!("{}", serde_json::json!({"side":side,"species":species,"attack":attack,
                "runs":step.runs,"rebuilt_mass":rebuilt,"unchanged_mass":unchanged,"masked":excluded}));
            if excluded { incorrectly_masked.push((side, species)); }
        }
    }
    assert!(incorrectly_masked.is_empty(), "{incorrectly_masked:?}");
}

#[test]
fn existing_substitute_and_insufficient_hp_still_exclude_certain_failures() {
    let dex = load_dex();
    for side in 0..2 {
        assert!(masked(&position(&dex, side, "Snorlax", "return", "curse"), &dex, side));
        let mut b = position(&dex, side, "Scizor", "quickattack", "swordsdance");
        let own = b.active_id(side).unwrap();
        b.poke_mut(own).hp = b.poke(own).maxhp / 4;
        assert!(masked(&b, &dex, side));
        b.remove_volatile(&dex, own, "substitute");
        assert!(masked(&b, &dex, side));
    }
}

#[test]
fn hidden_move_identity_and_pp_do_not_control_substitute_exclusion() {
    let dex = load_dex();
    for side in 0..2 {
        for species in ["Scizor", "Snorlax"] {
            let mut b = position(&dex, side, species, "return", "swordsdance");
            let expected = masked(&b, &dex, side);
            assert_eq!(expected, species == "Snorlax");
            let foe = b.active_id(1 - side).unwrap();
            for key in ["quickattack", "machpunch", "extremespeed", "splash"] {
                for pp in [0, 1, 32] {
                    b.poke_mut(foe).move_slots[0].id = dex.moves.id(key).unwrap();
                    b.poke_mut(foe).move_slots[0].pp = pp;
                    assert_eq!(masked(&b, &dex, side), expected);
                }
            }
        }
    }
}

#[test]
fn copied_moves_unknown_learnsets_and_quick_claw_keep_substitute_available() {
    let dex = load_dex();
    for side in 0..2 {
        for case in ["transformed", "copied", "unknown", "quickclaw"] {
            let mut b = position(&dex, side, "Snorlax", "return", "curse");
            let foe = b.active_id(1 - side).unwrap();
            match case {
                "transformed" => b.poke_mut(foe).transformed = true,
                "copied" => b.poke_mut(foe).move_slots[0].shared = false,
                "unknown" => b.poke_mut(foe).species = dex.species.id("mewtwo").unwrap(),
                "quickclaw" => {
                    b.poke_mut(foe).item = dex.items.id("quickclaw");
                    b.quick_claw_roll = false;
                }
                _ => unreachable!(),
            }
            assert!(!masked(&b, &dex, side), "{case}");
        }
    }
}

#[test]
fn substitute_bellman_1v1_baton_pass_and_ohko_priority_invariants() {
    use nc2000_bot::{
        eval::{compact_eval01_blindspot, EvalWeights},
        linear::toeplitz_coupled_race,
        mcts::{playout_pick, Playout},
        RmConfig, SkuctSearch, SplitMix64,
    };
    use nc2000_engine::{
        battle::EffectHandle,
        compact::CompactBattle,
        state::Scalar,
    };

    let dex = load_dex();
    let w = EvalWeights::default();
    let make = |sp: &str, lv: u8, item: &str, moves: &[&str]| -> PokemonSet {
        serde_json::from_value(serde_json::json!({
            "species": sp, "name": sp, "level": lv, "item": item, "ability": "No Ability",
            "gender": "M", "moves": moves,
            "evs": {"hp": 255, "atk": 255, "def": 255, "spa": 255, "spd": 255, "spe": 255},
            "ivs": {"hp": 30, "atk": 30, "def": 30, "spa": 30, "spd": 30, "spe": 30}
        }))
        .unwrap()
    };

    // 1. 1v1 Bellman race (`toeplitz_coupled_race`) & `compact_eval01_blindspot`:
    //    Zapdos at 75% HP WITH an active Substitute vs Rest Snorlax must evaluate strictly higher
    //    than Zapdos at 75% HP without a Substitute (Substitute absorbs Body Slam and blocks paralysis).
    let zap = make(
        "Zapdos",
        55,
        "Leftovers",
        &["Thunderbolt", "Substitute", "Swagger", "Hidden Power"],
    );
    let lax = make(
        "Snorlax",
        55,
        "Leftovers",
        &["Body Slam", "Earthquake", "Curse", "Rest"],
    );
    let mut b1 = Battle::from_fixture(&dex, "1,2,3,4", &[zap], &[lax.clone()]).unwrap();
    b1.choose(&dex, 0, "team 1").unwrap();
    b1.choose(&dex, 1, "team 1").unwrap();
    let p0 = b1.active_id(0).unwrap();
    let cost = b1.poke(p0).maxhp / 4;
    b1.poke_mut(p0).hp -= cost;
    let (t_no_sub, cb_no_sub) = CompactBattle::from_battle(&b1, &dex);
    let tr_no_sub = toeplitz_coupled_race(&cb_no_sub, &t_no_sub, 0, 0, 1, 0);
    let ev_no_sub = compact_eval01_blindspot(&cb_no_sub, &t_no_sub, &w);

    b1.choose(&dex, 0, "move substitute").unwrap();
    b1.choose(&dex, 1, "move curse").unwrap();
    b1.poke_mut(b1.active_id(1).unwrap()).boosts = [0; 7];
    b1.poke_mut(p0).hp = b1.poke(p0).maxhp - cost;
    let (t_sub, cb_sub) = CompactBattle::from_battle(&b1, &dex);
    let tr_sub = toeplitz_coupled_race(&cb_sub, &t_sub, 0, 0, 1, 0);
    let ev_sub = compact_eval01_blindspot(&cb_sub, &t_sub, &w);
    assert!(
        tr_sub > tr_no_sub + 0.10,
        "1v1 Bellman race must credit active Substitute absorption: tr_sub={tr_sub} vs tr_no_sub={tr_no_sub}"
    );
    assert!(
        ev_sub > ev_no_sub + 0.05,
        "1v1 leaf eval must credit active Substitute: ev_sub={ev_sub} vs ev_no_sub={ev_no_sub}"
    );

    // 2. Baton Pass + Substitute rollout & receiver selection:
    //    Jolteon with an active Substitute vs sleeping Snorlax must Baton Pass to Thick Club Marowak
    //    (not passive Vaporeon) and Marowak with the passed Substitute must evaluate higher than Jolteon.
    let jolt = make(
        "Jolteon",
        50,
        "Leftovers",
        &["Thunderbolt", "Substitute", "Baton Pass", "Agility"],
    );
    let wak = make(
        "Marowak",
        55,
        "Thick Club",
        &["Earthquake", "Rock Slide", "Swords Dance", "Hidden Power"],
    );
    let vap = make(
        "Vaporeon",
        50,
        "Leftovers",
        &["Surf", "Ice Beam", "Rest", "Growth"],
    );
    let skarm = make(
        "Skarmory",
        50,
        "Leftovers",
        &["Drill Peck", "Whirlwind", "Rest", "Curse"],
    );
    let tyra = make(
        "Tyranitar",
        50,
        "Leftovers",
        &["Rock Slide", "Crunch", "Roar", "Fire Blast"],
    );
    let mut b2 = Battle::from_fixture(
        &dex,
        "1,2,3,4",
        &[jolt, wak, vap],
        &[lax.clone(), skarm.clone(), tyra.clone()],
    )
    .unwrap();
    b2.choose(&dex, 0, "team 1, 2, 3").unwrap();
    b2.choose(&dex, 1, "team 1, 2, 3").unwrap();
    let p2_lax = b2.active_id(1).unwrap();
    b2.set_status(&dex, p2_lax, "slp", None, EffectHandle::None, true);
    b2.poke_mut(p2_lax)
        .status_state
        .set(nc2000_engine::state::DK::Time, Scalar::Int(3));
    b2.choose(&dex, 0, "move substitute").unwrap();
    b2.choose(&dex, 1, "move bodyslam").unwrap();
    assert!(b2
        .poke(b2.active_id(0).unwrap())
        .has_volatile(dex.conds_id("substitute").unwrap()));

    let (t_jolt_sub, cb_jolt_sub) = CompactBattle::from_battle(&b2, &dex);
    let ev_jolt_sub = compact_eval01_blindspot(&cb_jolt_sub, &t_jolt_sub, &w);

    // Rollout greedy pick on Jolteon with Sub up must choose Baton Pass, and then switch 2 (Marowak)
    let rules = RmConfig::default().rollout_rules();
    let cs_move = b2.legal_choices(&dex, 0);
    let pick_move = playout_pick(
        &b2,
        &dex,
        &Playout::Heavy {
            eps: 0.0,
            turns: 8,
            weights: w.clone(),
        },
        0,
        &cs_move,
        &mut SplitMix64::new(1),
        rules,
    );
    assert_eq!(
        pick_move.to_input(&dex),
        "move batonpass",
        "Jolteon with active Substitute must Baton Pass to bench sweeper"
    );

    b2.choose(&dex, 0, "move batonpass").unwrap();
    b2.choose(&dex, 1, "move bodyslam").unwrap();
    let cs_sw = b2.legal_choices(&dex, 0);
    let pick_sw = playout_pick(
        &b2,
        &dex,
        &Playout::Heavy {
            eps: 0.0,
            turns: 8,
            weights: w.clone(),
        },
        0,
        &cs_sw,
        &mut SplitMix64::new(1),
        rules,
    );
    assert_eq!(
        pick_sw.to_input(&dex),
        "switch 2",
        "Baton Pass with Substitute must select Thick Club Marowak (switch 2) over Vaporeon (switch 3)"
    );

    b2.choose(&dex, 0, "switch 2").unwrap();
    let (t_wak_sub, cb_wak_sub) = CompactBattle::from_battle(&b2, &dex);
    let ev_wak_sub = compact_eval01_blindspot(&cb_wak_sub, &t_wak_sub, &w);
    assert!(
        ev_wak_sub > ev_jolt_sub,
        "Marowak receiving Substitute via Baton Pass must evaluate higher than Jolteon holding it: {ev_wak_sub} vs {ev_jolt_sub}"
    );

    // 3. Guaranteed faster OHKO (`Alakazam` Psychic vs 25% HP paralyzed `Kingdra`) must
    //    take the immediate KO with `move psychic` instead of wasting a turn on `Substitute`.
    let zam = make(
        "Alakazam",
        55,
        "Miracle Berry",
        &["Psychic", "Recover", "Substitute", "Thunder Wave"],
    );
    let king = make(
        "Kingdra",
        55,
        "Leftovers",
        &["Surf", "Dragon Breath", "Rest", "Sleep Talk"],
    );
    let mut b3 = Battle::from_fixture(&dex, "1,2,3,4", &[zam], &[king]).unwrap();
    b3.choose(&dex, 0, "team 1").unwrap();
    b3.choose(&dex, 1, "team 1").unwrap();
    let p2_king = b3.active_id(1).unwrap();
    b3.poke_mut(p2_king).hp = b3.poke(p2_king).maxhp / 4;
    b3.set_status(&dex, p2_king, "par", None, EffectHandle::None, true);
    let mut search = SkuctSearch::new(&b3, &dex, RmConfig::default(), 42);
    search.step(&dex, 400);
    assert_eq!(
        search.best(0).unwrap().to_input(&dex),
        "move psychic",
        "Alakazam must take the guaranteed OHKO with Psychic instead of clicking Substitute"
    );
}
