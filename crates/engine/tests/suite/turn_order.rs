//! Deterministic turn-order edge cases for simultaneous phazing and wake-turn ordering.

use crate::suite::common;
use nc2000_engine::battle::PokemonSet;
use nc2000_engine::dex::Dex;
use nc2000_engine::state::{Battle, PokeId, Status};

fn mk(species: &str, level: u8, item: &str, moves: &[&str]) -> PokemonSet {
    serde_json::from_value(serde_json::json!({
        "name": species, "species": species, "item": item, "ability": "No Ability",
        "moves": moves, "level": level,
        "evs": {"hp":255,"atk":255,"def":255,"spa":255,"spd":255,"spe":255},
        "ivs": {"hp":30,"atk":30,"def":30,"spa":30,"spd":30,"spe":30},
        "happiness": 255
    }))
    .unwrap()
}

fn start(seed: &str, p1: &[PokemonSet], p2: &[PokemonSet]) -> (&'static Dex, Battle) {
    let d = common::dex();
    let mut b = Battle::from_fixture(d, seed, p1, p2).unwrap();
    b.choose(d, 0, "team 1,2,3").unwrap();
    b.choose(d, 1, "team 1,2,3").unwrap();
    (d, b)
}

fn act(b: &Battle, side: usize) -> PokeId {
    b.active_id(side).unwrap()
}

/// Whirlwind/Roar fail when a move action is still queued (gen2 onTryHit):
/// two -1-priority phazes in the same turn -> the FASTER one fails.
#[test]
fn q1_faster_whirlwind_fails_when_the_slower_one_is_still_queued() {
    let p1 = vec![
        mk("Misdreavus", 50, "Miracle Berry", &["Whirlwind", "Rest", "Night Shade"]),
        mk("Skarmory", 55, "Leftovers", &["Drill Peck", "Rest"]),
        mk("Blissey", 50, "Leftovers", &["Soft-Boiled", "Toxic"]),
    ];
    let p2 = vec![
        mk("Zapdos", 50, "Miracle Berry", &["Whirlwind", "Thunderbolt"]),
        mk("Snorlax", 50, "Mint Berry", &["Body Slam", "Rest"]),
        mk("Miltank", 55, "Bright Powder", &["Return", "Curse"]),
    ];
    let (d, mut b) = start("1,2,3,4", &p1, &p2);
    let m0 = b.log.len();
    b.choose(d, 0, "move whirlwind").unwrap();
    b.choose(d, 1, "move whirlwind").unwrap();
    let tail: Vec<String> = b.log[m0..].to_vec();
    let fast = tail.iter().position(|l| l.starts_with("|move|p2a") && l.contains("Whirlwind")).unwrap();
    let slow = tail.iter().position(|l| l.starts_with("|move|p1a") && l.contains("Whirlwind")).unwrap();
    assert!(fast < slow, "Zapdos L50 is the faster phazer: {tail:?}");
    assert_eq!(tail[fast + 1], "|-fail|p1a: Misdreavus", "faster phaze fails: {tail:?}");
    assert!(
        tail[slow..].iter().any(|l| l.starts_with("|drag|p2a")),
        "the slower phaze lands: {tail:?}"
    );
    assert!(
        !tail[..slow].iter().any(|l| l.starts_with("|drag|")),
        "no drag before the slower phaze: {tail:?}"
    );
}

