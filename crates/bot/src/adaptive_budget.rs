//! Opt-in iteration budget for one legacy blind battle decision.
use crate::blind::BlindSearch;
use crate::{Belief, Observer};
use nc2000_engine::{battle::SearchChoice, dex::Dex};

pub const BATTLE_CAP: u32 = 243_000;
const INTERVAL: u32 = 27_000;

#[derive(Default)]
pub(crate) struct AdaptiveBattleBudget {
    previous: Option<(u32, SearchChoice)>,
    stopped: bool,
}

impl AdaptiveBattleBudget {
    /// Absolute checkpoints make the decision independent of caller slice size.
    pub(crate) fn step(
        &mut self,
        search: &mut BlindSearch,
        dex: &Dex,
        belief: &Belief,
        obs: &Observer,
        n: u32,
    ) -> u32 {
        if search.is_preview() {
            return search.step(dex, belief, obs, n);
        }
        let mut done = search.iterations();
        if done == 0 && search.eligible_flags().iter().filter(|&&ok| ok).count() == 1 {
            self.stopped = true;
        }
        let target = done.saturating_add(n).min(BATTLE_CAP);
        while !self.stopped && done < target {
            let next = target.min(done + INTERVAL - done % INTERVAL);
            done = search.step(dex, belief, obs, next - done);
            if done < BATTLE_CAP && done % INTERVAL == 0 {
                let candidate = sole_candidate(search);
                self.record(done, candidate);
            }
        }
        done
    }
    fn record(&mut self, done: u32, candidate: Option<SearchChoice>) {
        self.stopped =
            candidate.is_some_and(|choice| self.previous == Some((done - INTERVAL, choice)));
        self.previous = candidate.map(|choice| (done, choice));
    }
}

fn sole_candidate(search: &BlindSearch) -> Option<SearchChoice> {
    let eligible = search.eligible_flags();
    let visits = search.visits();
    let means = search.means();
    let leader = unique_leader(&eligible, visits, &means)?;
    let choice = search.actions()[leader];
    (search.best() == Some(choice)).then_some(choice)
}

fn unique_leader(eligible: &[bool], visits: &[u32], means: &[f64]) -> Option<usize> {
    let leader = (0..visits.len())
        .filter(|&i| eligible[i])
        .max_by_key(|&i| visits[i])?;
    // Every rival must fall below the Bayesian root LP's visit cutoff. Also
    // require sampled alternatives, a value lead, and the actual endgame-aware pick.
    let cutoff = crate::root_policy::min_equilibrium_visits(visits[leader]);
    for i in 0..visits.len() {
        if i != leader
            && eligible[i]
            && (visits[i] == 0 || visits[i] >= cutoff || means[leader] - means[i] < 0.02)
        {
            return None;
        }
    }
    Some(leader)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_eligible_rivals_need_visits_and_clear_gaps() {
        let both = [true, true];
        assert_eq!(unique_leader(&both, &[800, 99], &[0.7, 0.6]), Some(0));
        assert_eq!(unique_leader(&both, &[99, 800], &[0.5, 0.7]), Some(1));
        assert_eq!(unique_leader(&both, &[800, 100], &[0.7, 0.6]), None);
        assert_eq!(unique_leader(&both, &[800, 0], &[0.7, 0.5]), None);
        assert_eq!(unique_leader(&both, &[800, 99], &[0.7, 0.69]), None);
        assert_eq!(unique_leader(&both, &[800, 800], &[0.9, 0.5]), None);
        assert_eq!(
            unique_leader(&[false, true, true], &[999, 800, 99], &[0.9, 0.7, 0.6]),
            Some(1)
        );
    }

    #[test]
    fn caller_slices_preserve_the_tree_and_the_stopping_iteration() {
        use crate::{smmcts::SelRule, RmConfig};
        use nc2000_engine::{battle::PokemonSet, state::Battle};
        let dex = conformance::load_dex();
        let mon = |species: &str, level, moves: &[&str]| -> PokemonSet {
            serde_json::from_value(serde_json::json!({
                "species": species, "name": species, "level": level, "moves": moves,
            }))
            .unwrap()
        };
        for (own, foe, clear) in [
            (
                mon("Alakazam", 50, &["Psychic", "Splash"]),
                mon("Gengar", 50, &["Shadow Ball"]),
                true,
            ),
            (
                mon("Pikachu", 55, &["Thunderbolt", "Thunder Punch"]),
                mon("Magikarp", 5, &["Splash"]),
                false,
            ),
        ] {
            let mut battle = Battle::from_fixture(&dex, "1,2,3,4", &[own], &[foe]).unwrap();
            for side in 0..2 {
                battle.choose(&dex, side, "team 1").unwrap();
            }
            let obs = Observer::new(&battle, 0);
            let belief = Belief::pinned_from_battle(&battle, &obs);
            let cfg = RmConfig {
                rule: SelRule::Ucb,
                c: 0.4,
                ..Default::default()
            };
            let search = || BlindSearch::new(&battle, &dex, cfg.clone(), 0, 123);
            let mut once = search();
            let done =
                AdaptiveBattleBudget::default().step(&mut once, &dex, &belief, &obs, u32::MAX);
            assert_eq!(done, if clear { 2 * INTERVAL } else { BATTLE_CAP });
            let mut sliced = search();
            let mut budget = AdaptiveBattleBudget::default();
            while sliced.iterations() < done {
                let before = sliced.iterations();
                assert!(budget.step(&mut sliced, &dex, &belief, &obs, 8_321) > before);
            }
            assert_eq!(budget.step(&mut sliced, &dex, &belief, &obs, 1), done);
            assert_eq!(sliced.visits(), once.visits());
            assert_eq!(sliced.search_rng_state(), once.search_rng_state());
            assert_eq!(sliced.best(), once.best());
        }
    }

    #[test]
    fn requires_the_same_candidate_at_adjacent_checkpoints() {
        let mut budget = AdaptiveBattleBudget::default();
        let a = Some(SearchChoice::Switch(1));
        let b = Some(SearchChoice::Switch(2));
        for (step, pick) in [(1, a), (2, None), (3, a), (4, b), (6, b)] {
            budget.record(step * INTERVAL, pick);
            assert!(!budget.stopped);
        }
        budget.record(7 * INTERVAL, b);
        assert!(budget.stopped);
        let live: serde_json::Value =
            serde_json::from_str(include_str!("../../../data/live-search.json")).unwrap();
        assert_eq!(live["battle"]["adaptive"], BATTLE_CAP);
    }
}
