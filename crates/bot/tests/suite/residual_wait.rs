use super::common;
use nc2000_bot::{
    smmcts::{dominated_actions, SelRule},
    Belief, BlindSearch, Observer, RmConfig,
};
use nc2000_engine::{
    battle::{enumerate::enumerate_step, EffectHandle, Outcome, PokemonSet},
    state::{Battle, PokeId, Status},
};
use serde_json::json;

#[path = "../support/residual_wait.rs"]
mod fixture;

#[test]
fn preserves_a_certified_winning_wait_without_changing_search() {
    let dex = common::dex();
    let (preview, b, _) = fixture::position(&dex);
    let mut observer = Observer::new(&preview, 0);
    let mut belief = Belief::pinned_from_battle(&preview, &observer);
    observer.observe(&b, &dex);
    belief.sync(&dex, &observer);
    let mut search = BlindSearch::new(
        &b,
        &dex,
        RmConfig {
            c: 0.4,
            rule: SelRule::Ucb,
            ..Default::default()
        },
        0,
        81001,
    );
    search.step(&dex, &belief, &observer, 3000);
    let chosen = search.best().unwrap();
    assert!(matches!(
        chosen.to_input(&dex).as_str(),
        "move toxic" | "move thunderwave"
    ));
    let mut probe = b.clone();
    for reply in probe.legal_choices(&dex, 1) {
        let win = enumerate_step(&dex, &b, [Some(chosen), Some(reply)], 10000).unwrap();
        assert!(win
            .leaves
            .iter()
            .all(|l| l.battle.outcome() == Some(Outcome::P1Win)));
    }
}

#[test]
fn public_residual_effects_relax_only_noop_exclusions() {
    let dex = common::dex();
    let set = |species: &str, moves: &[&str]| -> PokemonSet {
        serde_json::from_value(json!({"species":species,"name":species,"level":50,"moves":moves}))
            .unwrap()
    };
    for side in 0..2 {
        let own = vec![
            set("Smeargle", &["Snore", "Explosion", "Spore"]),
            set("Magikarp", &["Splash"]),
        ];
        let foe = vec![set("Snorlax", &["Rest"]), set("Magikarp", &["Splash"])];
        let teams = if side == 0 { [own, foe] } else { [foe, own] };
        let mut base = Battle::from_fixture(&dex, "1,2,3,4", &teams[0], &teams[1]).unwrap();
        base.choose(&dex, 0, "team 1,2").unwrap();
        base.choose(&dex, 1, "team 1,2").unwrap();
        let me = base.active_id(side).unwrap();
        let them = base.active_id(1 - side).unwrap();
        let own_bench = PokeId {
            side: side as u8,
            slot: base.sides[side].party[1],
        };
        let foe_bench = PokeId {
            side: (1 - side) as u8,
            slot: base.sides[1 - side].party[1],
        };
        base.poke_mut(own_bench).hp = 0;
        base.poke_mut(own_bench).fainted = true;
        base.sides[side].pokemon_left = 1;
        base.restore_status(&dex, foe_bench, Status::Slp, Some(me));
        for target in [me, them, foe_bench] {
            for effect in [
                "none",
                "psn",
                "tox",
                "brn",
                "par",
                "curse",
                "leechseed",
                "partiallytrapped",
                "nightmare",
                "awake_nightmare",
                "sandstorm",
                "rain",
            ] {
                let mut b = base.clone();
                match effect {
                    "psn" => {
                        b.restore_status(&dex, target, Status::Psn, None);
                    }
                    "tox" => {
                        b.restore_status(&dex, target, Status::Tox, None);
                    }
                    "brn" => {
                        b.restore_status(&dex, target, Status::Brn, None);
                    }
                    "par" => {
                        b.restore_status(&dex, target, Status::Par, None);
                    }
                    "nightmare" | "awake_nightmare" => {
                        if effect == "nightmare" {
                            b.restore_status(&dex, target, Status::Slp, None);
                        }
                        b.add_volatile(&dex, target, "nightmare", None, EffectHandle::None);
                    }
                    "curse" | "leechseed" | "partiallytrapped" => {
                        b.add_volatile(&dex, target, effect, Some(me), EffectHandle::None);
                    }
                    "sandstorm" | "rain" => {
                        b.field.weather = dex.conds_id(if effect == "rain" {
                            "raindance"
                        } else {
                            effect
                        });
                    }
                    _ => {}
                }
                if target == me && matches!(effect, "par" | "nightmare") {
                    continue;
                }
                b.refresh_battle_mask(&dex);
                let excluded = dominated_actions(&b, &dex, side);
                let masked = |key| {
                    excluded
                        .iter()
                        .any(|(a, _)| a.to_input(&dex) == format!("move {key}"))
                };
                let residual = effect == "sandstorm"
                    || (target == them
                        && !matches!(effect, "none" | "par" | "awake_nightmare" | "rain"));
                assert_eq!(
                    masked("snore"),
                    !residual,
                    "{side} {target:?} {effect}"
                );
                assert!(masked("explosion"));
                if b.has_sleeping_pokemon(1 - side) {
                    assert!(masked("spore"));
                }
            }
        }
        let mut sand = base.clone();
        sand.field.weather = dex.conds_id("sandstorm");
        sand.refresh_battle_mask(&dex);
        sand.poke_mut(them).types = nc2000_engine::dex::TypeList::one(dex.known_types.ground);
        assert!(
            dominated_actions(&sand, &dex, side)
                .iter()
                .any(|(a, _)| a.to_input(&dex) == "move snore")
        );
    }
}