/// Rest = 3 sleep "time" units: two turns fully skipped, acting on the third.
/// On the wake-up turn a FASTER foe's move resolves while the target is still asleep.
#[test]
fn q3_rest_sleeps_two_turns_and_wakes_inside_its_own_beforemove() {
    let p1 = vec![
        mk("Umbreon", 50, "Leftovers", &["Rest", "Toxic", "Charm"]),
        mk("Blissey", 50, "Leftovers", &["Soft-Boiled", "Toxic"]),
        mk("Skarmory", 55, "Leftovers", &["Drill Peck", "Rest"]),
    ];
    let p2 = vec![
        mk("Suicune", 51, "Bright Powder", &["Surf", "Ice Beam", "Rest"]),
        mk("Snorlax", 50, "Mint Berry", &["Body Slam", "Rest"]),
        mk("Steelix", 50, "Leftovers", &["Earthquake", "Rest"]),
    ];
    let (d, mut b) = start("1,2,3,4", &p1, &p2);
    b.choose(d, 0, "move charm").unwrap();
    b.choose(d, 1, "move surf").unwrap();
    b.choose(d, 0, "move rest").unwrap();
    b.choose(d, 1, "move surf").unwrap();
    let sleeper = act(&b, 0);
    assert_eq!(b.poke(sleeper).status, Status::Slp);
    assert_eq!(
        b.poke(sleeper).status_state.get_int(nc2000_engine::state::DK::Time),
        3,
        "Rest sets sleep time 3"
    );
    for _ in 0..2 {
        let m = b.log.len();
        b.choose(d, 0, "move charm").unwrap();
        b.choose(d, 1, "move surf").unwrap();
        assert!(b.log[m..].iter().any(|l| l.contains("|cant|") && l.contains("slp")));
        assert_eq!(b.poke(sleeper).status, Status::Slp);
    }
    let m2 = b.log.len();
    b.choose(d, 0, "move charm").unwrap();
    b.choose(d, 1, "move surf").unwrap();
    let tail: Vec<String> = b.log[m2..].to_vec();
    let dmg = tail.iter().position(|l| l.starts_with("|-damage|p1a")).unwrap();
    let cure = tail.iter().position(|l| l.starts_with("|-curestatus|p1a") && l.contains("slp")).unwrap();
    assert!(dmg < cure, "faster foe's move resolves BEFORE the wake-up: {tail:?}");
    assert!(tail[dmg].contains("slp"), "target still shows slp when hit: {tail:?}");
    assert_eq!(b.poke(sleeper).status, Status::None, "awake at end of turn C");
}

/// Converse: if the SLEEPER is faster, it wakes inside BeforeMove before the slower foe resolves.
#[test]
fn q3_slower_attacker_hits_an_already_awake_sleeper() {
    let p1 = vec![
        mk("Umbreon", 50, "Leftovers", &["Charm", "Toxic", "Rest"]),
        mk("Blissey", 50, "Leftovers", &["Soft-Boiled", "Toxic"]),
        mk("Skarmory", 55, "Leftovers", &["Drill Peck", "Rest"]),
    ];
    let p2 = vec![
        mk("Suicune", 51, "Bright Powder", &["Surf", "Ice Beam", "Rest"]),
        mk("Snorlax", 50, "Mint Berry", &["Body Slam", "Rest"]),
        mk("Steelix", 50, "Leftovers", &["Earthquake", "Rest"]),
    ];
    let (d, mut b) = start("1,2,3,4", &p1, &p2);
    let sleeper = act(&b, 1);
    b.poke_mut(sleeper).hp = b.poke(sleeper).maxhp / 2;
    b.choose(d, 0, "move charm").unwrap();
    b.choose(d, 1, "move rest").unwrap();
    assert_eq!(b.poke(sleeper).status, Status::Slp);
    for _ in 0..2 {
        b.choose(d, 0, "move charm").unwrap();
        b.choose(d, 1, "move surf").unwrap();
        assert_eq!(b.poke(sleeper).status, Status::Slp);
    }
    let m = b.log.len();
    b.choose(d, 0, "move toxic").unwrap();
    b.choose(d, 1, "move surf").unwrap();
    let tail: Vec<String> = b.log[m..].to_vec();
    let cure = tail.iter().position(|l| l.starts_with("|-curestatus|p2a") && l.contains("slp")).unwrap();
    let foe_move = tail.iter().position(|l| l.starts_with("|move|p1a")).unwrap();
    assert!(cure < foe_move, "the faster sleeper wakes first: {tail:?}");
    assert_ne!(b.poke(sleeper).status, Status::Slp);
}

