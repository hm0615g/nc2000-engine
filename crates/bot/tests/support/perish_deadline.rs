use nc2000_engine::{
    battle::{PokemonSet, SearchChoice},
    dex::Dex,
    state::Battle,
};

#[derive(Clone, Copy, Debug)]
pub enum WaitingMove {
    Return,
    Snore,
}

impl WaitingMove {
    pub fn key(self) -> &'static str {
        match self {
            Self::Return => "return",
            Self::Snore => "snore",
        }
    }
}

fn set(species: &str, moves: &[&str]) -> PokemonSet {
    serde_json::from_value(serde_json::json!({
        "species":species,"name":species,"level":50,"moves":moves,
        "item":"","ability":"No Ability","happiness":255,
        "evs":{"hp":255,"atk":255,"def":255,"spa":255,"spd":255,"spe":255},
        "ivs":{"hp":30,"atk":30,"def":30,"spa":30,"spd":30,"spe":30}
    }))
    .unwrap()
}

fn action(b: &mut Battle, dex: &Dex, side: usize, input: &str) -> SearchChoice {
    b.legal_choices(dex, side)
        .into_iter()
        .find(|a| a.to_input(dex) == input)
        .unwrap_or_else(|| panic!("illegal fixture action for side {side}: {input}"))
}

fn step(b: &mut Battle, dex: &Dex, own: &str, foe: &str) {
    let joint = [Some(action(b, dex, 0, own)), Some(action(b, dex, 1, foe))];
    b.apply_choices(dex, joint).unwrap();
}

pub fn position(dex: &Dex, wait: WaitingMove) -> (Battle, Battle) {
    let mut misdreavus = set(
        "Misdreavus",
        &["Mean Look", wait.key(), "Shadow Ball", "Psychic"],
    );
    misdreavus.level = 55;
    misdreavus.item = "Leftovers".into();
    let mut gengar = set("Gengar", &["Destiny Bond", "Perish Song", "Night Shade"]);
    for stat in ["spa", "spd"] {
        gengar.evs.as_mut().unwrap().insert(stat.into(), 0);
    }
    if matches!(wait, WaitingMove::Snore) {
        gengar.evs.as_mut().unwrap().insert("spe".into(), 0);
    }
    let teams = [
        vec![misdreavus, set("Snorlax", &["Self-Destruct"])],
        vec![gengar, set("Magikarp", &["Splash"])],
    ];
    let preview = Battle::from_fixture(dex, "1,2,3,4", &teams[0], &teams[1]).unwrap();
    let mut b = preview.clone();
    b.choose(dex, 0, "team 1,2").unwrap();
    b.choose(dex, 1, "team 1,2").unwrap();
    step(&mut b, dex, "move psychic", "move nightshade");
    for _ in 0..7 {
        step(&mut b, dex, "move meanlook", "move destinybond");
    }
    step(&mut b, dex, "switch 2", "move perishsong");
    step(&mut b, dex, "move selfdestruct", "move nightshade");
    let replacement = action(&mut b, dex, 0, "switch 2");
    b.apply_choices(dex, [Some(replacement), None]).unwrap();
    step(
        &mut b,
        dex,
        "move meanlook",
        match wait {
            WaitingMove::Return => "move nightshade",
            WaitingMove::Snore => "move destinybond",
        },
    );

    let own = b.poke(b.active_id(0).unwrap());
    let foe = b.poke(b.active_id(1).unwrap());
    let perish = dex.conds_id("perishsong").unwrap();
    let bond = dex.conds_id("destinybond").unwrap();
    let pp = |p: &nc2000_engine::state::Pokemon, key: &str| {
        p.move_slots
            .iter()
            .find(|slot| dex.moves.key(slot.id) == key)
            .unwrap()
            .pp
    };
    assert_eq!(b.sides[0].pokemon_left, 1);
    assert_eq!(b.sides[1].pokemon_left, 2);
    assert!(!own.has_volatile(perish));
    assert_eq!(foe.volatile(perish).and_then(|v| v.duration), Some(1));
    assert_eq!(pp(own, "meanlook"), 0);
    assert!(foe.trapped);
    assert!(!own.fainted && !foe.fainted);
    assert!(b.sides[1]
        .party
        .iter()
        .copied()
        .filter(|&slot| slot != b.active_id(1).unwrap().slot)
        .any(|slot| {
            let p = &b.sides[1].roster[slot as usize];
            !p.fainted && p.hp > 0
        }));
    match wait {
        WaitingMove::Return => {
            assert!(foe.speed > own.speed);
            assert!(!foe.has_volatile(bond));
            assert_eq!(pp(foe, "destinybond"), 1);
        }
        WaitingMove::Snore => {
            assert!(own.speed > foe.speed);
            assert!(foe.has_volatile(bond));
            assert_eq!(pp(foe, "destinybond"), 0);
        }
    }
    (preview, b)
}
