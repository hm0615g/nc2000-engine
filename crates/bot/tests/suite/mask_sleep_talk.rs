//! What the awake-Sleep-Talk mask rule (`noop_reason`, smmcts.rs) must and
//! must not do.

use super::common;
use nc2000_bot::smmcts::{dominated_actions, dominated_actions_with, MaskRules};
use nc2000_engine::battle::{EffectHandle, PokemonSet, SearchChoice};
use nc2000_engine::dex::Dex;
use nc2000_engine::state::{Battle, PokeId, Status, DK};

fn team_snore_vs_ghost() -> (Vec<PokemonSet>, Vec<PokemonSet>) {
    let mine: Vec<PokemonSet> = serde_json::from_str(
        r#"[
        {"name":"Suicune","species":"Suicune","item":"","ability":"No Ability",
         "moves":["Surf","Sleep Talk","Snore","Rest"],
         "nature":"Serious","evs":{"hp":255,"atk":255,"def":255,"spa":255,"spd":255,"spe":255},"gender":"N","level":50},
        {"name":"Snorlax","species":"Snorlax","item":"","ability":"No Ability",
         "moves":["Splash","Sleep Powder","Body Slam","Rest"],
         "nature":"Serious","evs":{"hp":255,"atk":255,"def":255,"spa":255,"spd":255,"spe":255},"gender":"M","level":50},
        {"name":"Exeggutor","species":"Exeggutor","item":"","ability":"No Ability",
         "moves":["Splash","Sleep Powder","Psychic","Rest"],
         "nature":"Serious","evs":{"hp":255,"atk":255,"def":255,"spa":255,"spd":255,"spe":255},"gender":"M","level":50}
    ]"#,
    )
    .unwrap();
    let theirs: Vec<PokemonSet> = serde_json::from_str(
        r#"[
        {"name":"Misdreavus","species":"Misdreavus","item":"","ability":"No Ability",
         "moves":["Splash","Rest","Curse","Perish Song"],
         "nature":"Serious","evs":{"hp":255,"atk":255,"def":255,"spa":255,"spd":255,"spe":255},"gender":"F","level":50},
        {"name":"Snorlax","species":"Snorlax","item":"","ability":"No Ability",
         "moves":["Splash","Body Slam","Rest","Sleep Powder"],
         "nature":"Serious","evs":{"hp":255,"atk":255,"def":255,"spa":255,"spd":255,"spe":255},"gender":"M","level":50},
        {"name":"Exeggutor","species":"Exeggutor","item":"","ability":"No Ability",
         "moves":["Splash","Psychic","Rest","Sleep Powder"],
         "nature":"Serious","evs":{"hp":255,"atk":255,"def":255,"spa":255,"spd":255,"spe":255},"gender":"M","level":50}
    ]"#,
    )
    .unwrap();
    (mine, theirs)
}

fn setup() -> (&'static Dex, Battle) {
    let dex = common::dex();
    let (a, b) = team_snore_vs_ghost();
    let mut bt = Battle::from_fixture(dex, "7,8,9,10", &a, &b).unwrap();
    bt.set_log_enabled(false);
    bt.choose(dex, 0, "team 1, 2, 3").unwrap();
    bt.choose(dex, 1, "team 1, 2, 3").unwrap();
    (dex, bt)
}

fn strand(b: &mut Battle, side: usize) {
    let party = b.sides[side].party.clone();
    for &slot in party.iter().skip(1) {
        let id = PokeId { side: side as u8, slot };
        b.poke_mut(id).hp = 0;
        b.poke_mut(id).fainted = true;
    }
    b.sides[side].pokemon_left = 1;
}

fn reasons(v: Vec<(SearchChoice, &'static str)>, dex: &Dex, key: &str) -> Vec<&'static str> {
    let id = dex.moves.id(key).unwrap();
    v.into_iter()
        .filter(|(c, _)| matches!(c, SearchChoice::Move(m) if *m == id))
        .map(|(_, w)| w)
        .collect()
}

#[test]
fn no_shadowing_of_downstream_rules() {
    let (dex, mut b) = setup();
    strand(&mut b, 1);
    let me = b.active_id(0).unwrap();
    let foe = b.active_id(1).unwrap();

    // (a) user ASLEEP: the rule must not stop downstream type-immunity scan.
    let mut asleep = b.clone();
    asleep.set_status(dex, me, "slp", Some(me), EffectHandle::None, true);
    asleep.poke_mut(me).status_state.set_int(DK::Time, 3);
    let shipped = reasons(dominated_actions(&asleep, dex, 0), dex, "snore");
    let legacy = reasons(
        dominated_actions_with(
            &asleep,
            dex,
            0,
            MaskRules {
                sleep_talk_awake: false,
                ..MaskRules::default()
            },
        ),
        dex,
        "snore",
    );

    // (b) user AWAKE but SLOWER: same non-shadowing guarantee.
    let mut slow = b.clone();
    slow.poke_mut(foe).boosts[4] = 6;
    let shipped_s = reasons(dominated_actions(&slow, dex, 0), dex, "snore");
    let legacy_s = reasons(
        dominated_actions_with(
            &slow,
            dex,
            0,
            MaskRules {
                sleep_talk_awake: false,
                ..MaskRules::default()
            },
        ),
        dex,
        "snore",
    );

    assert!(!legacy.is_empty(), "legacy mask must refuse Snore into a stranded Ghost");
    assert_eq!(shipped, legacy, "the awake-Sleep-Talk rule shadowed the type-immunity rule");
    assert_eq!(shipped_s, legacy_s, "same, for an awake but slower user");
}

#[test]
fn wake_turn_sleep_talk_is_dead_and_deliberately_unmasked() {
    let (dex, b) = setup();
    let me = b.active_id(0).unwrap();
    let mut w = b.clone();
    w.set_status(dex, me, "slp", Some(me), EffectHandle::None, true);
    w.poke_mut(me).status_state.set_int(DK::Time, 1);
    let refused = reasons(dominated_actions(&w, dex, 0), dex, "sleeptalk");
    w.choose(dex, 0, "move sleeptalk").unwrap();
    w.choose(dex, 1, "move splash").unwrap();
    assert!(refused.is_empty(), "mask says nothing on the wake turn");
    assert_eq!(w.poke(me).status, Status::None, "it woke up");
}
