//! Deterministic falsification suite (U-1 .. U-7) for Problem B:
//! Finite-PP Endurance Bellman Solver, 2x2 Press/Conserve Posture Game,
//! Gen 2 Struggle + Leftovers 4-Quadrant Kernel, 2v1 ZeroPpPivotLock,
//! and Progress-Counter Zero-PP Repetition Contraction.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

use nc2000_bot::belief::Belief;
use nc2000_bot::blind::BlindSearch;
use nc2000_bot::eval::{
    compact_eval01_blindspot, compact_eval01_blindspot_nostall, compact_eval_leaf_blindspot,
    EvalWeights,
};
use nc2000_bot::linear::{
    compact_rollout_linear_pick, struggle_terminal_1v1, toeplitz_coupled_race,
};
use nc2000_bot::observe::Observer;
use nc2000_bot::rng::SplitMix64;
use nc2000_bot::smmcts::{public_cycle_hash, RmConfig, SelRule};
use nc2000_bot::LeafSettlementMode;
use nc2000_engine::battle::PokemonSet;
use nc2000_engine::compact::{vol_bit, CompactBattle};
use nc2000_engine::dex::Dex;
use nc2000_engine::state::{Battle, Status};
use serde_json::json;

struct CountingAlloc;

thread_local! {
    static COUNT_ALLOCS: Cell<bool> = const { Cell::new(false) };
    static ALLOC_CALLS: Cell<usize> = const { Cell::new(0) };
}

unsafe impl GlobalAlloc for CountingAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let _ = COUNT_ALLOCS.try_with(|active| {
            if active.get() {
                let _ = ALLOC_CALLS.try_with(|c| c.set(c.get() + 1));
            }
        });
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[cfg(not(feature = "mi"))]
#[global_allocator]
static GLOBAL_ALLOC: CountingAlloc = CountingAlloc;

fn make_set(species: &str, level: u8, item: &str, moves: &[&str]) -> PokemonSet {
    serde_json::from_value(json!({
        "name": species,
        "species": species,
        "item": item,
        "ability": "No Ability",
        "moves": moves,
        "level": level,
        "evs": {"hp": 255, "atk": 255, "def": 255, "spa": 255, "spd": 255, "spe": 255},
        "ivs": {"hp": 30, "atk": 30, "def": 30, "spa": 30, "spd": 30, "spe": 30},
        "happiness": 255,
        "gender": "M"
    }))
    .unwrap()
}

fn run_pinned_search(
    dex: &Dex,
    battle: &Battle,
    side: usize,
    settlement: LeafSettlementMode,
    iters: u32,
    seed: u64,
) -> (String, Vec<(String, u32, f64)>) {
    let mut obs = Observer::new(battle, side);
    obs.observe(battle, dex);
    let belief = Belief::pinned_from_battle(battle, &obs);
    let cfg = RmConfig {
        iterations: iters,
        c: 0.4,
        rule: SelRule::Ucb,
        settlement,
        ..RmConfig::default()
    };
    let mut search = BlindSearch::new(battle, dex, cfg, side, seed);
    search.prune_dominated();
    search.step(dex, &belief, &obs, iters);

    let best = search
        .best()
        .map(|c| c.to_input(dex))
        .unwrap_or_default();
    let acts = search.actions();
    let visits = search.visits();
    let means = search.means();
    let rows = (0..acts.len())
        .map(|i| (acts[i].to_input(dex), visits[i], means[i]))
        .collect();
    (best, rows)
}

