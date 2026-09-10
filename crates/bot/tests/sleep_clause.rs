use conformance::load_dex;
use nc2000_bot::smmcts::dominated_actions;
use nc2000_bot::{Belief, BlindSearch, Observer, RmConfig, SkuctSearch};
use nc2000_engine::battle::{PokemonSet, SearchChoice};
use nc2000_engine::dex::Dex;
use nc2000_engine::state::{Battle, PokeId, Status};

fn set(species: &str, moves: &[&str]) -> PokemonSet {
    serde_json::from_value(serde_json::json!({
        "species": species, "name": species, "moves": moves,
        "level": 50, "ability": "No Ability", "item": "",
        "evs": {"hp":255,"atk":255,"def":255,"spa":255,"spd":255,"spe":255},
        "ivs": {"hp":30,"atk":30,"def":30,"spa":30,"spd":30,"spe":30}
    }))
    .unwrap()
}

fn position(rest: bool, fast: bool) -> (Dex, Battle, Observer, Belief) {
    let dex = load_dex();
    let ours = [
        set("Parasect", &["Spore", "Slash"]),
        set("Jynx", &["Ice Beam"]),
        set("Clefable", &["Metronome"]),
    ];
    let theirs = [
        set("Mr. Mime", &["Reflect"]),
        set("Snorlax", &["Rest"]),
        set("Blissey", &["Defense Curl"]),
    ];
    let mut battle = Battle::from_fixture(&dex, "1,2,3,4", &ours, &theirs).unwrap();
    let mut observed = Observer::new(&battle, 0);
    let belief = Belief::pinned_from_battle(&battle, &observed);
    battle.choose(&dex, 0, "team 1,2,3").unwrap();
    battle.choose(&dex, 1, "team 1,2,3").unwrap();
    let actor = battle.active_id(0).unwrap();
    let bench = PokeId {
        side: 1,
        slot: battle.sides[1].party[1],
    };
    battle.restore_status(
        &dex,
        bench,
        Status::Slp,
        Some(if rest { bench } else { actor }),
    );
    battle.poke_mut(bench).previously_switched_in = 1;
    if fast {
        battle.poke_mut(actor).boosts[4] = 6;
    }
    observed.observe(&battle, &dex);
    (dex, battle, observed, belief)
}

#[test]
fn final_choices_avoid_sleep_forfeits_for_both_sleep_sources() {
    for rest in [false, true] {
        let (dex, battle, observed, belief) = position(rest, true);
        let sleep = SearchChoice::Move(dex.moves.id("spore").unwrap());
        assert!(dominated_actions(&battle, &dex, 0)
            .iter()
            .any(|(c, why)| *c == sleep && why.contains("forfeit")));
        for seed in [1, 7] {
            let cfg = RmConfig::default();
            let mut blind = BlindSearch::new(&battle, &dex, cfg.clone(), 0, seed);
            let index = blind.actions().iter().position(|&c| c == sleep).unwrap();
            assert!(blind.dominated()[index]);
            blind.step(&dex, &belief, &observed, 128);
            assert_ne!(blind.best(), Some(sleep));
            let mut full = SkuctSearch::new(&battle, &dex, cfg, seed);
            full.step(&dex, 128);
            assert_ne!(full.best(0), Some(sleep));
        }
    }
}

#[test]
fn search_scores_an_unmasked_second_sleep_as_a_terminal_loss() {
    for rest in [false, true] {
        let (dex, mut battle, observed, belief) = position(rest, false);
        let target = battle.active_id(1).unwrap();
        battle.poke_mut(target).trapped = true;
        let sleep = SearchChoice::Move(dex.moves.id("spore").unwrap());
        assert!(dominated_actions(&battle, &dex, 0)
            .iter()
            .all(|(c, _)| *c != sleep));
        let mut search = BlindSearch::new(&battle, &dex, RmConfig::default(), 0, 11);
        let index = search.actions().iter().position(|&c| c == sleep).unwrap();
        for _ in 0..16 {
            assert_eq!(search.step_forced(&dex, &belief, &observed, index), 0.0);
        }
        assert_eq!(search.means()[index], 0.0);
    }
}

#[test]
fn freeze_does_not_trigger_the_sleep_forfeit_mask() {
    let (dex, mut battle, _, _) = position(false, true);
    let frozen = PokeId {
        side: 1,
        slot: battle.sides[1].party[1],
    };
    battle.restore_status(&dex, frozen, Status::Frz, None);
    let sleep = SearchChoice::Move(dex.moves.id("spore").unwrap());
    assert!(dominated_actions(&battle, &dex, 0)
        .iter()
        .all(|(c, _)| *c != sleep));
    battle.choose(&dex, 0, "switch 2").unwrap();
    battle.choose(&dex, 1, "move reflect").unwrap();
    let ice = SearchChoice::Move(dex.moves.id("icebeam").unwrap());
    assert!(dominated_actions(&battle, &dex, 0)
        .iter()
        .all(|(c, _)| *c != ice));
}
