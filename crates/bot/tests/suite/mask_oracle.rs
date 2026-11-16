//! Engine-oracle soundness & decoupling contract suite for `nc2000_bot::mask`
//! and `nc2000_bot::endgame_root`.
//!
//! Guards against dual-engine drift by testing `public_dominated_actions` and
//! `dominated_actions` directly against `crates/engine` (`Battle::apply_choices`)
//! across determinized worlds, opponent replies, and PRNG seeds, instead of
//! relying solely on point-assertion unit tests.

use nc2000_bot::{
    mask::{dominated_actions, public_dominated_actions},
    player::PlayerChannel,
    player_state::LocalPlayer,
    preview::{MetaPool, MetaTeam},
    Agent, Belief, BlindAgent, BlindSearch, MaxDamageAgent, OpenAgent, ProtocolAgent, RmAgent,
    RmConfig, SkuctSearch, SplitMix64,
};
use nc2000_engine::{
    battle::{Outcome, PokemonSet, SearchChoice},
    dex::Dex,
    state::Battle,
};

fn mk_set(species: &str, item: &str, moves: &[&str], level: u8) -> PokemonSet {
    serde_json::from_value(serde_json::json!({
        "name": species,
        "species": species,
        "item": item,
        "ability": "No Ability",
        "moves": moves,
        "level": level,
        "evs": {"hp": 255, "atk": 255, "def": 255, "spa": 255, "spd": 255, "spe": 255},
        "ivs": {"hp": 30, "atk": 30, "def": 30, "spa": 30, "spd": 30, "spe": 30},
        "happiness": 255
    }))
    .unwrap()
}