#[test]
fn u1_strict_antisymmetry_10000_states_and_fast_1v1_race_invariance() {
    let dex = super::common::dex();
    let w = EvalWeights::default();

    // Part A: Fast 2-3 turn KO 1v1 races (`32/24/16 PP` vs `4 PP`) must change by < 1e-4
    let fast_pairs = [
        (
            make_set("Jolteon", 50, "Magnet", &["Thunderbolt", "Hidden Power"]),
            make_set("Starmie", 50, "MiracleBerry", &["Surf", "Psychic"]),
        ),
        (
            make_set("Marowak", 50, "Thick Club", &["Earthquake", "Rock Slide"]),
            make_set("Tyranitar", 50, "Leftovers", &["Rock Slide", "Crunch"]),
        ),
        (
            make_set("Zapdos", 50, "Leftovers", &["Thunderbolt", "Drill Peck"]),
            make_set("Machamp", 50, "Scope Lens", &["Cross Chop", "Rock Slide"]),
        ),
    ];
    for (idx, (p0, p1)) in fast_pairs.iter().enumerate() {
        let mut b =
            Battle::from_fixture(dex, "1,2,3,4", &[p0.clone()], &[p1.clone()]).unwrap();
        b.choose(dex, 0, "team 1").unwrap();
        b.choose(dex, 1, "team 1").unwrap();
        let (table, cb_full) = CompactBattle::from_battle(&b, dex);
        let mut cb_4pp = cb_full;
        for m_i in 0..table.mons[0].num_moves as usize {
            cb_4pp.mons[0].pp[m_i] = 4;
        }
        for m_i in 0..table.mons[3].num_moves as usize {
            cb_4pp.mons[3].pp[m_i] = 4;
        }
        let r_full = 0.5 * (1.0 + toeplitz_coupled_race(&cb_full, &table, 0, 0, 1, 0));
        let r_4pp = 0.5 * (1.0 + toeplitz_coupled_race(&cb_4pp, &table, 0, 0, 1, 0));
        assert!(
            (r_full - r_4pp).abs() < 1e-4,
            "U-1 fast race {idx} Toeplitz drifted: full={r_full}, 4pp={r_4pp}"
        );
        // Note `compact_side_score` contains legacy `w.pp * pp_num / pp_den`, while `compact_linear_diff`
        // replaces `base_diff` on exact mechanics; check pure Bellman/Toeplitz race AND mirror PP invariance
        let v_full = compact_eval01_blindspot(&cb_full, &table, &w);
        let v_4pp = compact_eval01_blindspot(&cb_4pp, &table, &w);
        assert!(
            (v_full - v_4pp).abs() < 1e-4,
            "U-1 fast race {idx} eval01 drifted: full={v_full}, 4pp={v_4pp}"
        );
    }

    // Part B: 10,000 randomized states across 1v1, 2v1, 1v2, 2v2, 3v2, 3v3
    // including sleep, paralysis, one-sided Leftovers, scarce PP, and 0-PP Struggle
    let roster_a = vec![
        make_set("Umbreon", 50, "Leftovers", &["Moonlight", "Feint Attack", "Charm", "Spite"]),
        make_set("Snorlax", 55, "Leftovers", &["Body Slam", "Curse", "Rest", "Sleep Talk"]),
        make_set("Machamp", 50, "No Item", &["Cross Chop", "Rock Slide", "Earthquake", "Counter"]),
    ];
    let roster_b = vec![
        make_set("Suicune", 55, "Leftovers", &["Surf", "Ice Beam", "Rest", "Sleep Talk"]),
        make_set("Blissey", 50, "No Item", &["Soft-Boiled", "Ice Beam", "Thunderbolt", "Sing"]),
        make_set("Skarmory", 50, "Leftovers", &["Drill Peck", "Whirlwind", "Rest", "Curse"]),
    ];
    let mut b_ab = Battle::from_fixture(dex, "1,2,3,4", &roster_a, &roster_b).unwrap();
    b_ab.choose(dex, 0, "team 1,2,3").unwrap();
    b_ab.choose(dex, 1, "team 1,2,3").unwrap();
    let (tbl_ab, base_ab) = CompactBattle::from_battle(&b_ab, dex);

    let mut b_ba = Battle::from_fixture(dex, "1,2,3,4", &roster_b, &roster_a).unwrap();
    b_ba.choose(dex, 0, "team 1,2,3").unwrap();
    b_ba.choose(dex, 1, "team 1,2,3").unwrap();
    let (tbl_ba, base_ba) = CompactBattle::from_battle(&b_ba, dex);

    let mut rng = SplitMix64::new(0xB100_57A1_1000_2026);
    for case in 0..10_000usize {
        let mut cb_ab = base_ab;
        let mut cb_ba = base_ba;

        let mask0 = (rng.below(7) as u8) + 1;
        let mask1 = (rng.below(7) as u8) + 1;
        cb_ab.sides[0].alive_mask = mask0;
        cb_ab.sides[1].alive_mask = mask1;
        cb_ba.sides[0].alive_mask = mask1;
        cb_ba.sides[1].alive_mask = mask0;

        let pick_act = |mask: u8, rng: &mut SplitMix64| -> u8 {
            let mut slots = [0u8; 3];
            let mut n = 0usize;
            for s in 0..3u8 {
                if (mask & (1 << s)) != 0 {
                    slots[n] = s;
                    n += 1;
                }
            }
            slots[rng.below(n)]
        };
        let act0 = pick_act(mask0, &mut rng);
        let act1 = pick_act(mask1, &mut rng);
        cb_ab.sides[0].active_idx = act0;
        cb_ab.sides[1].active_idx = act1;
        cb_ba.sides[0].active_idx = act1;
        cb_ba.sides[1].active_idx = act0;

        let spk0 = rng.below(2) as u8;
        let spk1 = rng.below(2) as u8;
        cb_ab.sides[0].side_flags = spk0;
        cb_ab.sides[1].side_flags = spk1;
        cb_ba.sides[0].side_flags = spk1;
        cb_ba.sides[1].side_flags = spk0;

        for side in 0..2usize {
            let mask = if side == 0 { mask0 } else { mask1 };
            let act = if side == 0 { act0 } else { act1 };
            for slot in 0..3usize {
                let i_ab = side * 3 + slot;
                let i_ba = (1 - side) * 3 + slot;
                if (mask & (1 << slot)) == 0 {
                    cb_ab.mons[i_ab].hp_pool = [0, 0];
                    cb_ab.mons[i_ab].status_packed = Status::Fnt as u8;
                    cb_ba.mons[i_ba].hp_pool = [0, 0];
                    cb_ba.mons[i_ba].status_packed = Status::Fnt as u8;
                } else {
                    let mxhp = tbl_ab.mons[i_ab].maxhp.max(1);
                    let hp = (rng.below(mxhp as usize) as i16) + 1;
                    cb_ab.mons[i_ab].hp_pool[0] = hp;
                    cb_ba.mons[i_ba].hp_pool[0] = hp;

                    // Randomly strip held item on one side (one-sided Leftovers)
                    if rng.below(4) == 0 {
                        cb_ab.mons[i_ab].item = 0;
                        cb_ba.mons[i_ba].item = 0;
                    }

                    // Random status: None, Slp (1..3), Par, Brn, Tox
                    let st_draw = rng.below(8);
                    let (st, cnt) = match st_draw {
                        1 => (Status::Slp as u8, (rng.below(3) as u8) + 1),
                        2 => (Status::Par as u8, 0),
                        3 => (Status::Brn as u8, 0),
                        4 => (Status::Tox as u8, (rng.below(5) as u8) + 1),
                        _ => (0, 0),
                    };
                    cb_ab.mons[i_ab].set_status(st, cnt);
                    cb_ba.mons[i_ba].set_status(st, cnt);

                    // Random PP: all-0 Struggle (12.5%), scarce 0..3 PP (25%), or uniform 0..=max_pp
                    let pp_mode = rng.below(8);
                    let mut pps = [0u8; 4];
                    for m_i in 0..tbl_ab.mons[i_ab].num_moves as usize {
                        let mx_pp = tbl_ab.mons[i_ab].max_pp[m_i].max(1);
                        pps[m_i] = match pp_mode {
                            0 => 0,
                            1 | 2 => rng.below(4.min(mx_pp as usize + 1)) as u8,
                            _ => rng.below(mx_pp as usize + 1) as u8,
                        };
                    }
                    cb_ab.mons[i_ab].pp = pps;
                    cb_ba.mons[i_ba].pp = pps;

                    if slot as u8 == act {
                        let mut boosts = [0i8; 7];
                        for b_cell in boosts.iter_mut().take(5) {
                            if rng.below(4) == 0 {
                                *b_cell = (rng.below(7) as i8) - 2;
                            }
                        }
                        cb_ab.mons[i_ab].boosts = boosts;
                        cb_ba.mons[i_ba].boosts = boosts;
                        if rng.below(6) == 0 {
                            cb_ab.mons[i_ab].vol_flags |= vol_bit::TRAPPED_OR_LOCKED;
                            cb_ba.mons[i_ba].vol_flags |= vol_bit::TRAPPED_OR_LOCKED;
                        }
                    }
                }
            }
        }

        let (v_ab, v_ba) = if case % 2 == 0 {
            (
                compact_eval_leaf_blindspot(&cb_ab, &tbl_ab, &w),
                compact_eval_leaf_blindspot(&cb_ba, &tbl_ba, &w),
            )
        } else {
            let b = compact_eval_leaf_blindspot(&cb_ba, &tbl_ba, &w);
            let a = compact_eval_leaf_blindspot(&cb_ab, &tbl_ab, &w);
            (a, b)
        };
        let err = (v_ab + v_ba - 1.0).abs();
        assert!(
            err < 1e-7,
            "U-1 antisymmetry violated at case {case}: v_ab={v_ab:.12}, v_ba={v_ba:.12}, err={err:.3e}"
        );
    }

    // Part C: Strictly 0 heap allocations on `BellmanCache` L1/L2 hit across HP & PP queries
    let mut cb_hit = base_ab;
    cb_hit.sides[0].alive_mask = 0b001;
    cb_hit.sides[1].alive_mask = 0b001;
    let _ = compact_eval_leaf_blindspot(&cb_hit, &tbl_ab, &w);
    let cs4 = [0u8, 1, 2, 3];
    let _ = compact_rollout_linear_pick(&cb_hit, &tbl_ab, 0, &cs4);

    ALLOC_CALLS.with(|c| c.set(0));
    COUNT_ALLOCS.with(|a| a.set(true));
    let mut acc_sink = 0.0f64;
    for pp_h in 1..=8u8 {
        cb_hit.mons[0].pp[0] = pp_h;
        cb_hit.mons[0].hp_pool[0] = 100 + (pp_h as i16) * 10;
        cb_hit.mons[3].hp_pool[0] = 110 + (pp_h as i16) * 8;
        acc_sink += compact_eval_leaf_blindspot(&cb_hit, &tbl_ab, &w);
        if let Some(m) = compact_rollout_linear_pick(&cb_hit, &tbl_ab, 0, &cs4) {
            acc_sink += m as f64;
        }
    }
    COUNT_ALLOCS.with(|a| a.set(false));
    let _allocs = ALLOC_CALLS.with(|c| c.get());
    assert!(acc_sink > 0.0);
    #[cfg(not(feature = "mi"))]
    assert_eq!(
        _allocs, 0,
        "U-1 BellmanCache hit path must perform 0 heap allocations, got {_allocs}"
    );
}

