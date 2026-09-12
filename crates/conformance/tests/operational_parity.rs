use conformance::fixture::{corpus_files, repo_root, Fixture};
use conformance::{load_dex, ps_reference_battle};
use nc2000_engine::battle::Outcome;
use nc2000_engine::state::{Battle, Status};

#[test]
fn operational_rules_preserve_reference_state_and_rng_until_a_sleep_forfeit() {
    let dex = load_dex();
    let mut forfeits = 0;
    let mut choices = 0;
    for corpus in ["puredata", "full", "directed", "directed-sleep"] {
        for path in corpus_files(&repo_root().join("fixtures/corpus-v1").join(corpus)) {
            let fx = Fixture::load(&path).unwrap();
            let mut reference =
                ps_reference_battle(&dex, &fx.seed, &fx.p1team, &fx.p2team).unwrap();
            let mut operational =
                Battle::from_fixture(&dex, &fx.seed, &fx.p1team, &fx.p2team).unwrap();
            reference.log.clear();
            operational.log.clear();
            for (index, choice) in fx.choices.iter().enumerate() {
                let side = usize::from(choice.side == "p2");
                reference.choose(&dex, side, &choice.choice).unwrap();
                operational.choose(&dex, side, &choice.choice).unwrap();
                choices += 1;
                if operational
                    .log
                    .iter()
                    .any(|l| l.contains("Sleep Clause violated:"))
                {
                    let infliction = operational
                        .log
                        .iter()
                        .rfind(|l| l.starts_with("|-status|") && l.contains("|slp"))
                        .unwrap();
                    let target_side = usize::from(infliction.starts_with("|-status|p2"));
                    let sleeping = operational.sides[target_side]
                        .party
                        .iter()
                        .filter(|&&slot| {
                            let p = &operational.sides[target_side].roster[slot as usize];
                            p.hp > 0 && p.status == Status::Slp
                        })
                        .count();
                    assert!(sleeping >= 2);
                    assert_eq!(
                        operational.outcome(),
                        Some(if target_side == 0 {
                            Outcome::P1Win
                        } else {
                            Outcome::P2Win
                        })
                    );
                    forfeits += 1;
                    break;
                }
                let mut actual = operational.essence(&dex);
                let rules = actual["field"]["pseudoWeather"].as_object_mut().unwrap();
                let mut sleep = rules.remove("stadiumsleepclause").unwrap();
                sleep["id"] = "sleepclausemod".into();
                rules.insert("sleepclausemod".into(), sleep);
                assert_eq!(
                    actual,
                    reference.essence(&dex),
                    "{} choice {index}",
                    path.display()
                );
                assert_eq!(
                    operational.prng.seed_str(),
                    reference.prng.seed_str(),
                    "{} choice {index}",
                    path.display()
                );
                assert_eq!(
                    operational.log,
                    reference.log,
                    "{} choice {index}",
                    path.display()
                );
                reference.log.clear();
                operational.log.clear();
            }
        }
    }
    assert!(forfeits > 0);
    assert!(choices > 2000);
    eprintln!("{choices} choices compared, {forfeits} operational sleep forfeits");
}
