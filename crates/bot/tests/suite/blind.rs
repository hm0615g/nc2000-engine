//! M10b Gate B — no psychic tells at parity budget — plus blind-agent
//! lifecycle smoke.
//!
//! # Gate B (`no_psychic_tells_on_the_collision_pair`)
//!
//! Behavioral invariance: run the blind agent (fixed agent seed, fixed
//! budget) as P1 against BOTH collision-pair teams as the hidden truth,
//! with the opponent driven by a shared script (choices legal under both
//! truths) so the two games' *public* trajectories agree as long as
//! possible. While they agree, the blind agent's decisions must be
//! IDENTICAL — any divergence is a leak of set-level hidden information.
//!
//! "Public trajectories agree" is checked by the strongest available
//! projection: determinize both true battles with the SAME candidate and
//! the SAME rng — the determinizer overwrites exactly the hidden fields, so
//! Debug-equality of the outputs means everything the search can see is
//! bit-identical. This is conservative: it also pins the declared non-goal
//! fields the determinizer keeps (hidden status counters, quick_claw_roll),
//! which can legitimately drift once the two truths' PRNG streams diverge —
//! the loop then just stops, it does not fail. The gate requires at least
//! the preview decision plus one in-battle decision proven identical, with
//! the belief still holding both candidates (the ambiguity actually
//! exercised).
//!
//! The two truth battles align the collision teams' display order (bench
//! order is the builder's free choice, not team identity — the belief's
//! preview filter is order-insensitive for the same reason), so the
//! species-by-display-position public view matches.

use std::sync::Arc;

use super::common;
use nc2000_bot::preview::MetaPool;
use nc2000_bot::{Agent, Belief, BlindAgent, BlindSearch, Observer, RmConfig, SplitMix64};
use nc2000_engine::battle::{PokemonSet, SearchChoice};
use nc2000_engine::dex::Dex;
use nc2000_engine::state::Battle;

fn pool() -> Arc<MetaPool> {
    common::meta_pool_v0_arc().clone()
}

/// The pool's known species+level collision (README: exactly one).
fn collision_pair(pool: &MetaPool) -> (usize, usize) {
    let sig = |sets: &[PokemonSet]| {
        let mut v: Vec<(String, u8)> =
            sets.iter().map(|s| (s.species.clone(), s.level)).collect();
        v.sort();
        v
    };
    for i in 0..pool.teams.len() {
        for j in i + 1..pool.teams.len() {
            if sig(&pool.teams[i].sets) == sig(&pool.teams[j].sets) {
                return (i, j);
            }
        }
    }
    panic!("no species+level collision in the pool (README says there is one)");
}

/// Permute `other` to `reference`'s (species, level) display order.
fn align_order(reference: &[PokemonSet], other: &[PokemonSet]) -> Vec<PokemonSet> {
    let mut remaining: Vec<PokemonSet> = other.to_vec();
    reference
        .iter()
        .map(|r| {
            let k = remaining
                .iter()
                .position(|t| t.species == r.species && t.level == r.level)
                .expect("collision pair must share the species+level multiset");
            remaining.remove(k)
        })
        .collect()
}

/// Debug projection of everything the blind search can see of `battle`:
/// determinized with a fixed candidate + fixed rng, hidden fields are
/// overwritten identically, so equality across the two truths ⇔ the
/// search-visible state is bit-identical.
fn projection(
    dex: &Dex,
    bel: &Belief,
    battle: &Battle,
    obs: &Observer,
    cand: usize,
) -> String {
    let mut rng = SplitMix64::new(0x9E37);
    format!("{:?}", bel.determinize_with(dex, battle, obs, Some(cand), &mut rng))
}

#[test]
fn no_psychic_tells_on_the_collision_pair() {
    collision_invariance(false);
}

#[test]
fn adaptive_mixture_has_no_psychic_tells_on_the_collision_pair() {
    collision_invariance(true);
}