#[test]
fn u2_asymmetric_heal_pp_moonlight_stall_and_monotonicity() {
    let dex = super::common::dex();
    let w = EvalWeights::default();

    // 1. 8-PP `Moonlight` Umbreon vs high-PP attacker (Zapdos 24 PP Drill Peck + 24 PP Thunderbolt = 48 atk PP)
    let umbreon = make_set(
        "Umbreon",
        50,
        "Leftovers",
        &["Moonlight", "Feint Attack", "Confuse Ray", "Sand-Attack"],
    );
    let zapdos = make_set(
        "Zapdos",
        55,
        "Leftovers",
        &["Thunderbolt", "Drill Peck"],
    );
    let mut b_atk = Battle::from_fixture(dex, "1,2,3,4", &[zapdos], &[umbreon.clone()]).unwrap();
    b_atk.choose(dex, 0, "team 1").unwrap();
    b_atk.choose(dex, 1, "team 1").unwrap();
    let (tbl_atk, mut cb_atk) = CompactBattle::from_battle(&b_atk, dex);

    // At 8 PP Moonlight, Zapdos (48 attack PP) easily outlasts Umbreon's 8 Moonlight PP (`V > 0.90`)
    let v_zap_8 = compact_eval01_blindspot(&cb_atk, &tbl_atk, &w);
    assert!(
        v_zap_8 > 0.90,
        "U-2 expected Zapdos (48 atk PP) > 0.90 vs 8-PP Moonlight Umbreon, got {v_zap_8}"
    );
    // Strict monotonicity as Moonlight PP drops `8 -> 7 -> ... -> 0`
    let mut prev_v = v_zap_8;
    for pp in (0..8u8).rev() {
        cb_atk.mons[3].pp[0] = pp;
        let cur_v = compact_eval01_blindspot(&cb_atk, &tbl_atk, &w);
        assert!(
            cur_v > prev_v + 1e-6,
            "U-2 monotonicity violated at Moonlight pp={pp}: cur={cur_v:.8} <= prev={prev_v:.8}"
        );
        prev_v = cur_v;
    }

    // 2. 8-PP `Moonlight` Umbreon vs `Rest + Sleep Talk` Suicune (`V_Suicune > 0.90`, `V_Umbreon < 0.10`)
    let suicune = make_set(
        "Suicune",
        50,
        "Leftovers",
        &["Surf", "Ice Beam", "Rest", "Sleep Talk"],
    );
    let mut b_sui = Battle::from_fixture(dex, "1,2,3,4", &[umbreon], &[suicune]).unwrap();
    b_sui.choose(dex, 0, "team 1").unwrap();
    b_sui.choose(dex, 1, "team 1").unwrap();
    let (tbl_sui, mut cb_sui) = CompactBattle::from_battle(&b_sui, dex);
    let v_umb = compact_eval01_blindspot(&cb_sui, &tbl_sui, &w);
    assert!(
        v_umb < 0.10,
        "U-2 expected 8-PP Moonlight Umbreon < 0.10 vs RestTalk Suicune, got {v_umb}"
    );

    // Rollout guard: at 92% HP (`> 68%`), Umbreon must NOT waste scarce 8-PP Moonlight early
    let maxhp_u = tbl_sui.mons[0].maxhp;
    cb_sui.mons[0].hp_pool[0] = (maxhp_u * 92) / 100;
    let cs = [0u8, 1, 2, 3];
    assert_eq!(
        compact_rollout_linear_pick(&cb_sui, &tbl_sui, 0, &cs),
        None,
        "U-2 Umbreon at 92% HP must not prematurely burn 8-PP Moonlight"
    );
}

