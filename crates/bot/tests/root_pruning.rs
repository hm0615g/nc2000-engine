use conformance::fixture::repo_root;
use conformance::load_dex;
use nc2000_bot::preview::load_meta_pool;
use nc2000_bot::{Belief, BlindSearch, Observer, RmConfig};
use nc2000_engine::battle::SearchChoice;
use nc2000_engine::dex::Dex;
use nc2000_engine::state::Battle;

fn position() -> (Dex, Battle, Observer, Belief) {
    let dex = load_dex();
    let pool = load_meta_pool(&repo_root().join("data/meta-pool-v0/meta-pool.json"));
    let mut own = pool.teams[0].sets.clone();
    for set in &mut own {
        set.moves = ["Surf", "Rest", "Sleep Talk", "Snore"]
            .map(String::from)
            .to_vec();
    }
    let foe = &pool.teams[1].sets;
    let mut battle = Battle::from_fixture(&dex, "19,23,29,31", &own, foe).unwrap();
    let mut observed = Observer::new(&battle, 0);
    let picks = [
        Some(battle.legal_choices(&dex, 0)[0]),
        Some(battle.legal_choices(&dex, 1)[0]),
    ];
    battle.apply_choices(&dex, picks).unwrap();
    let active = battle.active_id(0).unwrap();
    battle.poke_mut(active).boosts[4] = 6;
    observed.observe(&battle, &dex);
    let belief = Belief::pinned(&dex, "foe", foe, &observed);
    (dex, battle, observed, belief)
}

#[test]
fn final_choice_exclusions_can_also_exclude_search_allocation() {
    let (dex, battle, observed, belief) = position();
    let mut search = BlindSearch::new(&battle, &dex, RmConfig::default(), 0, 37);
    let rest = SearchChoice::Move(dex.moves.id("rest").unwrap());
    let index = search.actions().iter().position(|&a| a == rest).unwrap();
    assert!(search.dominated()[index]);
    search.prune_dominated();
    let mut calls = 0;
    search.step_with_leaf(&dex, &belief, &observed, 64, &mut |sim, _, _| {
        assert!(sim.turn >= battle.turn);
        calls += 1;
        0.5
    });
    assert!(calls > 0);
    assert_eq!(search.visits()[index], 0);
    assert_eq!(search.visits().iter().sum::<u32>(), 64);
    assert_ne!(search.best(), Some(rest));
}

#[test]
fn pruning_preserves_the_only_request_allowed_action() {
    let (dex, battle, observed, belief) = position();
    let mut search = BlindSearch::new(&battle, &dex, RmConfig::default(), 0, 41);
    let rest = SearchChoice::Move(dex.moves.id("rest").unwrap());
    let allowed: Vec<_> = search.actions().iter().map(|&a| a == rest).collect();
    search.mask_actions(&allowed);
    search.prune_dominated();
    search.step_with_leaf(&dex, &belief, &observed, 8, &mut |_, _, _| 0.5);
    assert_eq!(search.best(), Some(rest));
}
