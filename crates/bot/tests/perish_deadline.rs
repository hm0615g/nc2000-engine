use conformance::load_dex;
use nc2000_bot::{
    smmcts::{dominated_actions, SelRule},
    Belief, BlindSearch, Observer, RmConfig,
};
use nc2000_engine::{
    battle::{EffectHandle, PokemonSet, SearchChoice},
    dex::Dex,
    state::{Battle, PokeId, Status},
};

#[path = "support/perish_deadline.rs"]
mod support;

use support::WaitingMove;

fn waits_for_perish(wait: WaitingMove) {
    let dex = load_dex();
    let (preview, battle) = support::position(&dex, wait);
    let mut observer = Observer::new(&preview, 0);
    let mut belief = Belief::pinned_from_battle(&preview, &observer);
    observer.observe(&battle, &dex);
    belief.sync(&dex, &observer);
    let cfg = RmConfig {
        iterations: 3000,
        rule: SelRule::Ucb,
        c: 1.0,
        ..RmConfig::default()
    };
    let mut search = BlindSearch::new(&battle, &dex, cfg, 0, 102001);
    search.step(&dex, &belief, &observer, 3000);
    let waiting = SearchChoice::Move(dex.moves.id(wait.key()).unwrap());
    assert_eq!(search.best(), Some(waiting), "waiting move: {wait:?}");
}

#[test]
fn waits_without_attacking_into_a_faster_destiny_bond() {
    waits_for_perish(WaitingMove::Return);
}

#[test]
fn waits_without_attacking_into_an_active_destiny_bond() {
    waits_for_perish(WaitingMove::Snore);
}

fn mask_position(dex: &Dex, side: usize) -> Battle {
    let set = |species: &str, moves: &[&str]| -> PokemonSet {
        serde_json::from_value(serde_json::json!({
            "species":species,"name":species,"level":50,"moves":moves,
            "item":"","ability":"No Ability"
        }))
        .unwrap()
    };
    let own = vec![
        set("Smeargle", &["Snore", "Sleep Talk", "Explosion", "Spore"]),
        set("Magikarp", &["Splash"]),
    ];
    let foe = vec![set("Snorlax", &["Rest"]), set("Magikarp", &["Splash"])];
    let teams = if side == 0 { [own, foe] } else { [foe, own] };
    let mut b = Battle::from_fixture(dex, "1,2,3,4", &teams[0], &teams[1]).unwrap();
    b.choose(dex, 0, "team 1,2").unwrap();
    b.choose(dex, 1, "team 1,2").unwrap();
    assert!(
        b.get_pokemon_action_speed(dex, b.active_id(side).unwrap())
            > b.get_pokemon_action_speed(dex, b.active_id(1 - side).unwrap())
    );
    b
}

fn countdown(b: &mut Battle, dex: &Dex, id: PokeId, remaining: i32) {
    b.add_volatile(dex, id, "perishsong", None, EffectHandle::None);
    b.poke_mut(id)
        .volatile_mut(dex.conds_id("perishsong").unwrap())
        .unwrap()
        .duration = Some(remaining);
}

fn masked(b: &Battle, dex: &Dex, side: usize, key: &str) -> bool {
    let action = SearchChoice::Move(dex.moves.id(key).unwrap());
    dominated_actions(b, dex, side)
        .iter()
        .any(|&(candidate, _)| candidate == action)
}

#[test]
fn waiting_is_retained_only_at_the_opposing_actives_deadline() {
    let dex = load_dex();
    for side in 0..2 {
        let base = mask_position(&dex, side);
        let own = base.active_id(side).unwrap();
        let foe = base.active_id(1 - side).unwrap();
        let foe_bench = PokeId {
            side: (1 - side) as u8,
            slot: base.sides[1 - side].party[1],
        };
        for target in [None, Some(own), Some(foe), Some(foe_bench)] {
            for remaining in 1..=3 {
                let mut b = base.clone();
                if let Some(id) = target {
                    countdown(&mut b, &dex, id, remaining);
                }
                let wait_has_value = target == Some(foe) && remaining == 1;
                for key in ["snore", "sleeptalk"] {
                    assert_eq!(
                        masked(&b, &dex, side, key),
                        !wait_has_value,
                        "side={side}, target={target:?}, remaining={remaining}, move={key}"
                    );
                }
            }
        }
    }
}

#[test]
fn an_opponents_deadline_does_not_allow_self_ko_or_sleep_forfeits() {
    let dex = load_dex();
    for side in 0..2 {
        let mut b = mask_position(&dex, side);
        let own = b.active_id(side).unwrap();
        let foe = b.active_id(1 - side).unwrap();
        let own_bench = PokeId {
            side: side as u8,
            slot: b.sides[side].party[1],
        };
        b.poke_mut(own_bench).hp = 0;
        b.poke_mut(own_bench).fainted = true;
        b.sides[side].pokemon_left = 1;
        let foe_bench = PokeId {
            side: (1 - side) as u8,
            slot: b.sides[1 - side].party[1],
        };
        b.restore_status(&dex, foe_bench, Status::Slp, Some(own));
        countdown(&mut b, &dex, foe, 1);
        assert!(masked(&b, &dex, side, "explosion"));
        assert!(masked(&b, &dex, side, "spore"));
        assert!(!masked(&b, &dex, side, "snore"));
    }
}