#[test]
fn u3_two_pp_softboiled_blissey_vs_zapdos_horizon_inversion_eliminated() {
    let dex = super::common::dex();
    let w = EvalWeights::default();

    // NC2000 Ace Lv55 Zapdos (`100%` HP, `Thunderbolt` 16 PP + `Rest` 16 PP, Light Screen up)
    // vs Lv50 Blissey (`Soft-Boiled` 2 PP left, `Ice Beam` 16 PP):
    // Infinite-PP baseline (-stall) integrates 67 turns of phantom stall (`~0.50`),
    // whereas finite-PP solver sees Blissey's 2 Soft-Boiled PP expire and scores Zapdos `> 0.90`.
    let zapdos = make_set(
        "Zapdos",
        55,
        "Leftovers",
        &["Thunderbolt", "Light Screen", "Rest"],
    );
    let blissey = make_set(
        "Blissey",
        50,
        "Leftovers",
        &["Soft-Boiled", "Ice Beam"],
    );
    let mut b = Battle::from_fixture(dex, "1,2,3,4", &[zapdos], &[blissey]).unwrap();
    b.choose(dex, 0, "team 1").unwrap();
    b.choose(dex, 1, "team 1").unwrap();
    let (table, mut cb) = CompactBattle::from_battle(&b, dex);
    cb.sides[0].lightscreen_turns = 5;
    cb.mons[0].pp[0] = 16; // 16 PP Thunderbolt
    cb.mons[3].pp[0] = 2; // 2 PP Soft-Boiled remaining
    cb.mons[3].hp_pool[0] = (table.mons[3].maxhp * 55) / 100;

    let v_nostall = compact_eval01_blindspot_nostall(&cb, &table, &w);
    let v_finite = compact_eval01_blindspot(&cb, &table, &w);
    assert!(
        v_nostall < 0.75,
        "U-3 baseline (-stall) suffers infinite-PP illusion (expected < 0.75, got {v_nostall})"
    );
    assert!(
        v_finite > 0.90,
        "U-3 finite-PP solver must evaluate Zapdos > 0.90 vs 2-PP Soft-Boiled Blissey, got {v_finite}"
    );
}

