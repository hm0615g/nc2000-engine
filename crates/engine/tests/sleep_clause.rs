use nc2000_engine::battle::{EffectHandle, Outcome, PokemonSet, RV};
use nc2000_engine::dex::Dex;
use nc2000_engine::state::{Battle, PokeId, Status};

fn dex() -> Dex {
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/gen2stadium2.json");
    Dex::from_json(&std::fs::read_to_string(path).unwrap()).unwrap()
}

fn mk(species: &str, item: &str, moves: &[&str]) -> PokemonSet {
    serde_json::from_value(serde_json::json!({
        "name": species, "species": species, "item": item, "ability": "No Ability",
        "moves": moves, "level": 50,
        "evs": {"hp":255,"atk":255,"def":255,"spa":255,"spd":255,"spe":255},
        "ivs": {"hp":30,"atk":30,"def":30,"spa":30,"spd":30,"spe":30},
        "happiness":255
    }))
    .unwrap()
}

fn start(d: &Dex, item: &str) -> Battle {
    let ours = vec![
        mk("Parasect", "", &["Spore", "Slash", "Swords Dance"]),
        mk("Clefable", "", &["Metronome", "Defense Curl"]),
        mk("Jynx", "", &["Lovely Kiss", "Psychic", "Ice Beam"]),
    ];
    let theirs = vec![
        mk("Snorlax", "", &["Amnesia", "Rest", "Body Slam"]),
        mk(
            "Mr. Mime",
            item,
            &["Encore", "Reflect", "Rest", "Substitute"],
        ),
        mk("Blissey", "", &["Defense Curl", "Soft-Boiled"]),
    ];
    let mut b = Battle::from_fixture(d, "1,2,3,4", &ours, &theirs).unwrap();
    b.choose(d, 0, "team 1,2,3").unwrap();
    b.choose(d, 1, "team 1,2,3").unwrap();
    b.log.clear();
    b
}

fn turn(d: &Dex, b: &mut Battle, ours: &str, theirs: &str) {
    b.choose(d, 0, ours).unwrap();
    b.choose(d, 1, theirs).unwrap();
}

fn mon(b: &Battle, side: usize, slot: usize) -> PokeId {
    PokeId {
        side: side as u8,
        slot: b.sides[side].party[slot],
    }
}

fn foe_asleep(d: &Dex, item: &str) -> Battle {
    let mut b = start(d, item);
    turn(d, &mut b, "move spore", "move amnesia");
    assert_eq!(b.poke(b.active_id(1).unwrap()).status, Status::Slp);
    b
}

fn assert_forfeit(d: &Dex, b: &mut Battle, loser: usize) {
    assert_eq!(
        b.outcome(),
        Some(if loser == 0 {
            Outcome::P2Win
        } else {
            Outcome::P1Win
        }),
        "{:?}",
        b.log
    );
    assert_eq!(b.needs_choice(), [false, false]);
    assert!(b.legal_choices(d, 0).is_empty());
    assert!(b.legal_choices(d, 1).is_empty());
    assert_eq!(b.log.iter().filter(|l| l.starts_with("|win|")).count(), 1);
    assert!(b.log.last().unwrap().starts_with("|win|"), "{:?}", b.log);
}

#[test]
fn a_second_foe_sleep_forfeits_on_a_staying_or_switching_target() {
    let d = dex();
    for switching in [false, true] {
        let mut b = foe_asleep(&d, "");
        if !switching {
            turn(&d, &mut b, "move swordsdance", "switch 2");
        }
        turn(
            &d,
            &mut b,
            "move spore",
            if switching {
                "switch 2"
            } else {
                "move reflect"
            },
        );
        assert_eq!(b.poke(b.active_id(1).unwrap()).status, Status::Slp);
        assert_forfeit(&d, &mut b, 0);
    }
}

#[test]
fn an_existing_rest_sleeper_counts_and_berries_do_not_prevent_forfeit() {
    let d = dex();
    for rest in [false, true] {
        for item in ["", "Mint Berry", "Miracle Berry"] {
            let mut b = if rest {
                let mut b = start(&d, item);
                turn(&d, &mut b, "move slash", "move amnesia");
                turn(&d, &mut b, "move swordsdance", "move rest");
                let sleeper = b.active_id(1).unwrap();
                assert_eq!(b.poke(sleeper).status_state.source, Some(sleeper));
                b
            } else {
                foe_asleep(&d, item)
            };
            turn(&d, &mut b, "move spore", "switch 2");
            assert_forfeit(&d, &mut b, 0);
            let target = b.active_id(1).unwrap();
            if !item.is_empty() {
                assert!(b.eat_item(&d, target, false, None, EffectHandle::None));
                assert_eq!(b.poke(target).status, Status::None);
                assert_eq!(b.outcome(), Some(Outcome::P2Win));
            }
        }
    }
}