/// An early-game Explosion/Self-Destruct must NOT leave `sides[s].last_move`
/// set into a later 1v1 mutual faint (e.g., Sandstorm double-KO), whereas a
/// genuine 1v1 Explosion mutual KO still triggers the Self-KO Clause loss.
#[test]
fn early_explosion_clears_side_last_move_and_does_not_poison_later_1v1_mutual_ko() {
    use nc2000_engine::battle::Outcome;
    use nc2000_engine::compact::CompactBattle;

    let d = common::dex();
    let p1 = vec![
        mk("Cloyster", 50, "", &["Explosion", "Surf"]),
        mk("Zapdos", 55, "", &["Thunderbolt", "Splash"]),
    ];
    let p2 = vec![
        mk("Snorlax", 50, "", &["Splash", "Rest"]),
        mk("Umbreon", 50, "", &["Sandstorm", "Splash"]),
    ];
    let mut b = Battle::from_fixture(d, "1,2,3,4", &p1, &p2).unwrap();
    b.choose(d, 0, "team 1,2").unwrap();
    b.choose(d, 1, "team 1,2").unwrap();

    // Turn 1 (2v2): P1 Cloyster explodes, KOing both Cloyster and Snorlax.
    let s1 = act(&b, 1);
    b.poke_mut(s1).hp = 1;
    b.choose(d, 0, "move explosion").unwrap();
    b.choose(d, 1, "move splash").unwrap();
    assert_eq!(b.sides[0].last_move, None, "early Explosion must clear side.last_move after faint resolution");
    assert_eq!(b.sides[1].last_move, None);

    // Forced switches into 1v1: Zapdos (faster, P1) vs Umbreon (slower, P2).
    b.choose(d, 0, "switch 2").unwrap();
    b.choose(d, 1, "switch 2").unwrap();
    assert_eq!(b.sides[0].pokemon_left, 1);
    assert_eq!(b.sides[1].pokemon_left, 1);

    // Turn 2 (1v1): Both at 1 HP; Umbreon sets Sandstorm -> mutual faint in Residual.
    // Without the fix, stale `sides[0].last_move` wrongly handed P2 the win via Self-KO Clause.
    // In PS gen2stadium2 `faintMessages`, `faintData` is the slower Umbreon -> `1 - side = P1Win`.
    let z = act(&b, 0);
    let u = act(&b, 1);
    b.poke_mut(z).hp = 1;
    b.poke_mut(u).hp = 1;
    b.choose(d, 0, "move splash").unwrap();
    b.choose(d, 1, "move sandstorm").unwrap();
    assert_eq!(b.outcome(), Some(Outcome::P1Win));

    // Genuine 1v1 Self-KO Clause still rules the Explosion user as the loser.
    let p1_1v1 = vec![mk("Cloyster", 55, "", &["Explosion"])];
    let p2_1v1 = vec![mk("Umbreon", 50, "", &["Splash"])];
    let mut b1 = Battle::from_fixture(d, "1,2,3,4", &p1_1v1, &p2_1v1).unwrap();
    b1.choose(d, 0, "team 1").unwrap();
    b1.choose(d, 1, "team 1").unwrap();
    let u1 = act(&b1, 1);
    b1.poke_mut(u1).hp = 1;
    let (tbl1, mut cb1) = CompactBattle::from_battle(&b1, d);
    b1.choose(d, 0, "move explosion").unwrap();
    b1.choose(d, 1, "move splash").unwrap();
    assert_eq!(b1.outcome(), Some(Outcome::P2Win));
    cb1.step_turn(&tbl1, [0, 0]);
    assert_eq!(cb1.outcome, 2, "1v1 Explosion mutual KO must still be P2Win in CompactBattle");

    // CompactBattle 2v2 early Explosion clears last_move_selfko cleanly.
    let mut b2 = Battle::from_fixture(d, "1,2,3,4", &p1, &p2).unwrap();
    b2.choose(d, 0, "team 1,2").unwrap();
    b2.choose(d, 1, "team 1,2").unwrap();
    let s1b = act(&b2, 1);
    b2.poke_mut(s1b).hp = 1;
    let (tbl2, mut cb2) = CompactBattle::from_battle(&b2, d);
    cb2.step_turn(&tbl2, [0, 0]); // Cloyster Explosion KOs both, auto-switches Zapdos & Umbreon
    assert_eq!(cb2.sides[0].last_move_selfko, u8::MAX);
    assert_eq!(cb2.sides[1].last_move_selfko, u8::MAX);
}