fn load_belief_pool() -> MetaPool {
    let path = format!("{}/../../data/belief-pool-v3/belief-pool.json", env!("CARGO_MANIFEST_DIR"));
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

/// Extract the protocol lines belonging strictly to `side`'s own move execution
/// during a single `apply_choices` turn, stopping before the other side's action
/// or end-of-turn `|upkeep|`.
fn own_move_slice(log: &[String], side: usize) -> &[String] {
    let own_prefix = format!("|move|p{}a:", side + 1);
    let opp_move = format!("|move|p{}a:", 2 - side);
    let opp_cant = format!("|cant|p{}a:", 2 - side);
    let opp_switch = format!("|switch|p{}a:", 2 - side);
    let Some(start) = log.iter().position(|l| l.starts_with(&own_prefix)) else {
        return &[];
    };
    let mut end = log.len();
    for (offset, line) in log[start + 1..].iter().enumerate() {
        if line == "|"
            || line.starts_with(&opp_move)
            || line.starts_with(&opp_cant)
            || line.starts_with(&opp_switch)
            || line == "|upkeep"
            || line.starts_with("|turn|")
        {
            end = start + 1 + offset;
            break;
        }
    }
    &log[start..end]
}

fn is_after_move_residual(line: &str) -> bool {
    [
        "[from] psn",
        "[from] brn",
        "[from] Leech Seed",
        "[from] Nightmare",
        "[from] Curse",
        "[from] item:",
    ]
    .iter()
    .any(|tag| line.contains(tag))
}

/// Returns `true` if `side`'s own move execution block produced any positive or
/// board-altering effect in the real Gen 2 engine (damage, Sub break, heal,
/// status/cure, boost/unboost, side condition, weather, volatile start, phaze,
/// or Baton Pass self-switch), ignoring Gen 2 `afterMoveSelf` status/item ticks.
fn move_slice_had_engine_effect(slice: &[String], side: usize) -> bool {
    if slice.is_empty() {
        return false;
    }
    let own_switch = format!("|switch|p{}a:", side + 1);
    for line in &slice[1..] {
        if (line.starts_with("|-damage|") || line.starts_with("|-heal|"))
            && is_after_move_residual(line)
        {
            continue;
        }
        if line.starts_with("|-damage|")
            || line.starts_with("|-heal|")
            || line.starts_with("|-status|")
            || line.starts_with("|-curestatus|")
            || line.starts_with("|-boost|")
            || line.starts_with("|-unboost|")
            || line.starts_with("|-clearallboost")
            || line.starts_with("|-sidestart|")
            || line.starts_with("|-sideend|")
            || line.starts_with("|-weather|")
            || line.starts_with("|-start|")
            || line.starts_with("|drag|")
            || line.starts_with(&own_switch)
            || (line.starts_with("|-end|") && line.contains("|Substitute"))
        {
            return true;
        }
    }
    false
}

/// Verify that every action masked by `public_dominated_actions` on `ObservedPosition`
/// satisfies:
/// 1. Information-set invariance across hidden opponent determinizations.
/// 2. Exact agreement with `BlindSearch::from_observation(...).dominated()`.
/// 3. Universal engine soundness across determinizations × all legal opponent replies × seeds.
/// 4. Survival non-starvation: masking never eliminates all non-losing waiting options when
///    unmasked actions lose immediately (e.g. Safeguard + Destiny Bond + sleeping bench).
fn verify_public_mask_against_engine_oracle(
    dex: &Dex,
    pool: &MetaPool,
    battle: &Battle,
    rng_seed: u64,
) {
    if battle.ended || battle.request_state != nc2000_engine::state::RequestState::Move {
        return;
    }
    for side in [0usize, 1] {
        let Ok((local, observer)) = LocalPlayer::new(battle, dex, side) else {
            continue;
        };
        let mut belief = Belief::new(dex, pool, &observer);
        belief.sync(dex, &observer);
        let mut rng = SplitMix64::new(rng_seed ^ (side as u64));
        let Ok(pos) = local.position(dex, &mut belief, &observer, &mut rng) else {
            continue;
        };

        let expected = public_dominated_actions(pos.model(), dex, side);
        let search = BlindSearch::from_observation(&pos, dex, RmConfig::default(), side, rng_seed);
        let search_dom: Vec<(SearchChoice, bool)> = search
            .actions()
            .iter()
            .copied()
            .zip(search.dominated().iter().copied())
            .collect();
        for (act, is_dom) in &search_dom {
            assert_eq!(
                *is_dom,
                expected.iter().any(|(c, _)| c == act),
                "BlindSearch::dominated() drifted from public_dominated_actions for side {side}"
            );
        }

        // 1. Information-set invariance across 8 sampled hidden worlds.
        let mut worlds = Vec::with_capacity(8);
        for _ in 0..8 {
            let det = pos.sample(dex, &belief, &observer, &mut rng);
            let det_mask = public_dominated_actions(det.model(), dex, side);
            assert_eq!(
                det_mask, expected,
                "public_dominated_actions leaked hidden determinization state on side {side}"
            );
            worlds.push(det.model().clone());
        }
        // Also include the true ground-truth battle as a mandatory world!
        worlds.push(battle.clone());

        if expected.is_empty() {
            continue;
        }

        let my_loss = if side == 0 {
            Outcome::P2Win
        } else {
            Outcome::P1Win
        };

        // 2. Universal engine oracle check across all worlds × legal opponent replies × PRNG seeds.
        for &(choice, reason) in &expected {
            let is_forfeit_or_self_ko = reason.contains("Sleep Clause") || reason.contains("self-KO");
            for (w_idx, w_base) in worlds.iter().enumerate() {
                let mut w_enum = w_base.clone();
                let my_legal = w_enum.legal_choices(dex, side);
                if !my_legal.contains(&choice) {
                    continue;
                }
                let opp_legal = w_enum.legal_choices(dex, 1 - side);
                for &opp_choice in &opp_legal {
                    for seed in [1u64, 17, 53, 129] {
                        let mut trial = w_base.clone();
                        trial.set_log_enabled(true);
                        trial.log.clear();
                        trial.reseed(seed);
                        let mut joint = [None, None];
                        joint[side] = Some(choice);
                        joint[1 - side] = Some(opp_choice);
                        trial.apply_choices(dex, joint).unwrap();
                        if is_forfeit_or_self_ko {
                            let slice = own_move_slice(&trial.log, side);
                            let executed_cleanly = !slice.is_empty()
                                && !slice.iter().any(|l| {
                                    l.starts_with("|-miss|")
                                        || l.starts_with("|-fail|")
                                        || l.starts_with("|-immune|")
                                        || l.contains("|Safeguard")
                                });
                            if executed_cleanly {
                                assert_eq!(
                                    trial.outcome(),
                                    Some(my_loss),
                                    "forfeit/self-KO mask `{reason}` on {:?} executed without losing in world {w_idx}: {:?}",
                                    choice,
                                    trial.log
                                );
                            }
                        } else {
                            let slice = own_move_slice(&trial.log, side);
                            assert!(
                                !move_slice_had_engine_effect(slice, side),
                                "public no-op mask `{reason}` falsely excluded live move {:?} vs {:?} (world {w_idx}, seed {seed}): {:?}",
                                choice,
                                opp_choice,
                                slice
                            );
                        }
                    }
                }
            }
        }

        // 3. Non-starvation under survival traps (e.g. Destiny Bond / Perish 1):
        // if an unmasked move exists, check on the true battle whether EVERY unmasked
        // action loses immediately against some opponent reply while a masked action survives!
        let mut b_legal = battle.clone();
        let all_acts = b_legal.legal_choices(dex, side);
        let unmasked: Vec<SearchChoice> = all_acts
            .iter()
            .copied()
            .filter(|c| !expected.iter().any(|(mc, _)| mc == c))
            .collect();
        if !unmasked.is_empty() {
            let opp_acts = b_legal.legal_choices(dex, 1 - side);
            for &opp_c in &opp_acts {
                let all_unmasked_lose = unmasked.iter().all(|&my_c| {
                    let mut t = battle.clone();
                    t.reseed(7);
                    let mut j = [None, None];
                    j[side] = Some(my_c);
                    j[1 - side] = Some(opp_c);
                    t.apply_choices(dex, j).unwrap();
                    t.outcome() == Some(my_loss)
                });
                if all_unmasked_lose {
                    for &(masked_c, reason) in &expected {
                        let mut t = battle.clone();
                        t.reseed(7);
                        let mut j = [None, None];
                        j[side] = Some(masked_c);
                        j[1 - side] = Some(opp_c);
                        t.apply_choices(dex, j).unwrap();
                        assert_eq!(
                            t.outcome(),
                            Some(my_loss),
                            "mask `{reason}` starved sole surviving wait action {:?} vs {:?}",
                            masked_c,
                            opp_c
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn engine_oracle_verifies_public_mask_across_battles_and_adversarial_traps() {
    let dex = conformance::load_dex();
    let pool = load_belief_pool();

    // Part A: Adversarial edge traps that previously caused dual-engine drift.
    // Trap 1: Safeguard + Destiny Bond Wobbuffet with sleeping Snorlax on bench.
    {
        let own = vec![mk_set("Jynx", "Leftovers", &["Lovely Kiss", "Ice Beam"], 55)];
        let foe = vec![
            mk_set("Wobbuffet", "Leftovers", &["Destiny Bond", "Safeguard", "Splash"], 50),
            mk_set("Snorlax", "Leftovers", &["Rest", "Curse"], 50),
        ];
        let mut b = Battle::from_fixture(&dex, "1,2,3,4", &own, &foe).unwrap();
        b.set_log_enabled(true);
        b.choose(&dex, 0, "team 1").unwrap();
        b.choose(&dex, 1, "team 2,1").unwrap();
        b.choose(&dex, 0, "move lovelykiss").unwrap();
        b.choose(&dex, 1, "move curse").unwrap();
        b.choose(&dex, 0, "move icebeam").unwrap();
        b.choose(&dex, 1, "switch 2").unwrap();
        b.choose(&dex, 0, "move icebeam").unwrap();
        b.choose(&dex, 1, "move safeguard").unwrap();
        b.choose(&dex, 0, "move icebeam").unwrap();
        b.choose(&dex, 1, "move splash").unwrap();
        b.choose(&dex, 0, "move lovelykiss").unwrap();
        b.choose(&dex, 1, "move destinybond").unwrap();
        verify_public_mask_against_engine_oracle(&dex, &pool, &b, 1001);
        assert!(
            public_dominated_actions(&b, &dex, 0).is_empty(),
            "Destiny Bond wait_exempt must keep harmless Lovely Kiss into Safeguard unmasked"
        );
    }

    // Trap 2: Mint Berry Snorlax at full HP facing faster Thunderbolt Zapdos.
    {
        let own = vec![mk_set("Snorlax", "Mint Berry", &["Rest", "Splash"], 50)];
        let foe = vec![mk_set("Zapdos", "Magnet", &["Thunderbolt"], 55)];
        let mut b = Battle::from_fixture(&dex, "1,2,3,4", &own, &foe).unwrap();
        b.set_log_enabled(true);
        b.choose(&dex, 0, "team 1").unwrap();
        b.choose(&dex, 1, "team 1").unwrap();
        verify_public_mask_against_engine_oracle(&dex, &pool, &b, 1002);
        assert!(
            dominated_actions(&b, &dex, 0).is_empty(),
            "slower full-HP Rest against faster attacker is live and must never be masked"
        );
    }

    // Trap 3: Screens already up, all-Ghost switch-in immunity, Safeguard status block,
    // and last-mon Perish Song/Destiny Bond (both with and without residual poison).
    {
        let own = vec![
            mk_set("Gengar", "Leftovers", &["Confuse Ray", "Hypnosis", "Perish Song", "Destiny Bond"], 50),
            mk_set("Snorlax", "Leftovers", &["Body Slam", "Reflect", "Baton Pass", "Roar"], 50),
        ];
        let foe = vec![
            mk_set("Misdreavus", "Leftovers", &["Shadow Ball", "Safeguard", "Mean Look", "Thunder"], 50),
            mk_set("Gengar", "Miracle Berry", &["Thunderbolt", "Ice Punch", "Explosion", "Haze"], 50),
        ];
        let mut b = Battle::from_fixture(&dex, "5,6,7,8", &own, &foe).unwrap();
        b.set_log_enabled(true);
        b.choose(&dex, 0, "team 2").unwrap();
        b.choose(&dex, 1, "team 1,2").unwrap();
        verify_public_mask_against_engine_oracle(&dex, &pool, &b, 1003);
        // Put Reflect and Safeguard up, then re-verify against oracle.
        b.choose(&dex, 0, "move reflect").unwrap();
        b.choose(&dex, 1, "move safeguard").unwrap();
        verify_public_mask_against_engine_oracle(&dex, &pool, &b, 1004);
    }

    // Part B: Multi-turn trajectory sweep across diverse belief-pool teams (both in-pool & off-pool).
    let mut a0 = MaxDamageAgent::conformant();
    let mut a1 = MaxDamageAgent::conformant();
    for game_idx in 0..8usize {
        let t0 = &pool.teams[(game_idx * 7) % pool.teams.len()].sets;
        let t1 = &pool.teams[(game_idx * 13 + 3) % pool.teams.len()].sets;
        let seed = format!("{},{},{},{}", game_idx + 1, game_idx + 11, game_idx + 21, game_idx + 31);
        let mut b = Battle::from_fixture(&dex, &seed, t0, t1).unwrap();
        b.set_log_enabled(true);
        for step in 0..24u64 {
            if b.ended {
                break;
            }
            verify_public_mask_against_engine_oracle(&dex, &pool, &b, 5000 + (game_idx as u64) * 100 + step);
            let need = b.needs_choice();
            let mut joint = [None, None];
            if need[0] {
                let legal = b.legal_choices(&dex, 0);
                joint[0] = Some(a0.choose(&b, &dex, 0, &legal));
            }
            if need[1] {
                let legal = b.legal_choices(&dex, 1);
                joint[1] = Some(a1.choose(&b, &dex, 1, &legal));
            }
            b.apply_choices(&dex, joint).unwrap();
        }
    }
}

#[test]
fn decoupled_modules_preserve_deterministic_search_and_reexports() {
    let dex = conformance::load_dex();
    let pool = load_belief_pool();
    let t0 = &pool.teams[0].sets;
    let t1 = &pool.teams[1].sets;
    let mut b = Battle::from_fixture(&dex, "11,22,33,44", t0, t1).unwrap();
    b.set_log_enabled(true);
    b.choose(&dex, 0, "team 1,2,3").unwrap();
    b.choose(&dex, 1, "team 1,2,3").unwrap();

    // Verify re-export parity between `nc2000_bot::mask` and `nc2000_bot::smmcts`.
    assert_eq!(
        nc2000_bot::mask::dominated_actions(&b, &dex, 0),
        nc2000_bot::smmcts::dominated_actions(&b, &dex, 0)
    );
    assert_eq!(
        nc2000_bot::mask::MaskRules::default(),
        nc2000_bot::smmcts::MaskRules::default()
    );
    assert_eq!(
        nc2000_bot::endgame_root::EndgameRules::default(),
        nc2000_bot::smmcts::EndgameRules::default()
    );

    // Verify deterministic repeatability across BlindSearch, SkuctSearch, RmAgent, and ProtocolAgent.
    let (local, obs) = LocalPlayer::new(&b, &dex, 0).unwrap();
    let mut belief1 = Belief::new(&dex, &pool, &obs);
    belief1.sync(&dex, &obs);
    let mut belief2 = Belief::new(&dex, &pool, &obs);
    belief2.sync(&dex, &obs);
    let pos1 = local.position(&dex, &mut belief1, &obs, &mut SplitMix64::new(99)).unwrap();
    let pos2 = local.position(&dex, &mut belief2, &obs, &mut SplitMix64::new(99)).unwrap();

    let cfg = RmConfig {
        iterations: 256,
        c: 0.4,
        ..Default::default()
    };
    let mut bs1 = BlindSearch::from_observation(&pos1, &dex, cfg.clone(), 0, 777);
    let mut bs2 = BlindSearch::from_observation(&pos2, &dex, cfg.clone(), 0, 777);
    bs1.step(&dex, &belief1, &obs, 256);
    bs2.step(&dex, &belief2, &obs, 256);
    assert_eq!(bs1.visits(), bs2.visits());
    assert_eq!(bs1.means(), bs2.means());
    assert_eq!(bs1.best(), bs2.best());

    let mut sk1 = SkuctSearch::new(&b, &dex, cfg.clone(), 777);
    let mut sk2 = SkuctSearch::new(&b, &dex, cfg.clone(), 777);
    sk1.step(&dex, 256);
    sk2.step(&dex, 256);
    assert_eq!(sk1.visits(0), sk2.visits(0));
    assert_eq!(sk1.best(0), sk2.best(0));

    let legal = b.legal_choices(&dex, 0);
    let mut rm1 = RmAgent::new(cfg.clone(), 777);
    let mut rm2 = RmAgent::new(cfg.clone(), 777);
    assert_eq!(rm1.choose(&b, &dex, 0, &legal), rm2.choose(&b, &dex, 0, &legal));

    let _ = ProtocolAgent::new(&dex, 0, pool, cfg, 777);
}

#[test]
fn analysis_report_dominated_reasons_never_contradict_public_mask_or_leak_imputation() {
    let dex = conformance::load_dex();
    let own = vec![mk_set("Starmie", "Leftovers", &["Recover", "Reflect", "Surf"], 50)];
    let lax_lefties = mk_set("Snorlax", "Leftovers", &["Body Slam", "Splash"], 50);
    let lax_claw = mk_set("Snorlax", "Quick Claw", &["Body Slam", "Splash"], 50);
    let mut b = Battle::from_fixture(&dex, "1,2,3,4", &own, std::slice::from_ref(&lax_claw)).unwrap();
    b.choose(&dex, 0, "team 1").unwrap();
    b.choose(&dex, 1, "team 1").unwrap();
    let mut ch = PlayerChannel::new(0);
    let f_init = ch.frame(&mut b, &dex).unwrap();
    // Put Reflect up while staying at full HP against Splash so `Reflect` becomes
    // publicly dominated (`screen already up`) while `Recover` at full HP remains
    // publicly UNMASKED (opponent item could be Quick Claw, or opponent could attack
    // if faster/Quick Claw triggers) even though full-world `dominated_actions` on a
    // Leftovers imputation would call `Recover` `"recover at full HP before foe"`.
    b.choose(&dex, 0, "move reflect").unwrap();
    b.choose(&dex, 1, "move splash").unwrap();
    let f_turn2 = ch.frame(&mut b, &dex).unwrap();

    let pool = MetaPool {
        teams: vec![
            MetaTeam {
                id: "lax-lefties".into(),
                sets: vec![lax_lefties],
                weight: Some(0.9),
            },
            MetaTeam {
                id: "lax-claw".into(),
                sets: vec![lax_claw],
                weight: Some(0.1),
            },
        ],
    };
    let cfg = RmConfig {
        iterations: 64,
        c: 0.4,
        ..Default::default()
    };
    let mut agent = ProtocolAgent::new(&dex, 0, pool, cfg, 123);
    agent.set_own_team(own);
    for line in f_init.lines.iter().chain(&f_turn2.lines) {
        agent.push_line(&dex, line);
    }
    agent.on_request(&dex, &f_turn2.request.to_string()).unwrap();
    agent.step(&dex, 64).unwrap();

    let rep = nc2000_bot::analysis::report(&agent, &dex, 0, 1);
    let rows = rep["actions"].as_array().unwrap();
    assert_eq!(rows.len(), 3);
    let mut saw_reflect = false;
    let mut saw_recover = false;
    for row in rows {
        let dom = row["dominated"].as_bool().unwrap();
        let has_reason = row["reason"].is_string();
        assert_eq!(
            dom, has_reason,
            "analysis::report row contradicted dominated vs reason: {row}"
        );
        match row["input"].as_str().unwrap() {
            "move reflect" => {
                saw_reflect = true;
                assert!(dom);
                assert_eq!(row["reason"].as_str(), Some("that side condition is already up"));
            }
            "move recover" => {
                saw_recover = true;
                assert!(!dom);
                assert!(row["reason"].is_null());
            }
            "move surf" => {
                assert!(!dom);
                assert!(row["reason"].is_null());
            }
            other => panic!("unexpected action row: {other}"),
        }
    }
    assert!(saw_reflect && saw_recover);
}

#[test]
fn all_blind_entrypoints_prune_public_dominated_actions_before_ucb_and_compose_with_masks() {
    let dex = conformance::load_dex();
    let own = vec![mk_set(
        "Gengar",
        "Leftovers",
        &["Ice Punch", "Thunderbolt", "Perish Song", "Destiny Bond"],
        50,
    )];
    let foe = vec![mk_set("Snorlax", "Leftovers", &["Body Slam", "Splash"], 50)];
    let mut b = Battle::from_fixture(&dex, "1,2,3,4", &own, &foe).unwrap();
    let pool = MetaPool {
        teams: vec![MetaTeam {
            id: "lax".into(),
            sets: foe.clone(),
            weight: Some(1.0),
        }],
    };
    let pool_arc = std::sync::Arc::new(pool.clone());
    let cfg = RmConfig {
        iterations: 128,
        c: 0.4,
        ..Default::default()
    };

    // Advance BlindAgent and OpenAgent through Team Preview so they hold live game state.
    let mut blind_agent = BlindAgent::new(cfg.clone(), pool_arc, None, 314);
    let mut open_agent = OpenAgent::new(cfg.clone(), None, 314);
    let prev_choices = b.legal_choices(&dex, 0);
    let _ = blind_agent.choose(&b, &dex, 0, &prev_choices);
    let _ = open_agent.choose(&b, &dex, 0, &prev_choices);

    b.choose(&dex, 0, "team 1").unwrap();
    b.choose(&dex, 1, "team 1").unwrap();
    let mut ch = PlayerChannel::new(0);
    let frame = ch.frame(&mut b, &dex).unwrap();

    // 1. ProtocolAgent entrypoint
    let mut proto = ProtocolAgent::new(&dex, 0, pool.clone(), cfg.clone(), 314);
    proto.set_own_team(own.clone());
    for line in &frame.lines {
        proto.push_line(&dex, line);
    }
    proto.on_request(&dex, &frame.request.to_string()).unwrap();
    proto.step(&dex, 128).unwrap();
    let proto_search = proto.search().unwrap();

    // 2. Direct BlindSearch::from_observation entrypoint (used by WasmBlindSearcher)
    let (local, obs) = LocalPlayer::new(&b, &dex, 0).unwrap();
    let mut bel = Belief::new(&dex, &pool, &obs);
    bel.sync(&dex, &obs);
    let pos = local.position(&dex, &mut bel, &obs, &mut SplitMix64::new(55)).unwrap();
    let mut obs_search = BlindSearch::from_observation(&pos, &dex, cfg.clone(), 0, 999);
    obs_search.step(&dex, &bel, &obs, 128);

    // 3. BlindAgent & OpenAgent root_policy (which reflects visit / decision support)
    let legal = b.legal_choices(&dex, 0);
    let blind_pol = blind_agent.root_policy(&b, &dex, 0, &legal);
    let open_pol = open_agent.root_policy(&b, &dex, 0, &legal);

    let perish = SearchChoice::Move(dex.moves.id("perishsong").unwrap());
    let dbond = SearchChoice::Move(dex.moves.id("destinybond").unwrap());
    for (i, &act) in legal.iter().enumerate() {
        let is_dom = act == perish || act == dbond;
        assert_eq!(proto_search.dominated()[i], is_dom);
        assert_eq!(obs_search.dominated()[i], is_dom);
        if is_dom {
            assert_eq!(
                proto_search.visits()[i],
                0,
                "ProtocolAgent must allocate 0 visits to publicly dominated {act:?}"
            );
            assert_eq!(
                obs_search.visits()[i],
                0,
                "BlindSearch::from_observation must allocate 0 visits to publicly dominated {act:?}"
            );
            assert_eq!(blind_pol[i], 0.0);
            assert_eq!(open_pol[i], 0.0);
        }
    }
    assert_eq!(proto_search.visits().iter().sum::<u32>(), 128);
    assert_eq!(obs_search.visits().iter().sum::<u32>(), 128);

    // 4. Explicit `mask_actions` called AFTER `from_observation` never accidentally un-prunes
    //    dominated actions when a live action stays allowed, yet falls back cleanly when ONLY
    //    dominated actions stay allowed.
    let mut partial_mask_search = BlindSearch::from_observation(&pos, &dex, cfg.clone(), 0, 999);
    // Allow Ice Punch (0), mask Thunderbolt (1), allow Perish Song (2) & Destiny Bond (3):
    // Perish Song and Destiny Bond must STILL get 0 visits because Ice Punch is undominated!
    partial_mask_search.mask_actions(&[true, false, true, true]);
    partial_mask_search.step(&dex, &bel, &obs, 32);
    assert_eq!(partial_mask_search.visits(), &[32, 0, 0, 0]);

    let mut only_dom_allowed = BlindSearch::from_observation(&pos, &dex, cfg, 0, 999);
    only_dom_allowed.mask_actions(&[false, false, true, false]);
    only_dom_allowed.step(&dex, &bel, &obs, 16);
    assert_eq!(only_dom_allowed.visits(), &[0, 0, 16, 0]);
    assert_eq!(only_dom_allowed.best(), Some(perish));
}
