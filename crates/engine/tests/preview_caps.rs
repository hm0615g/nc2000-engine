use nc2000_engine::battle::PokemonSet;
use nc2000_engine::dex::Dex;
use nc2000_engine::state::Battle;

fn dex() -> Dex {
    Dex::from_json(include_str!("../../../data/gen2stadium2.json")).unwrap()
}
fn team(level: u8) -> Vec<PokemonSet> {
    (0..6).map(|_| serde_json::from_value(serde_json::json!({
        "species": "Snorlax", "name": "Snorlax", "moves": ["Tackle"], "level": level,
        "item": "", "ability": "No Ability"
    })).unwrap()).collect()
}

#[test]
fn relaxed_preview_is_side_specific_and_survives_cloning() {
    let dex = dex();
    let mut battle = Battle::from_fixture(&dex, "1,2,3,4", &team(55), &team(55)).unwrap();
    assert!(battle.legal_choices(&dex, 0).is_empty());
    assert!(battle.choose(&dex, 0, "team 1,2,3").is_err());
    let standard_key = battle.state_key();
    battle.preview_level_caps[0] = None;
    assert_ne!(standard_key, battle.state_key());
    assert_eq!(battle.legal_choices(&dex, 0).len(), 120);
    assert!(battle.legal_choices(&dex, 1).is_empty());
    let mut clone = battle.clone();
    for choice in battle.legal_choices(&dex, 0) {
        let mut candidate = clone.clone();
        candidate.choose(&dex, 0, &choice.to_input(&dex)).unwrap();
    }
    clone.choose(&dex, 0, "team 1,2,3").unwrap();
    assert!(clone.choose(&dex, 1, "team 1,2,3").is_err());
    clone.preview_level_caps[1] = None;
    clone.choose(&dex, 1, "team 1,2,3").unwrap();
    assert_eq!(clone.turn, 1);
}

#[test]
fn ordinary_preview_keeps_level_limit_and_smaller_parties_work() {
    let dex = dex();
    let mut sets = team(50);
    sets[0].level = 55;
    sets[1].level = 55;
    let mut battle = Battle::from_fixture(&dex, "1,2,3,4", &sets, &sets).unwrap();
    assert!(battle.choose(&dex, 0, "team 1,2,3").is_err());
    battle.choose(&dex, 0, "team 1,3,4").unwrap();
    battle.choose(&dex, 1, "team 1,3,4").unwrap();
    let mut small = Battle::from_fixture(&dex, "1,2,3,4", &sets[..1], &sets[..2]).unwrap();
    small.choose(&dex, 0, "team 1").unwrap();
    small.choose(&dex, 1, "team 1,2").unwrap();
    assert_eq!(small.turn, 1);
}