#[test]
fn encore_forcing_a_sleep_move_still_forfeits() {
    let d = dex();
    let mut b = start(&d, "");
    turn(&d, &mut b, "move slash", "move amnesia");
    turn(&d, &mut b, "move swordsdance", "move rest");
    turn(&d, &mut b, "move swordsdance", "switch 2");
    turn(&d, &mut b, "move spore", "move substitute");
    turn(&d, &mut b, "move spore", "move encore");
    let user = b.active_id(0).unwrap();
    assert!(b.poke(user).has_volatile(d.conds_id("encore").unwrap()));
    assert!(b.legal_choices(&d, 0).iter().all(|c| match c {
        nc2000_engine::battle::SearchChoice::Move(id) => d.moves.key(*id) == "spore",
        _ => true,
    }));
    turn(&d, &mut b, "move spore", "switch 3");
    assert_forfeit(&d, &mut b, 0);
}

#[test]
fn a_sleep_move_called_by_metronome_still_forfeits() {
    let d = dex();
    let mut b = foe_asleep(&d, "");
    turn(&d, &mut b, "switch 2", "switch 2");
    b.reseed(53);
    turn(&d, &mut b, "move metronome", "move reflect");
    assert!(b
        .log
        .iter()
        .any(|l| l.contains("|Sleep Powder|") && l.ends_with("[from] Metronome")));
    assert_forfeit(&d, &mut b, 0);
}

#[test]
fn rest_after_foe_sleep_is_legal() {
    let d = dex();
    let mut b = foe_asleep(&d, "");
    turn(&d, &mut b, "move slash", "switch 2");
    turn(&d, &mut b, "move swordsdance", "move rest");
    assert_eq!(b.outcome(), None);
    let sleepers = b.sides[1]
        .party
        .iter()
        .filter(|&&slot| b.sides[1].roster[slot as usize].status == Status::Slp)
        .count();
    assert_eq!(sleepers, 2);
}

#[test]
fn failed_sleep_does_not_forfeit() {
    let d = dex();
    for status in [Status::Slp, Status::Par, Status::Frz] {
        let mut b = foe_asleep(&d, "");
        if status != Status::Slp {
            turn(&d, &mut b, "move swordsdance", "switch 2");
            let target = b.active_id(1).unwrap();
            b.restore_status(&d, target, status, None);
        }
        turn(
            &d,
            &mut b,
            "move spore",
            if status == Status::Slp {
                "move amnesia"
            } else {
                "move reflect"
            },
        );
        assert_eq!(b.outcome(), None, "{:?}", b.log);
    }
    for protection in ["substitute", "safeguard"] {
        let mut b = foe_asleep(&d, "");
        turn(&d, &mut b, "move swordsdance", "switch 2");
        let target = b.active_id(1).unwrap();
        if protection == "substitute" {
            b.add_volatile(&d, target, protection, Some(target), EffectHandle::None);
        } else {
            b.add_side_condition(&d, 1, protection, Some(target), EffectHandle::None);
        }
        turn(&d, &mut b, "move spore", "move reflect");
        assert_eq!(b.outcome(), None);
        assert_eq!(b.poke(target).status, Status::None);
    }
}

#[test]
fn a_missed_sleep_move_does_not_forfeit() {
    let d = dex();
    let mut b = foe_asleep(&d, "");
    turn(&d, &mut b, "switch 3", "switch 2");
    b.log.clear();
    let mut misses = 0;
    for seed in 0..32 {
        let mut trial = b.clone();
        trial.reseed(seed);
        turn(&d, &mut trial, "move lovelykiss", "move reflect");
        if trial.log.iter().any(|l| l.starts_with("|-miss|")) {
            misses += 1;
            assert_eq!(trial.outcome(), None);
        } else {
            assert_forfeit(&d, &mut trial, 0);
        }
    }
    assert!(misses > 0);
}