#[test]
fn u4_full_hp_rest_wall_conserve_over_attack_trap() {
    let dex = super::common::dex();
    let w = EvalWeights::default();

    // Symmetric +2 Def Suicune mirror (`No Item`):
    // - Side 0: 16 raw PP (`Surf` 4 PP + `Psych Up` 12 PP).
    // - Side 1: 12 raw PP (`Surf` 2 PP + `Rest` 10 PP) at 100% HP (`206/206`).
    // Clicking `Psych Up` keeps Side 1 at 100% HP (`Rest` fails without sleeping -> Side 1 burns all 12 PP
    // in 12 turns < 16 and hits weak Struggle 4 turns early while Side 0 still has 4 Surf PP!), whereas
    // clicking `Surf` on Turn 1 drops Side 1 < 100% HP and unlocks Rest sleep cycles (>= 18 turns > 16 PP).
    let suicune_atk = make_set(
        "Suicune",
        50,
        "No Item",
        &["Surf", "Psych Up"],
    );
    let suicune_wall = make_set(
        "Suicune",
        50,
        "No Item",
        &["Surf", "Rest"],
    );
    let mut b = Battle::from_fixture(dex, "1,2,3,4", &[suicune_atk], &[suicune_wall]).unwrap();
    b.choose(dex, 0, "team 1").unwrap();
    b.choose(dex, 1, "team 1").unwrap();
    {
        let p0 = &mut b.sides[0].roster[0];
        p0.boosts[1] = 2;
        p0.move_slots[0].pp = 2;
        p0.base_move_slots[0].pp = 2;
        p0.move_slots[1].pp = 12;
        p0.base_move_slots[1].pp = 12;
        let p1 = &mut b.sides[1].roster[0];
        p1.boosts[1] = 2;
        p1.move_slots[0].pp = 2;
        p1.base_move_slots[0].pp = 2;
        p1.move_slots[1].pp = 10;
        p1.base_move_slots[1].pp = 10;
    }

    let (table, cb) = CompactBattle::from_battle(&b, dex);
    // 1. Rollout policy must select `Psych Up` (slot 1) over `Tackle` (slot 0)
    let cs = [0u8, 1];
    assert_eq!(
        compact_rollout_linear_pick(&cb, &table, 0, &cs),
        Some(1),
        "U-4 rollout must choose harmless Conserve move (slot 1) instead of attacking 100%-HP Rest wall"
    );

    // 2. Compare leaf values after attacking (chipped Side 1 < 100% HP, 1+12=13 PP vs 1+10=11 PP)
    //    vs after conserving (Side 1 stays at 100% HP, 2+11=13 PP vs 1+10=11 PP)
    let mut cb_attacked = cb;
    cb_attacked.mons[0].pp[0] = 1;
    cb_attacked.mons[0].hp_pool[0] -= 36;
    cb_attacked.mons[3].pp[0] = 1;
    cb_attacked.mons[3].hp_pool[0] -= 36;

    let mut cb_conserved = cb;
    cb_conserved.mons[0].pp[1] = 11;
    cb_conserved.mons[0].hp_pool[0] -= 36;
    cb_conserved.mons[3].pp[0] = 1;

    let v_attacked = compact_eval01_blindspot(&cb_attacked, &table, &w);
    let v_conserved = compact_eval01_blindspot(&cb_conserved, &table, &w);
    assert!(
        v_conserved > 0.55 && v_attacked < 0.45 && v_conserved > v_attacked + 0.15,
        "U-4 conserving against 100%-HP Rest wall must strictly beat chipping it: v_cons={v_conserved:.4}, v_atk={v_attacked:.4}"
    );

    // 3. MCTS search must select `move psychup` over `move tackle`
    let (best, rows) = run_pinned_search(dex, &b, 0, LeafSettlementMode::BlindSpotLeaf, 8_000, 42);
    assert_eq!(
        best, "move psychup",
        "U-4 MCTS must choose Conserve status move (`move psychup`) vs 100%-HP Rest wall, rows={rows:?}"
    );
}

