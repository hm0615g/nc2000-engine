use conformance::fixture::repo_root;
use conformance::load_dex;
use nc2000_bot::pick_prior::{PickPrior, PreviewMon};
use nc2000_bot::preview::load_meta_pool;
use nc2000_bot::{Belief, Observer, SplitMix64};
use nc2000_engine::battle::SearchChoice;
use nc2000_engine::state::Battle;
use serde_json::json;

#[test]
fn hidden_true_picks_cannot_change_the_sampled_bench() {
    let dex = load_dex();
    let pool = load_meta_pool(&repo_root().join("data/meta-pool-v0/meta-pool.json"));
    let mut battle =
        Battle::from_fixture(&dex, "1,2,3,4", &pool.teams[0].sets, &pool.teams[1].sets).unwrap();
    let preview: Vec<PreviewMon> = battle.sides[0]
        .roster
        .iter()
        .map(|mon| PreviewMon {
            species: dex.species.key(mon.species).into(),
            level: mon.level,
            gender: mon.gender.as_str().into(),
            item: mon.item.is_some(),
        })
        .collect();
    let enemy_preview: Vec<_> = preview
        .iter()
        .map(|m| {
            json!({
                "species": m.species, "level": m.level, "gender": m.gender, "item": m.item,
            })
        })
        .collect();
    let choices = battle.legal_choices(&dex, 1);
    let first = choices[0];
    let SearchChoice::Team(first_slots) = first else {
        panic!()
    };
    let second = *choices
        .iter()
        .find(|choice| match choice {
            SearchChoice::Team(slots) => {
                slots[0] == first_slots[0] && slots.iter().any(|slot| !first_slots.contains(slot))
            }
            _ => false,
        })
        .unwrap();
    let preferred: Vec<_> = first_slots
        .iter()
        .map(|slot| {
            dex.species
                .key(battle.sides[1].roster[*slot as usize - 1].species)
        })
        .collect();
    let prior = PickPrior::from_json(
        &json!({
            "schema": "nc2000-pick-prior-v1", "smoothing": 0.2, "source": {},
            "rows": [{"team": pool.teams[1].id, "side": 1,
                "enemy_preview": enemy_preview,
                "choices": [{"species": preferred, "count": 4}]}],
        })
        .to_string(),
    )
    .unwrap();
    let own = battle.legal_choices(&dex, 0)[0];
    let mut states = [battle.clone(), battle];
    let mut observations = [Observer::new(&states[0], 0), Observer::new(&states[1], 0)];
    let mut beliefs = [
        Belief::new(&dex, &pool, &observations[0]),
        Belief::new(&dex, &pool, &observations[1]),
    ];
    for (index, choice) in [first, second].into_iter().enumerate() {
        states[index]
            .apply_choices(&dex, [Some(own), Some(choice)])
            .unwrap();
        observations[index].observe(&states[index], &dex);
        beliefs[index].sync(&dex, &observations[index]);
        beliefs[index].condition_pick_prior(&dex, &prior, &preview, &observations[index]);
    }
    assert_ne!(states[0].sides[1].party, states[1].sides[1].party);
    let mut favored = 0;
    for seed in 0..400 {
        let samples: Vec<_> = (0..2)
            .map(|i| {
                beliefs[i].determinize_with(
                    &dex,
                    &states[i],
                    &observations[i],
                    Some(1),
                    &mut SplitMix64::new(seed),
                )
            })
            .collect();
        assert_eq!(samples[0].sides[1].party, samples[1].sides[1].party);
        favored += usize::from(
            samples[0].sides[1]
                .party
                .iter()
                .all(|slot| first_slots.contains(&(slot + 1))),
        );
        for sample in &samples {
            let party = &sample.sides[1].party;
            assert_eq!(party.len(), 3);
            assert!(party.contains(&(first_slots[0] - 1)));
            for (position, &slot) in party.iter().enumerate() {
                assert_eq!(
                    sample.sides[1].roster[slot as usize].position as usize,
                    position
                );
                assert_eq!(party.iter().filter(|&&other| other == slot).count(), 1);
            }
        }
    }
    assert!(favored > 330, "prior was not used: {favored}/400");
}