#[test]
fn cured_or_fainted_sleepers_do_not_count() {
    let d = dex();
    for fainted in [false, true] {
        let mut b = foe_asleep(&d, "");
        turn(&d, &mut b, "move swordsdance", "switch 2");
        let sleeper = mon(&b, 1, 1);
        if fainted {
            b.poke_mut(sleeper).hp = 0;
            b.poke_mut(sleeper).fainted = true;
            b.sides[1].pokemon_left -= 1;
        } else {
            b.cure_status(&d, sleeper, false);
        }
        turn(&d, &mut b, "move spore", "move reflect");
        assert_eq!(b.outcome(), None);
        assert_eq!(b.poke(b.active_id(1).unwrap()).status, Status::Slp);
    }
}

#[test]
fn freeze_is_independent_of_sleep_and_blocks_only_a_second_freeze() {
    let d = dex();
    for first_status in [Status::Slp, Status::Frz] {
        let mut b = start(&d, "");
        let source = b.active_id(0).unwrap();
        let first = b.active_id(1).unwrap();
        let second = mon(&b, 1, 1);
        let third = mon(&b, 1, 2);
        let other = if first_status == Status::Slp {
            Status::Frz
        } else {
            Status::Slp
        };
        assert_eq!(
            b.try_set_status(
                &d,
                first,
                first_status.as_str(),
                Some(source),
                EffectHandle::None
            ),
            RV::True
        );
        assert_eq!(
            b.try_set_status(&d, second, other.as_str(), Some(source), EffectHandle::None),
            RV::True
        );
        assert_eq!(b.outcome(), None);
        assert_eq!(
            b.try_set_status(&d, third, "frz", Some(source), EffectHandle::None),
            RV::False
        );
        assert_eq!(b.poke(third).status, Status::None);
        assert_eq!(b.outcome(), None);
    }
}

#[test]
fn restored_sleepers_do_not_reenact_infliction_in_either_order() {
    let d = dex();
    for reverse in [false, true] {
        let mut b = start(&d, "");
        let source = b.active_id(0).unwrap();
        let foe_sleep = mon(&b, 1, 0);
        let rest_sleep = mon(&b, 1, 1);
        let mut entries = [(foe_sleep, source), (rest_sleep, rest_sleep)];
        if reverse {
            entries.reverse();
        }
        for (target, source) in entries {
            assert_eq!(
                b.restore_status(&d, target, Status::Slp, Some(source)),
                RV::True
            );
            assert_eq!(b.poke(target).status, Status::Slp);
            assert_eq!(b.poke(target).status_state.source, Some(source));
        }
        assert_eq!(b.outcome(), None);
        assert_eq!(b.needs_choice(), [true, true]);
        let target = mon(&b, 1, 2);
        assert_eq!(
            b.try_set_status(&d, target, "slp", Some(source), EffectHandle::None),
            RV::True
        );
        assert_forfeit(&d, &mut b, 0);
    }
}

#[test]
fn the_inflicting_side_loses_with_logging_enabled_or_disabled() {
    let d = dex();
    for loser in [0, 1] {
        let mut results = Vec::new();
        for logging in [false, true] {
            let mut b = start(&d, "");
            b.set_log_enabled(logging);
            let source = b.active_id(loser).unwrap();
            for i in [0, 1] {
                let target = mon(&b, 1 - loser, i);
                assert_eq!(
                    b.try_set_status(&d, target, "slp", Some(source), EffectHandle::None),
                    RV::True
                );
            }
            assert_eq!(
                b.outcome(),
                Some(if loser == 0 {
                    Outcome::P2Win
                } else {
                    Outcome::P1Win
                })
            );
            results.push((b.state_key128(), b.prng.seed_str()));
        }
        assert_eq!(results[0], results[1]);
    }
}

#[test]
fn forfeit_stops_after_move_damage_and_the_remaining_turn() {
    let d = dex();
    for called in [false, true] {
        let mut b = foe_asleep(&d, "");
        turn(
            &d,
            &mut b,
            if called {
                "switch 2"
            } else {
                "move swordsdance"
            },
            "switch 2",
        );
        let actor = b.active_id(0).unwrap();
        b.restore_status(&d, actor, Status::Psn, None);
        let hp = b.poke(actor).hp;
        if called {
            b.reseed(53);
        }
        turn(
            &d,
            &mut b,
            if called {
                "move metronome"
            } else {
                "move spore"
            },
            "move reflect",
        );
        assert_forfeit(&d, &mut b, 0);
        assert_eq!(b.poke(actor).hp, hp);
    }
}