#[test]
fn active_mixture_is_identical_across_ambiguous_hidden_stats() {
    let dex = common::dex();
    let set = |species: &str, moves: &[&str], atk: i32| -> PokemonSet {
        serde_json::from_value(serde_json::json!({"name":species,"species":species,"ability":"",
            "level":50,"moves":moves,"happiness":255,
            "evs":{"hp":255,"atk":atk,"def":255,"spa":255,"spd":255,"spe":255},
            "ivs":{"hp":30,"atk":30,"def":30,"spa":30,"spd":30,"spe":30}})).unwrap()
    };
    let own = vec![set("Starmie", &["Return", "Surf"], 255)];
    let opponents = [255, 4].map(|atk|vec![set("Wobbuffet", &["Counter", "Mirror Coat", "Safeguard"], atk)]);
    let meta = Arc::new(MetaPool {teams:opponents.iter().enumerate().map(|(i, sets)|nc2000_bot::preview::MetaTeam {
        id:format!("counter-{i}"), sets:sets.clone(), weight:None,
    }).collect()});
    let cfg = RmConfig {iterations:1000, c:0.4, ..Default::default()};
    let mut observations = Vec::new();
    for opponent in &opponents {
        let mut battle = Battle::from_fixture(&dex, "1,2,3,4", &own, opponent).unwrap();
        let mut agent = BlindAgent::new(cfg.clone(), meta.clone(), None, 7).with_adaptive_mixture(400);
        let legal = battle.legal_choices(&dex, 0);
        let pick = agent.choose(&battle, &dex, 0, &legal);
        let other = battle.legal_choices(&dex, 1)[0];
        battle.apply_choices(&dex, [Some(pick), Some(other)]).unwrap();
        for (side, hp) in [(0, 68), (1, 120)] {
            let id = battle.active_id(side).unwrap();
            battle.poke_mut(id).hp = hp;
            let subject = format!("p{}a: {}", side + 1, battle.poke(id).name.as_str());
            let (secret, shared) = battle.get_health(id);
            battle.add_split(
                if side == 0 { "p1" } else { "p2" },
                &["-damage", &subject, &secret],
                &["-damage", &subject, &shared],
            );
        }
        let before_rng = agent.search_rng_state();
        let legal = battle.legal_choices(&dex, 0);
        let choice = agent.choose(&battle, &dex, 0, &legal);
        let belief = agent.belief().unwrap(); let observer = agent.observer().unwrap();
        assert_eq!(belief.candidate_count(), 2);
        let mut reference = BlindSearch::new(&battle, &dex, cfg.clone(), 0, before_rng);
        reference.finish_with_typed_mixed_probes(&dex, belief, observer, 1000, 400);
        assert!(reference.mixed_policy().is_some());
        let probes = reference.joint_probe_visits().iter().sum::<u32>();
        reference.step_typed_joint_probes(&dex, belief, observer, 240);
        assert_eq!(reference.joint_probe_visits().iter().sum::<u32>(), probes + 240);
        let mut cutoff = BlindSearch::new(&battle, &dex, cfg.clone(), 0, before_rng);
        let mut sampled_children = Vec::new();
        let reply = nc2000_engine::battle::SearchChoice::Move(dex.moves.id("mirrorcoat").unwrap());
        for i in 0..120 {
            cutoff.step_forced_joint_with_root_child_value(&dex, belief, observer, i % 2, reply,
                &mut |child| {
                    let key = child.state_key128();
                    sampled_children.push(key);
                    (key % 2 == 0).then_some(0.375)
                });
        }
        assert!(!sampled_children.is_empty());
        cutoff.step(&dex, belief, observer, 120);
        observations.push((choice, agent.search_rng_state(), reference.root_matrix(),
            projection(&dex, belief, &battle, observer, 0), sampled_children,
            cutoff.root_matrix(), cutoff.visits().to_vec()));
    }
    assert_eq!(observations[0], observations[1]);
}