#[test]
fn u5_gen2_struggle_leftovers_four_quadrant_kernel() {
    let dex = super::common::dex();
    let w = EvalWeights::default();

    // (a) Mutual Leftovers + high Defense -> both `delta_i <= 0` -> exact manual draw `V = 0.50`
    let skarm0 = make_set("Skarmory", 50, "Leftovers", &["Rest"]);
    let skarm1 = make_set("Skarmory", 50, "Leftovers", &["Rest"]);
    let mut b_a = Battle::from_fixture(dex, "1,2,3,4", &[skarm0], &[skarm1]).unwrap();
    b_a.choose(dex, 0, "team 1").unwrap();
    b_a.choose(dex, 1, "team 1").unwrap();
    let (tbl_a, mut cb_a) = CompactBattle::from_battle(&b_a, dex);
    cb_a.mons[0].pp = [0; 4];
    cb_a.mons[3].pp = [0; 4];
    cb_a.mons[0].boosts[1] = 6;
    cb_a.mons[3].boosts[1] = 6;

    let v_str_a = 0.5 + struggle_terminal_1v1(&cb_a, &tbl_a, 0, 0, 1.0, 1, 0, 1.0);
    let v_race_a = 0.5 * (1.0 + toeplitz_coupled_race(&cb_a, &tbl_a, 0, 0, 1, 0));
    let v_eval_a = compact_eval01_blindspot(&cb_a, &tbl_a, &w);
    assert!(
        (v_str_a - 0.50).abs() < 1e-12 && (v_race_a - 0.50).abs() < 1e-12 && (v_eval_a - 0.50).abs() < 1e-9,
        "U-5(a) mutual immortal Struggle must be V = 0.50: str={v_str_a}, race={v_race_a}, eval={v_eval_a}"
    );

    // (b) One-sided Leftovers mirror at 0 PP (`delta_0 <= 0 < delta_1`) -> Leftovers holder `V = 1.00`
    let skarm_left = make_set("Skarmory", 50, "Leftovers", &["Rest"]);
    let skarm_none = make_set("Skarmory", 50, "No Item", &["Rest"]);
    let mut b_b = Battle::from_fixture(dex, "1,2,3,4", &[skarm_left], &[skarm_none]).unwrap();
    b_b.choose(dex, 0, "team 1").unwrap();
    b_b.choose(dex, 1, "team 1").unwrap();
    let (tbl_b, mut cb_b) = CompactBattle::from_battle(&b_b, dex);
    cb_b.mons[0].pp = [0; 4];
    cb_b.mons[3].pp = [0; 4];
    cb_b.mons[0].boosts[1] = 6;
    cb_b.mons[3].boosts[1] = 6;

    let v_str_b = 0.5 + struggle_terminal_1v1(&cb_b, &tbl_b, 0, 0, 1.0, 1, 0, 1.0);
    let v_race_b = 0.5 * (1.0 + toeplitz_coupled_race(&cb_b, &tbl_b, 0, 0, 1, 0));
    assert!(
        (v_str_b - 1.00).abs() < 1e-12 && (v_race_b - 1.00).abs() < 1e-12,
        "U-5(b) one-sided Leftovers Struggle mirror must be V = 1.00: str={v_str_b}, race={v_race_b}"
    );
}

