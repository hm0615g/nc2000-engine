//! M8 meta pool Max Total Level preview invariant.

use crate::suite::common;
use conformance::fixture::repo_root;
use nc2000_engine::battle::PokemonSet;
use nc2000_engine::state::Battle;

#[derive(serde::Deserialize)]
struct MetaPool {
    teams: Vec<MetaTeam>,
}

#[derive(serde::Deserialize)]
struct MetaTeam {
    id: String,
    sets: Vec<PokemonSet>,
}

#[test]
fn meta_pool_preview_respects_max_total_level() {
    let path = repo_root().join("data/meta-pool-v0/meta-pool.json");
    let pool: MetaPool =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    let dex = common::dex();
    let mut affected = 0;
    for team in &pool.teams {
        let levels: Vec<u32> = team.sets.iter().map(|s| s.level as u32).collect();
        let mut legal_subsets = 0u32;
        for a in 0..6 {
            for b in a + 1..6 {
                for c in b + 1..6 {
                    if levels[a] + levels[b] + levels[c] <= 155 {
                        legal_subsets += 1;
                    }
                }
            }
        }
        if legal_subsets < 20 {
            affected += 1;
        }
        let mut battle =
            Battle::from_fixture(dex, "1,2,3,4", &team.sets, &team.sets).unwrap();
        battle.set_log_enabled(false);
        let choices = battle.legal_choices(dex, 0);
        for c in &choices {
            let nc2000_engine::battle::SearchChoice::Team(slots) = c else {
                panic!("{}: non-team choice at preview", team.id);
            };
            let sum: u32 = slots.iter().map(|&s| levels[s as usize - 1]).sum();
            assert!(sum <= 155, "{}: enumerated illegal pick {slots:?} (sum {sum})", team.id);
        }
        assert_eq!(
            choices.len() as u32,
            legal_subsets * 6,
            "{}: enumerated {} preview picks, expected {} (= {legal_subsets} legal subsets x 6 orderings)",
            team.id,
            choices.len(),
            legal_subsets * 6,
        );
    }
    assert_eq!(affected, 17, "affected-team count drifted from the audit");
}