fn collision_invariance(mixed: bool) {
    let dex = common::dex();
    let meta = pool();
    let (ci, cj) = collision_pair(&meta);
    let our = meta.teams[0].sets.clone();
    let truth_a = meta.teams[ci].sets.clone();
    let truth_b = align_order(&truth_a, &meta.teams[cj].sets);

    let mut total_checkpoints = 0usize;
    let mut ambiguous_battle_checkpoints = 0usize;

    for (run, battle_seed) in ["11,22,33,44", "1,3,5,7"].iter().enumerate() {
        let mut battle_a = Battle::from_fixture(&dex, battle_seed, &our, &truth_a).unwrap();
        let mut battle_b = Battle::from_fixture(&dex, battle_seed, &our, &truth_b).unwrap();
        // outer battles log-ON: the observer's log channel is part of the
        // surface under test

        let cfg = RmConfig { iterations: 96, ..Default::default() };
        let agent_seed = 0xB11D + run as u64;
        let mut agent_a = BlindAgent::new(cfg.clone(), meta.clone(), None, agent_seed);
        let mut agent_b = BlindAgent::new(cfg.clone(), meta.clone(), None, agent_seed);
        if mixed {
            agent_a = agent_a.with_adaptive_mixture(38);
            agent_b = agent_b.with_adaptive_mixture(38);
        }

        // test-owned observers/beliefs (independent of the agents') define
        // the public-agreement region
        let mut obs = [Observer::new(&battle_a, 0), Observer::new(&battle_b, 0)];
        let mut bel =
            [Belief::new(&dex, &meta, &obs[0]), Belief::new(&dex, &meta, &obs[1])];

        let mut points = 0usize;
        loop {
            if battle_a.outcome().is_some()
                || battle_b.outcome().is_some()
                || battle_a.turn > 8
            {
                break;
            }
            obs[0].observe(&battle_a, &dex);
            bel[0].sync(&dex, &obs[0]);
            obs[1].observe(&battle_b, &dex);
            bel[1].sync(&dex, &obs[1]);

            if points == 0 {
                // premise: the collision keeps both candidates alive at preview
                for b in &bel {
                    assert!(
                        b.alive().contains(&ci) && b.alive().contains(&cj),
                        "premise broken: collision candidates not both alive at preview \
                         (alive {:?})",
                        b.alive()
                    );
                }
            }

            // ---- public-agreement region check (see module doc)
            if bel[0].alive() != bel[1].alive() {
                break;
            }
            let agree = bel[0].alive().iter().all(|&cand| {
                projection(&dex, &bel[0], &battle_a, &obs[0], cand)
                    == projection(&dex, &bel[1], &battle_b, &obs[1], cand)
            });
            if !agree {
                break;
            }

            // ---- the gate: identical decisions from identical public views
            let cs_a = battle_a.legal_choices(&dex, 0);
            let cs_b = battle_b.legal_choices(&dex, 0);
            assert_eq!(cs_a, cs_b, "own legal choices diverged inside the agreement region");
            let mut picks_a = [None, None];
            let mut picks_b = [None, None];
            if !cs_a.is_empty() {
                let pa = agent_a.choose(&battle_a, &dex, 0, &cs_a);
                let pb = agent_b.choose(&battle_b, &dex, 0, &cs_b);
                if mixed && points == 0 {
                    let mut baseline = BlindAgent::new(cfg.clone(), meta.clone(), None, agent_seed);
                    assert_eq!(pa, baseline.choose(&battle_a, &dex, 0, &cs_a));
                    assert_eq!(agent_a.search_rng_state(), baseline.search_rng_state());
                }
                assert_eq!(
                    pa, pb,
                    "GATE B LEAK: blind decision diverged between hidden truths \
                     (seed {battle_seed}, turn {}, checkpoint {points})",
                    battle_a.turn
                );
                total_checkpoints += 1;
                if bel[0].candidate_count() >= 2
                    && !matches!(cs_a[0], SearchChoice::Team(_))
                {
                    ambiguous_battle_checkpoints += 1;
                }
                picks_a[0] = Some(pa);
                picks_b[0] = Some(pb);
            }

            // ---- scripted opponent: first choice legal under BOTH truths
            let os_a = battle_a.legal_choices(&dex, 1);
            let os_b = battle_b.legal_choices(&dex, 1);
            assert_eq!(
                os_a.is_empty(),
                os_b.is_empty(),
                "request kinds diverged inside the agreement region"
            );
            if !os_a.is_empty() {
                let Some(c) = os_a.iter().copied().find(|c| os_b.contains(c)) else {
                    break; // no shared legal action left: region ends here
                };
                picks_a[1] = Some(c);
                picks_b[1] = Some(c);
            }
            battle_a.apply_choices(&dex, picks_a).unwrap();
            battle_b.apply_choices(&dex, picks_b).unwrap();
            points += 1;
        }
    }

    assert!(
        total_checkpoints >= 3,
        "only {total_checkpoints} identical-decision checkpoints proven (need preview + \
         in-battle coverage)"
    );
    assert!(
        ambiguous_battle_checkpoints >= 1,
        "no in-battle checkpoint had both collision candidates alive — the ambiguity was \
         never exercised"
    );
}

