use nc2000_engine::battle::{PokemonSet, SearchChoice};
use nc2000_engine::dex::{toid, Dex};
use nc2000_engine::state::Battle;

fn dex() -> Dex {
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/gen2stadium2.json");
    Dex::from_json(&std::fs::read_to_string(path).unwrap()).unwrap()
}

fn set(species: &str, moves: &[&str]) -> PokemonSet {
    serde_json::from_value(serde_json::json!({
        "species": species, "name": species, "level": 50,
        "moves": moves, "item": "", "ability": "No Ability",
        "evs": {"hp":255,"atk":255,"def":255,"spa":255,"spd":255,"spe":255},
        "ivs": {"hp":30,"atk":30,"def":30,"spa":30,"spd":30,"spe":30}
    }))
    .unwrap()
}

fn battle(dex: &Dex, trap: &str, victim: &str) -> Battle {
    let source = [
        set("Smeargle", &[trap, "Splash"]),
        set("Snorlax", &["Rest"]),
        set("Skarmory", &["Rest"]),
    ];
    let target = [
        set(victim, &["Rest"]),
        set("Starmie", &["Recover"]),
        set("Tauros", &["Rest"]),
    ];
    let mut b = Battle::from_fixture(dex, "1,2,3,4", &source, &target).unwrap();
    b.choose(dex, 0, "team 1,2,3").unwrap();
    b.choose(dex, 1, "team 1,2,3").unwrap();
    b
}

fn switches(b: &mut Battle, dex: &Dex) -> usize {
    b.legal_choices(dex, 1)
        .iter()
        .filter(|a| matches!(a, SearchChoice::Switch(_)))
        .count()
}

fn trapping_lifecycle(trap: &str, victim: &str) {
    let dex = dex();
    let mut b = battle(&dex, trap, victim);
    let target = b.active_id(1).unwrap();
    let binding = matches!(trap, "Fire Spin" | "Clamp");
    let condition = dex
        .conds_id(if binding {
            "partiallytrapped"
        } else {
            "trapped"
        })
        .unwrap();
    assert_eq!(switches(&mut b, &dex), 2);
    for _ in 0..10 {
        b.choose(&dex, 0, &format!("move {}", toid(trap))).unwrap();
        b.choose(&dex, 1, "move rest").unwrap();
        if b.poke(target).has_volatile(condition) {
            break;
        }
    }
    assert!(
        b.poke(target).has_volatile(condition),
        "{trap} must land on {victim}: {:?}",
        b.log
    );
    assert!(b.poke(target).trapped, "{trap} must trap {victim}");
    assert_eq!(switches(&mut b, &dex), 0);
    let mut illegal = b.clone();
    assert!(illegal.choose(&dex, 1, "switch 2").is_err());

    if binding {
        let mut expired = b.clone();
        for _ in 0..6 {
            expired.choose(&dex, 0, "move splash").unwrap();
            expired.choose(&dex, 1, "move rest").unwrap();
            if !expired.poke(target).has_volatile(condition) {
                break;
            }
            assert!(expired.poke(target).trapped);
            assert_eq!(switches(&mut expired, &dex), 0);
        }
        assert!(!expired.poke(target).fainted);
        assert!(!expired.poke(target).has_volatile(condition));
        assert!(!expired.poke(target).trapped);
        assert_eq!(switches(&mut expired, &dex), 2);
        expired.choose(&dex, 1, "switch 2").unwrap();
    }

    b.choose(&dex, 0, "switch 2").unwrap();
    b.choose(&dex, 1, "move rest").unwrap();
    assert!(!b.poke(target).has_volatile(condition));
    assert!(!b.poke(target).trapped);
    assert_eq!(switches(&mut b, &dex), 2);
    b.choose(&dex, 1, "switch 2").unwrap();
}

macro_rules! trap_case {
    ($name:ident, $trap:literal, $victim:literal) => {
        #[test]
        fn $name() {
            trapping_lifecycle($trap, $victim);
        }
    };
}

trap_case!(mean_look_gengar, "Mean Look", "Gengar");
trap_case!(mean_look_misdreavus, "Mean Look", "Misdreavus");
trap_case!(mean_look_snorlax, "Mean Look", "Snorlax");
trap_case!(spider_web_gengar, "Spider Web", "Gengar");
trap_case!(spider_web_misdreavus, "Spider Web", "Misdreavus");
trap_case!(spider_web_snorlax, "Spider Web", "Snorlax");
trap_case!(fire_spin_gengar, "Fire Spin", "Gengar");
trap_case!(fire_spin_misdreavus, "Fire Spin", "Misdreavus");
trap_case!(fire_spin_snorlax, "Fire Spin", "Snorlax");
trap_case!(clamp_gengar, "Clamp", "Gengar");
trap_case!(clamp_misdreavus, "Clamp", "Misdreavus");
trap_case!(clamp_snorlax, "Clamp", "Snorlax");

#[test]
fn try_trap_agrees_with_gen2_status_immunity() {
    let dex = dex();
    let mut mismatches = Vec::new();
    for species in ["Gengar", "Misdreavus", "Snorlax"] {
        let mut b = battle(&dex, "Mean Look", species);
        let target = b.active_id(1).unwrap();
        assert!(b.run_status_immunity(&dex, target, "trapped", false));
        if !b.try_trap(&dex, target) || !b.poke(target).trapped {
            mismatches.push(species);
        }
    }
    assert!(mismatches.is_empty(), "dex allows trapping {mismatches:?}");
}