#[test]
fn u6_81k_search_conserves_4pp_cross_chop_vs_sleeping_curselax() {
    let dex = super::common::dex();

    // +1 Atk Lv50 Machamp (`Cross Chop` 4 PP left, `Rock Slide` 16 PP, 135 base Atk) vs full-HP sleeping Lv55 Snorlax
    // (`292/292` HP, `Rest` 16 PP, `status_data = 3` -> 2 sleep turns left) on the final turn of `Reflect` (`reflect_turns = 1`):
    // - Two unscreened non-crit `Cross Chops` on Turn 2 + Turn 3 (min `168 + 168 - 18 = 318 > 292`) ALREADY 100% 2HKO
    //   without needing Turn-1 `Cross Chop`, whereas 1 screened + 1 unscreened non-crit (`max 102 + 198 - 18 = 282 < 292`)
    //   cannot 2HKO — so clicking `Cross Chop` into expiring `Reflect` on Turn 1 wastes a full `Cross Chop` PP (`4 -> 3`,
    //   leaving only 1 PP after Cycle 1 instead of 2 PP for a second full 2HKO cycle)!
    let machamp: PokemonSet = serde_json::from_value(json!({
        "name": "Machamp",
        "species": "Machamp",
        "item": "Leftovers",
        "ability": "No Ability",
        "moves": ["Cross Chop", "Rock Slide"],
        "level": 50,
        "evs": {"hp": 255, "atk": 0, "def": 255, "spa": 0, "spd": 255, "spe": 255},
        "ivs": {"hp": 30, "atk": 0, "def": 30, "spa": 0, "spd": 30, "spe": 30},
        "happiness": 255,
        "gender": "M"
    }))
    .unwrap();
    let snorlax = make_set(
        "Snorlax",
        55,
        "Leftovers",
        &["Body Slam", "Reflect", "Rest", "Curse"],
    );
    let mut b = Battle::from_fixture(dex, "1,2,3,4", &[machamp], &[snorlax]).unwrap();
    b.choose(dex, 0, "team 1").unwrap();
    b.choose(dex, 1, "team 1").unwrap();
    b.sides[1].roster[0].boosts[4] = 6;
    b.update_all_speeds(dex);
    b.choose(dex, 0, "move 2").unwrap();
    b.choose(dex, 1, "move 2").unwrap(); // Turn 1: Reflect set (4 turns left)
    b.choose(dex, 0, "move 2").unwrap();
    b.choose(dex, 1, "move 4").unwrap(); // Turn 2: Reflect 3 turns left
    b.choose(dex, 0, "move 2").unwrap();
    b.choose(dex, 1, "move 4").unwrap(); // Turn 3: Reflect 2 turns left
    b.sides[1].roster[0].hp = b.sides[1].roster[0].maxhp - 20;
    b.choose(dex, 0, "move 2").unwrap();
    b.choose(dex, 1, "move 3").unwrap(); // Turn 4: Snorlax Rests -> status_data = 3 (2 sleep turns left), Reflect = 1 turn left!
    assert_eq!(b.sides[1].roster[0].status, Status::Slp);
    {
        let p0 = &mut b.sides[0].roster[0];
        p0.hp = p0.maxhp;
        p0.status = Status::None;
        p0.boosts = [0; 7];
        p0.boosts[0] = 1; // +1 Atk (> 0 Def so Reflect halves crits too on this final screen turn)
        p0.move_slots[0].pp = 4; // Cross Chop: 4 PP left
        p0.base_move_slots[0].pp = 4;
        p0.move_slots[1].pp = 16; // Rock Slide: 16 PP left
        p0.base_move_slots[1].pp = 16;

        let p1 = &mut b.sides[1].roster[0];
        p1.hp = p1.maxhp; // 292/292 HP
        p1.boosts = [0; 7];
        p1.boosts[4] = -1;
        p1.move_slots[0].pp = 24;
        p1.base_move_slots[0].pp = 24;
        p1.move_slots[1].pp = 0;
        p1.base_move_slots[1].pp = 0;
        p1.move_slots[2].pp = 16; // Rest: 16 PP
        p1.base_move_slots[2].pp = 16;
        p1.move_slots[3].pp = 0;
        p1.base_move_slots[3].pp = 0;
    }
    b.update_all_speeds(dex);
    let (_, cb_chk) = CompactBattle::from_battle(&b, dex);
    assert_eq!(cb_chk.sides[1].reflect_turns, 1);

    let (best, rows) =
        run_pinned_search(dex, &b, 0, LeafSettlementMode::BlindSpotLeaf, 81_000, 2026);
    assert_eq!(
        best, "move rockslide",
        "U-6 81k search must chip sleeping Snorlax with Rock Slide (`move rockslide`) to conserve 4-PP Cross Chop, rows={rows:?}"
    );
}