// -------------------------------------------------------- lifecycle smoke

/// One full pool game (blind P1 vs a scripted greedy P2, log ON): the
/// belief never goes fallback and always keeps the true team; then the SAME
/// agent instance starts a new game against a different team — the preview
/// reset path must rebuild the observer/belief (arena constructs agents
/// fresh per game; this is the safety net for other harnesses).
#[test]
fn blind_lifecycle_and_reset() {
    let dex = common::dex();
    let meta = pool();
    let cfg = RmConfig { iterations: 32, ..Default::default() };
    let mut agent = BlindAgent::new(cfg, meta.clone(), None, 77);

    for (game, truth_idx) in [(0usize, 1usize), (1, 5)] {
        let mut battle = Battle::from_fixture(
            &dex,
            "2,4,6,8",
            &meta.teams[0].sets,
            &meta.teams[truth_idx].sets,
        )
        .unwrap();
        let mut opp = nc2000_bot::MaxDamageAgent::new();
        let mut decisions = 0usize;
        while battle.outcome().is_none() && battle.turn <= 100 {
            let mut picks = [None, None];
            let cs = battle.legal_choices(&dex, 0);
            if !cs.is_empty() {
                picks[0] = Some(agent.choose(&battle, &dex, 0, &cs));
                decisions += 1;
                let bel = agent.belief().expect("belief exists after a decision");
                assert!(!bel.is_fallback(), "pool opponent must never go fallback");
                assert!(
                    bel.alive().contains(&truth_idx),
                    "game {game}: true team {truth_idx} filtered (alive {:?})",
                    bel.alive()
                );
            }
            let os = battle.legal_choices(&dex, 1);
            if !os.is_empty() {
                picks[1] = Some(opp.choose(&battle, &dex, 1, &os));
            }
            battle.apply_choices(&dex, picks).unwrap();
        }
        assert!(decisions > 5, "game {game}: too few decisions ({decisions})");
    }
}

#[test]
fn blind_agent_routes_lead_prior_to_preview_search_only_on_offpool_nonuniform_prior() {
    use nc2000_bot::BeliefPrior;
    use nc2000_engine::dex::toid;

    let dex = common::dex();
    let meta = pool();
    let (_, fx) = &common::corpus_json_values()[0];
    let opp_sets: Vec<PokemonSet> = serde_json::from_value(fx["p1team"].clone()).unwrap();
    let own_sets = meta.teams[1].sets.clone();

    let sp0 = toid(&opp_sets[0].species);
    let sp1 = toid(&opp_sets[1].species);
    assert_ne!(sp0, sp1);

    let cfg = RmConfig {
        iterations: 120,
        ..Default::default()
    };

    let run_preview = |opp: &[PokemonSet], prior_json: Option<&str>| -> SearchChoice {
        let mut battle = Battle::from_fixture(&dex, "1,2,3,4", opp, &own_sets).unwrap();
        let mut agent = BlindAgent::new(cfg.clone(), meta.clone(), None, 7);
        if let Some(text) = prior_json {
            agent.set_belief_prior(Arc::new(BeliefPrior::from_json(text)));
        }
        let choices = battle.legal_choices(&dex, 1);
        agent.choose(&battle, &dex, 1, &choices)
    };

    let base = run_preview(&opp_sets, None);
    let unrelated = run_preview(
        &opp_sets,
        Some(r#"{"species":{"shuckle":{"lead":0.99}}}"#),
    );
    let all_equal_species = opp_sets
        .iter()
        .map(|s| format!(r#""{}":{{"lead":0.25}}"#, toid(&s.species)))
        .collect::<Vec<_>>()
        .join(",");
    let uniform = run_preview(
        &opp_sets,
        Some(&format!(r#"{{"species":{{{all_equal_species}}}}}"#)),
    );
    assert_eq!(unrelated, base, "non-matching lead prior must stay identical");
    assert_eq!(uniform, base, "uniform lead weights across roster must stay identical");

    // In-pool opponent must remain bit-identical even with skewed lead weights.
    let pool_opp = meta.teams[0].sets.clone();
    let psp0 = toid(&pool_opp[0].species);
    let psp1 = toid(&pool_opp[1].species);
    let pool_base = run_preview(&pool_opp, None);
    let pool_with_prior = run_preview(
        &pool_opp,
        Some(&format!(r#"{{"species":{{"{psp0}":{{"lead":0.99}},"{psp1}":{{"lead":0.01}}}}}}"#)),
    );
    assert_eq!(pool_with_prior, pool_base, "in-pool opponent must ignore BeliefPrior lead");

    // Skewed lead-only prior on an off-pool opponent sets fallback_lead_weights and
    // must shift Team Preview choice across opposite lead priors.
    let prior_a = format!(
        r#"{{"species":{{"{sp0}":{{"lead":0.99}},"{sp1}":{{"lead":0.01}}}}}}"#
    );
    let prior_b = format!(
        r#"{{"species":{{"{sp0}":{{"lead":0.01}},"{sp1}":{{"lead":0.99}}}}}}"#
    );
    let mut diffs = 0usize;
    for seed in 1u64..=12 {
        let mut run_seeded = |prior_json: &str| {
            let mut battle = Battle::from_fixture(&dex, "1,2,3,4", &opp_sets, &own_sets).unwrap();
            let mut agent = BlindAgent::new(cfg.clone(), meta.clone(), None, seed);
            agent.set_belief_prior(Arc::new(BeliefPrior::from_json(prior_json)));
            let choices = battle.legal_choices(&dex, 1);
            let pick = agent.choose(&battle, &dex, 1, &choices);
            let g_obs = agent.observer().unwrap();
            assert!(
                agent.belief().unwrap().has_preview_prior(g_obs),
                "skewed lead prior on off-pool team must activate has_preview_prior"
            );
            pick
        };
        if run_seeded(&prior_a) != run_seeded(&prior_b) {
            diffs += 1;
        }
    }
    assert!(
        diffs > 0,
        "BlindAgent Team Preview never responded to opposite lead-only BeliefPriors"
    );
}

#[test]
fn blind_agent_wires_pick_prior_for_preview_and_battle() {
    use nc2000_bot::PickPrior;
    use nc2000_engine::dex::toid;

    let dex = common::dex();
    let meta = pool();
    let opp_team = &meta.teams[0];
    let opp_sets = opp_team.sets.clone();
    let own_sets = meta.teams[1].sets.clone();

    let level_sum = |t: [usize; 3]| -> i32 {
        t.iter().map(|&i| opp_sets[i].level as i32).sum()
    };
    let legal_for_lead = |lead: usize, avoid: usize| -> [usize; 3] {
        let mut rest: Vec<usize> = (0..opp_sets.len()).filter(|&i| i != lead && i != avoid).collect();
        rest.sort_by_key(|&i| opp_sets[i].level);
        [lead, rest[0], rest[1]]
    };
    let triple_a = legal_for_lead(0, 1);
    let triple_b = legal_for_lead(1, 0);
    assert!(level_sum(triple_a) <= 155 && level_sum(triple_b) <= 155);

    let sp = |i: usize| dex.species.key(dex.species.id(&toid(&opp_sets[i].species)).unwrap()).to_string();
    let preview_json: Vec<serde_json::Value> = own_sets
        .iter()
        .map(|s| {
            serde_json::json!({
                "species": dex.species.key(dex.species.id(&toid(&s.species)).unwrap()),
                "level": s.level,
                "gender": match s.gender.as_deref() { Some("M") => "M", Some("F") => "F", _ => "" },
                "item": !s.item.is_empty(),
            })
        })
        .collect();
    let make_prior = |fav: [usize; 3]| -> Arc<PickPrior> {
        let text = serde_json::json!({
            "schema": "nc2000-pick-prior-v1",
            "smoothing": 0.001,
            "backoff_strength": 1.0,
            "source": {},
            "rows": [{
                "team": opp_team.id,
                "enemy_preview": preview_json,
                "side": 0,
                "choices": [{ "species": [sp(fav[0]), sp(fav[1]), sp(fav[2])], "count": 500 }]
            }]
        })
        .to_string();
        Arc::new(PickPrior::from_json(&text).unwrap())
    };
    let prior_a = make_prior(triple_a);
    let prior_b = make_prior(triple_b);

    let cfg = RmConfig {
        iterations: 160,
        ..Default::default()
    };

    let mut preview_diffs = 0usize;
    for seed in 1u64..=12 {
        let mut run_preview = |prior: Arc<PickPrior>| -> SearchChoice {
            let mut battle = Battle::from_fixture(&dex, "1,2,3,4", &opp_sets, &own_sets).unwrap();
            let mut agent = BlindAgent::new(cfg.clone(), meta.clone(), None, seed).with_pick_prior(prior);
            let choices = battle.legal_choices(&dex, 1);
            let pick = agent.choose(&battle, &dex, 1, &choices);
            let obs = agent.observer().unwrap();
            let belief = agent.belief().unwrap();
            assert!(belief.has_active_pick_draws(obs));
            assert!(belief.has_preview_prior(obs));
            pick
        };
        if run_preview(prior_a.clone()) != run_preview(prior_b.clone()) {
            preview_diffs += 1;
        }
    }
    assert!(
        preview_diffs > 0,
        "BlindAgent Team Preview must respond to opposite PickPriors"
    );

    // Also verify Turn 1+ battle re-conditioning and determinized hidden bench preference.
    let (bench_a, bench_b) = (0..6)
        .find_map(|lead| {
            let mut triples = Vec::new();
            let others: Vec<usize> = (0..6).filter(|&i| i != lead).collect();
            for a in 0..others.len() {
                for b in (a + 1)..others.len() {
                    let t = [lead, others[a], others[b]];
                    if level_sum(t) <= 155 {
                        triples.push(t);
                    }
                }
            }
            (triples.len() >= 2).then(|| (triples[0], triples[triples.len() - 1]))
        })
        .expect("need two legal triples with a shared lead");
    assert!(level_sum(bench_a) <= 155 && level_sum(bench_b) <= 155);
    let prior_bench_a = make_prior(bench_a);
    let prior_bench_b = make_prior(bench_b);

    let run_turn1 = |prior: Arc<PickPrior>| -> usize {
        let mut battle = Battle::from_fixture(&dex, "1,2,3,4", &opp_sets, &own_sets).unwrap();
        let mut agent = BlindAgent::new(cfg.clone(), meta.clone(), None, 7).with_pick_prior(prior);
        let prev_choices = battle.legal_choices(&dex, 1);
        let p2_prev = agent.choose(&battle, &dex, 1, &prev_choices);
        battle.apply_choice(&dex, 0, SearchChoice::Team(bench_a.map(|i| (i + 1) as u8))).unwrap();
        battle.apply_choice(&dex, 1, p2_prev).unwrap();
        let choices = battle.legal_choices(&dex, 1);
        let _ = agent.choose(&battle, &dex, 1, &choices);
        let obs = agent.observer().unwrap();
        let belief = agent.belief().unwrap();
        assert!(belief.has_active_pick_draws(obs), "Turn 1 must refresh active pick_draws");
        let mut rng = SplitMix64::new(99);
        let mut hits_a = 0usize;
        let sp_set_a = [
            dex.species.id(&toid(&opp_sets[bench_a[1]].species)).unwrap(),
            dex.species.id(&toid(&opp_sets[bench_a[2]].species)).unwrap(),
        ];
        for _ in 0..40 {
            let det = belief.determinize(&dex, &battle, obs, &mut rng);
            let mut species: Vec<_> = det.sides[0]
                .party
                .iter()
                .map(|&s| det.sides[0].roster[s as usize].base_species)
                .collect();
            species.sort();
            if sp_set_a.iter().all(|s| species.contains(s)) {
                hits_a += 1;
            }
        }
        hits_a
    };
    assert!(
        run_turn1(prior_bench_a) > run_turn1(prior_bench_b) + 15,
        "Turn 1 hidden-bench sampling did not follow PickPrior"
    );
}