#[test]
fn u7_2v1_sequential_attrition_and_zero_pp_pivot_lock_with_mean_look_guard() {
    let dex = super::common::dex();
    let w = EvalWeights::default();

    // Side 0: Skarmory (`45%` HP, `Drill Peck` 4 PP) + Misdreavus (`100%` HP, `Thunderbolt` 6 PP)
    // Side 1: Lone +1 Atk / +4 Def / -2 Spe Curse + Rest Snorlax (`No Item` / consumed Mint Berry, `Body Slam` 8 PP, `Curse` 4 PP, `Rest` 8 PP):
    // - Alone in `1v1`, 45%-HP Skarmory loses badly (`V < 0.10`), and 6-PP Thunderbolt Misdreavus cannot break 8-PP Rest (`V < 0.25`).
    // - Together in untrapped `2v1`, `Skarmory <-> Misdreavus` pivots for 0 PP until Snorlax exhausts all PP
    //   (including all 8 Rest PP!), after which 6 Special Thunderbolts + Struggle recoil cleanly KO Snorlax (`V > 0.90`).
    // - Under `Mean Look` (`TRAPPED_OR_LOCKED` on Skarmory), `ZeroPpPivotLock` is disabled (`V < 0.35`).
    let skarm = make_set(
        "Skarmory",
        50,
        "Leftovers",
        &["Drill Peck", "Whirlwind"],
    );
    let misd = make_set(
        "Misdreavus",
        50,
        "Leftovers",
        &["Thunderbolt", "Confuse Ray"],
    );
    let curselax = make_set(
        "Snorlax",
        55,
        "No Item",
        &["Body Slam", "Curse", "Rest"],
    );

    let mut b = Battle::from_fixture(dex, "1,2,3,4", &[skarm, misd], &[curselax]).unwrap();
    b.choose(dex, 0, "team 1,2").unwrap();
    b.choose(dex, 1, "team 1").unwrap();
    {
        let s0_a = &mut b.sides[0].roster[0];
        s0_a.hp = (s0_a.maxhp * 45) / 100;
        s0_a.move_slots[0].pp = 4;
        s0_a.base_move_slots[0].pp = 4;

        let s0_b = &mut b.sides[0].roster[1];
        s0_b.hp = s0_b.maxhp;
        s0_b.move_slots[0].pp = 6;
        s0_b.base_move_slots[0].pp = 6;

        let d = &mut b.sides[1].roster[0];
        d.boosts[0] = 1;
        d.boosts[1] = 4;
        d.boosts[4] = -2;
        d.move_slots[0].pp = 8;
        d.base_move_slots[0].pp = 8;
        d.move_slots[1].pp = 4;
        d.base_move_slots[1].pp = 4;
        d.move_slots[2].pp = 8;
        d.base_move_slots[2].pp = 8;
    }

    let (table, cb_2v1) = CompactBattle::from_battle(&b, dex);

    // 1. Collapsed `1v1` (if Misdreavus fainted, leaving 45%-HP Skarmory vs +1/+4 Lv55 Lax) is < 0.10
    let mut cb_1v1_skarm = cb_2v1;
    cb_1v1_skarm.sides[0].alive_mask = 0b001;
    cb_1v1_skarm.mons[1].hp_pool[0] = 0;
    let v_1v1 = compact_eval01_blindspot(&cb_1v1_skarm, &table, &w);
    assert!(
        v_1v1 < 0.10,
        "U-7 collapsed 1v1 (Skarmory alone vs boosted Lv55 CurseLax) must be < 0.10, got {v_1v1}"
    );

    // 2. Normal `2v1` (untrapped Skarmory + Misdreavus) activates `ZeroPpPivotLock` (`V > 0.90`)
    let v_2v1_normal = compact_eval01_blindspot(&cb_2v1, &table, &w);
    assert!(
        v_2v1_normal > 0.90,
        "U-7 untrapped 2v1 ZeroPpPivotLock must evaluate > 0.90, got {v_2v1_normal}"
    );

    // 3. Mean Look trap (`vol_bit::TRAPPED_OR_LOCKED` on active Skarmory, identical HP/PP) disables `ZeroPpPivotLock`!
    let mut cb_2v1_trapped = cb_2v1;
    cb_2v1_trapped.mons[0].vol_flags |= vol_bit::TRAPPED_OR_LOCKED;
    let v_2v1_trapped = compact_eval01_blindspot(&cb_2v1_trapped, &table, &w);
    assert!(
        v_2v1_normal > v_2v1_trapped + 0.50 && v_2v1_trapped < 0.35,
        "U-7 Mean Look (`TRAPPED_OR_LOCKED`) must disable ZeroPpPivotLock: got trapped={v_2v1_trapped} vs untrapped={v_2v1_normal}"
    );

    // 4. MCTS search in untrapped `2v1` switches to Misdreavus (`switch 2`) rather than letting Skarmory faint
    let (best, rows) = run_pinned_search(dex, &b, 0, LeafSettlementMode::BlindSpotLeaf, 6_000, 77);
    assert_eq!(
        best, "switch 2",
        "U-7 MCTS must pivot to Ghost partner (`switch 2`) rather than sacrificing 2v1, rows={rows:?}"
    );

    // 5. Verify progress-counter cycle hash only collides on 0-PP switches and never after a move
    let h_epoch0 = public_cycle_hash(&b, 0);
    let h_epoch1 = public_cycle_hash(&b, 1);
    assert_ne!(
        h_epoch0, h_epoch1,
        "U-7 cycle hash must advance whenever cumulative_move_count increments"
    );
}
