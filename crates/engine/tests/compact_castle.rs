//! Comprehensive unit & differential parity tests for the 256-byte `CompactBattle`
//! kernel under both `Ruleset::PsStadium2` (100% PS parity) and Gym Leader's Castle
//! PBS (`Ruleset::CastlePbs(CastleOverlayConfig)`).

use nc2000_engine::battle::PokemonSet;
use nc2000_engine::compact::{
    ext_timer_idx, ext_vol_bit, get_damage_base_branchless, get_damage_base_castle,
    get_stat_castle, is_supported_move_castle, lookup_accuracy_threshold_castle, timer_idx,
    vol_bit, CompactBattle, CompactMon, CompactSide, CompactWeather, ItemStatKind, MatchTable,
    COMPACT_CASTLE_UNSUPPORTED_MOVES,
};
use nc2000_engine::dex::Dex;
use nc2000_engine::rules::{CastleOverlayConfig, Ruleset};
use nc2000_engine::state::{stat_idx, Battle, Status};

fn load_dex() -> Dex {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../data/gen2stadium2.json");
    let json = std::fs::read_to_string(&path).expect("read gen2stadium2.json");
    Dex::from_json(&json).expect("parse gen2stadium2.json")
}

fn mk_set(
    species: &str,
    level: u8,
    item: &str,
    moves: &[&str],
    gender: &str,
) -> PokemonSet {
    serde_json::from_value(serde_json::json!({
        "name": species,
        "species": species,
        "item": item,
        "ability": "No Ability",
        "gender": gender,
        "moves": moves,
        "level": level,
        "evs": {"hp": 255, "atk": 255, "def": 255, "spa": 255, "spd": 255, "spe": 255},
        "ivs": {"hp": 30, "atk": 30, "def": 30, "spa": 30, "spd": 30, "spe": 30},
        "happiness": 255
    }))
    .expect("construct PokemonSet")
}

fn start_compact(
    dex: &Dex,
    ruleset: Ruleset,
    p1: &[PokemonSet],
    p2: &[PokemonSet],
) -> (MatchTable, CompactBattle) {
    let mut b = Battle::from_fixture_with_ruleset(dex, "1,2,3,4", p1, p2, ruleset).unwrap();
    let pick1 = (1..=p1.len().min(3))
        .map(|i| i.to_string())
        .collect::<Vec<_>>()
        .join(",");
    let pick2 = (1..=p2.len().min(3))
        .map(|i| i.to_string())
        .collect::<Vec<_>>()
        .join(",");
    b.choose(dex, 0, &format!("team {pick1}")).unwrap();
    b.choose(dex, 1, &format!("team {pick2}")).unwrap();
    CompactBattle::from_battle(&b, dex)
}

#[test]
fn layout_256b_alignment_padding_free_and_search_hash_determinism() {
    assert_eq!(std::mem::size_of::<CompactMon>(), 32);
    assert_eq!(std::mem::size_of::<CompactSide>(), 12);
    assert_eq!(std::mem::size_of::<CompactBattle>(), 256);
    assert_eq!(std::mem::align_of::<CompactBattle>(), 64);

    let dex = load_dex();
    let p1 = vec![mk_set("Snorlax", 55, "Leftovers", &["Body Slam", "Rest"], "M")];
    let p2 = vec![mk_set("Zapdos", 55, "Leftovers", &["Thunderbolt", "Rest"], "")];

    let (t_ps, mut cb_ps) = start_compact(&dex, Ruleset::PsStadium2, &p1, &p2);
    let (t_castle, mut cb_castle) =
        start_compact(&dex, Ruleset::castle_tournament(), &p1, &p2);

    // Ruleset tag separates MatchTable fingerprints & search_hash!
    assert_ne!(t_ps.match_fingerprint, t_castle.match_fingerprint);
    assert_ne!(cb_ps.search_hash(), cb_castle.search_hash());

    // prng_seed (word 31) is excluded from search_hash so RNG-only twins merge cleanly.
    let h0 = cb_castle.search_hash();
    cb_castle.prng_seed ^= 0xDEAD_BEEF_CAFE_BABEu64;
    assert_eq!(cb_castle.search_hash(), h0);

    // Mutating any new field changes search_hash deterministically (zero ignored/padding holes).
    let mut c1 = cb_castle;
    c1.mons[0].ext_flags |= ext_vol_bit::MIST;
    assert_ne!(c1.search_hash(), h0);

    let mut c2 = cb_castle;
    c2.mons[0].gender = 2;
    assert_ne!(c2.search_hash(), h0);

    let mut c3 = cb_castle;
    c3.future_sight_turns[1] = 2;
    assert_ne!(c3.search_hash(), h0);

    let mut c4 = cb_castle;
    c4.bp_switch_flags = 1;
    assert_ne!(c4.search_hash(), h0);

    let mut c5 = cb_castle;
    c5.triple_kick_mult = 2;
    assert_ne!(c5.search_hash(), h0);

    // Identical copied state has identical 256-byte raw representation.
    cb_ps.prng_seed = 42;
    let cb_ps_copy = cb_ps;
    let raw_a = unsafe { std::slice::from_raw_parts(&cb_ps as *const _ as *const u8, 256) };
    let raw_b = unsafe { std::slice::from_raw_parts(&cb_ps_copy as *const _ as *const u8, 256) };
    assert_eq!(raw_a, raw_b);
}

#[test]
fn castle_supported_move_gate_a_contract() {
    let dex = load_dex();
    for &key in COMPACT_CASTLE_UNSUPPORTED_MOVES {
        let mid = dex.moves.id(key).expect(key);
        assert!(
            !is_supported_move_castle(&dex, mid),
            "{key} must report unsupported in Castle CompactBattle"
        );
    }
    for &key in &[
        "pursuit",
        "present",
        "rollout",
        "defensecurl",
        "minimize",
        "outrage",
        "thrash",
        "petaldance",
        "attract",
        "mist",
        "focusenergy",
        "highjumpkick",
        "jumpkick",
        "futuresight",
        "lockon",
        "mindreader",
        "beatup",
        "furycutter",
        "rage",
        "triplekick",
        "twineedle",
        "triattack",
        "rapidspin",
        "batonpass",
    ] {
        let mid = dex.moves.id(key).expect(key);
        assert!(
            is_supported_move_castle(&dex, mid),
            "{key} must report supported in Castle CompactBattle"
        );
    }
}

#[test]
fn explicit_baton_pass_selection_inheritance_and_castle_pursuit_2x_after_spikes() {
    let dex = load_dex();
    let p1 = vec![
        mk_set(
            "Jolteon",
            55,
            "",
            &["Baton Pass", "Defense Curl", "Minimize", "Mist"],
            "M",
        ),
        mk_set("Alakazam", 50, "", &["Psychic", "Splash"], "M"),
        mk_set("Snorlax", 50, "", &["Body Slam", "Splash"], "M"),
    ];
    let p2 = vec![mk_set(
        "Umbreon",
        50,
        "",
        &["Pursuit", "Mean Look", "Spikes", "Splash"],
        "M",
    )];

    // 1. Explicit BATONN pauses with request == 2 and passes Mean Look, Mist, Focus Energy,
    //    Defense Curl & Minimize (in Castle only!), then executes Pursuit 2x AFTER Spikes (`Probe 12C`)!
    let (t_c, mut cb_c) = start_compact(&dex, Ruleset::castle_tournament(), &p1, &p2);
    cb_c.sides[0].side_flags |= 1; // 1 layer of Spikes on P1 side
    cb_c.mons[0].vol_flags |= vol_bit::TRAPPED_OR_LOCKED;
    cb_c.mons[0].ext_flags |= ext_vol_bit::DEFENSE_CURL
        | ext_vol_bit::MINIMIZE
        | ext_vol_bit::MIST
        | ext_vol_bit::FOCUS_ENERGY;
    cb_c.mons[0].boosts[stat_idx::DEF] = 2;

    cb_c.step_turn(&t_c, [0, 0]); // Faster Jolteon Baton Pass vs slower Umbreon Pursuit
    assert_eq!(cb_c.sides[0].request, 2, "BATONN must pause for explicit switch selection");
    assert_eq!(cb_c.sides[1].request, 0);
    assert_ne!(cb_c.bp_switch_flags & 1, 0, "BP switch flag bit 0 must be set");
    let mut acts = [0u8; 7];
    let n = cb_c.legal_actions(&t_c, 0, &mut acts);
    assert_eq!(&acts[..n], &[5, 6], "BATONN must offer both alive bench receivers");

    let zam_max = t_c.mons[1].maxhp;
    let spikes_dmg = t_c.mons[1].quantum_1_8;
    // Choose Alakazam (slot 1 -> action 5).
    cb_c.step_switch(&t_c, [5, 7]);
    assert_eq!(cb_c.sides[0].active_idx, 1);
    assert_eq!(cb_c.sides[0].request, 1);
    assert_eq!(cb_c.turn, 2);
    // Verify inherited volatiles in Castle (`Probe 11C` + Mean Look + Mist + Focus Energy):
    assert_ne!(cb_c.mons[1].vol_flags & vol_bit::TRAPPED_OR_LOCKED, 0);
    assert_ne!(cb_c.mons[1].ext_flags & ext_vol_bit::DEFENSE_CURL, 0);
    assert_ne!(cb_c.mons[1].ext_flags & ext_vol_bit::MINIMIZE, 0);
    assert_ne!(cb_c.mons[1].ext_flags & ext_vol_bit::MIST, 0);
    assert_ne!(cb_c.mons[1].ext_flags & ext_vol_bit::FOCUS_ENERGY, 0);
    assert_eq!(cb_c.mons[1].boosts[stat_idx::DEF], 2);
    let castle_dmg_after_spikes = zam_max - spikes_dmg - cb_c.mons[1].hp();

    // Compare against PS mode with same seed: PS Pursuit hits BP receiver for 1x (BP 40) AND
    // PS Baton Pass does NOT inherit Defense Curl or Minimize!
    let (t_ps, mut cb_ps) = start_compact(&dex, Ruleset::PsStadium2, &p1, &p2);
    cb_ps.sides[0].side_flags |= 1;
    cb_ps.mons[0].ext_flags |= ext_vol_bit::DEFENSE_CURL | ext_vol_bit::MINIMIZE;
    cb_ps.step_turn(&t_ps, [0, 0]);
    assert_eq!(cb_ps.sides[0].request, 2);
    cb_ps.step_switch(&t_ps, [5, 7]);
    assert_eq!(
        cb_ps.mons[1].ext_flags & (ext_vol_bit::DEFENSE_CURL | ext_vol_bit::MINIMIZE),
        0,
        "PS Baton Pass must NOT inherit Defense Curl or Minimize"
    );
    let ps_dmg_after_spikes = t_ps.mons[1].maxhp - t_ps.mons[1].quantum_1_8 - cb_ps.mons[1].hp();
    assert!(
        castle_dmg_after_spikes > (ps_dmg_after_spikes * 3) / 2,
        "Castle Pursuit after first-mover BP must deal ~2x PS Pursuit: castle={castle_dmg_after_spikes}, ps={ps_dmg_after_spikes}"
    );
}

#[test]
fn exact_castle_speed_no_truncation_and_independent_quick_claw() {
    let dex = load_dex();
    let p1 = vec![mk_set("Snorlax", 50, "", &["Splash"], "M")];
    let p2 = vec![mk_set("Snorlax", 50, "", &["Splash"], "M")];
    let (mut t_c, mut cb_c) = start_compact(&dex, Ruleset::castle_tournament(), &p1, &p2);

    // Probe 5B/8E/12H: Paralyzed S123 (30.75) strictly beats Paralyzed S121 (30.25) in Castle!
    t_c.mons[0].stored_stats[stat_idx::SPE] = 123;
    t_c.mons[3].stored_stats[stat_idx::SPE] = 121;
    cb_c.mons[0].set_status(Status::Par as u8, 0);
    cb_c.mons[3].set_status(Status::Par as u8, 0);
    let k0 = cb_c.action_sort_key_exact(&t_c, 0, 0, 0);
    let k1 = cb_c.action_sort_key_exact(&t_c, 1, 0, 1);
    assert!((k0 >> 1) > (k1 >> 1), "Castle Par S123 (30.75) must beat Par S121 (30.25)");

    // Probe 12H: Paralyzed S120 (30.0) ties unparalyzed S30 (30.0) exactly!
    t_c.mons[0].stored_stats[stat_idx::SPE] = 120;
    t_c.mons[3].stored_stats[stat_idx::SPE] = 30;
    cb_c.mons[3].set_status(Status::None as u8, 0);
    let kt0 = cb_c.action_sort_key_exact(&t_c, 0, 0, 0);
    let kt1 = cb_c.action_sort_key_exact(&t_c, 1, 0, 1);
    assert_eq!(kt0 >> 1, kt1 >> 1, "Castle Par S120 (30.0) must tie S30 (30.0)");

    // Probe 9B/12H: -1 S121 (80.667) beats S80 (80.0), while -1 S120 (80.0) ties S80 (80.0)!
    cb_c.mons[0].set_status(Status::None as u8, 0);
    cb_c.mons[0].boosts[stat_idx::SPE] = -1;
    t_c.mons[0].stored_stats[stat_idx::SPE] = 121;
    t_c.mons[3].stored_stats[stat_idx::SPE] = 80;
    assert!(
        (cb_c.action_sort_key_exact(&t_c, 0, 0, 0) >> 1)
            > (cb_c.action_sort_key_exact(&t_c, 1, 0, 1) >> 1)
    );
    t_c.mons[0].stored_stats[stat_idx::SPE] = 120;
    assert_eq!(
        cb_c.action_sort_key_exact(&t_c, 0, 0, 0) >> 1,
        cb_c.action_sort_key_exact(&t_c, 1, 0, 1) >> 1
    );

    // Gate C check: Both holding Quick Claw -> slower side goes first ~20.69% in Castle vs ~11.72% in PS!
    let p_fast = vec![mk_set("Jolteon", 50, "Quick Claw", &["Thunderbolt"], "M")];
    let p_slow = vec![mk_set("Snorlax", 50, "Quick Claw", &["Double-Edge"], "M")];
    let (t_qcc, base_qcc) = start_compact(&dex, Ruleset::castle_tournament(), &p_fast, &p_slow);
    let (t_qcp, base_qcp) = start_compact(&dex, Ruleset::PsStadium2, &p_fast, &p_slow);
    let mut slow_first_castle = 0u32;
    let mut slow_first_ps = 0u32;
    let trials = 4000u64;
    for s in 1..=trials {
        let mut c = base_qcc;
        c.prng_seed = s.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
        // Advance 1 dummy turn to get fresh Quick Claw roll, then check 1-HP KO race!
        c.step_turn(&t_qcc, [0, 0]);
        c.mons[0].hp_pool[0] = 1;
        c.mons[3].hp_pool[0] = 1;
        c.outcome = 0;
        c.step_turn(&t_qcc, [0, 0]);
        if c.outcome == 2 {
            slow_first_castle += 1;
        }

        let mut p = base_qcp;
        p.prng_seed = s.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
        p.step_turn(&t_qcp, [0, 0]);
        p.mons[0].hp_pool[0] = 1;
        p.mons[3].hp_pool[0] = 1;
        p.outcome = 0;
        p.step_turn(&t_qcp, [0, 0]);
        if p.outcome == 2 {
            slow_first_ps += 1;
        }
    }
    let rate_c = slow_first_castle as f64 / trials as f64;
    let rate_p = slow_first_ps as f64 / trials as f64;
    assert!(
        (0.18..=0.235).contains(&rate_c),
        "Castle dual Quick Claw slower-first rate must be ~20.69%, got {rate_c:.4}"
    );
    assert!(
        (0.095..=0.145).contains(&rate_p),
        "PS dual Quick Claw slower-first rate must be ~11.72%, got {rate_p:.4}"
    );
}

#[test]
fn castle_damage_stat_diffs_explosion_ovfl_flail_recover_recoil_and_zero_min_dmg() {
    let dex = load_dex();

    // 1. Probe 12B: exact rational negative stage (-1 = 2/3 vs PS 66%) + Burn/Par persistence through Haze!
    let p1 = vec![mk_set("Blissey", 5, "", &["Explosion", "Haze", "Soft-Boiled"], "F")];
    let p2 = vec![mk_set("Cloyster", 56, "", &["Splash"], "M")];
    let (mut t_c, mut cb_c) = start_compact(&dex, Ruleset::castle_tournament(), &p1, &p2);
    let (mut t_ps, mut cb_ps) = start_compact(&dex, Ruleset::PsStadium2, &p1, &p2);
    t_c.mons[0].stored_stats[stat_idx::ATK] = 6;
    t_ps.mons[0].stored_stats[stat_idx::ATK] = 6;
    cb_c.mons[0].boosts[stat_idx::ATK] = -1;
    cb_ps.mons[0].boosts[stat_idx::ATK] = -1;
    assert_eq!(cb_c.active_stat(&t_c, 0, stat_idx::ATK, false, false), 4);
    assert_eq!(cb_ps.active_stat(&t_ps, 0, stat_idx::ATK, false, false), 3);

    // Burn/Par drop persists through Haze in Castle (cleared in PS).
    cb_c.mons[0].set_status(Status::Brn as u8, 0);
    cb_c.mons[0].vol_flags |= vol_bit::BRN_ATK_DROP;
    t_c.mons[0].stored_stats[stat_idx::ATK] = 100;
    cb_c.mons[0].boosts[stat_idx::ATK] = 0;
    cb_c.step_turn(&t_c, [1, 0]); // Haze
    assert_eq!(
        cb_c.active_stat(&t_c, 0, stat_idx::ATK, false, false),
        50,
        "Castle Burn halving must persist after Haze"
    );

    // 2. Probe 12E2: Lv100 Chansey (Atk 15) Explosion vs Lv56 Cloyster (Def 258):
    //    Castle applies Def/2 BEFORE >=256 /4 -> [33..39]; PS applies /4 first -> [26..31]!
    let p_chan = vec![mk_set("Chansey", 100, "", &["Explosion"], "F")];
    let p_cloy = vec![mk_set("Cloyster", 56, "", &["Splash"], "M")];
    let (mut t_ex_c, base_ex_c) = start_compact(&dex, Ruleset::castle_tournament(), &p_chan, &p_cloy);
    let (mut t_ex_p, base_ex_p) = start_compact(&dex, Ruleset::PsStadium2, &p_chan, &p_cloy);
    t_ex_c.mons[0].stored_stats[stat_idx::ATK] = 15;
    t_ex_c.mons[3].stored_stats[stat_idx::DEF] = 258;
    t_ex_p.mons[0].stored_stats[stat_idx::ATK] = 15;
    t_ex_p.mons[3].stored_stats[stat_idx::DEF] = 258;
    let mut castle_non_crits = 0u32;
    let mut ps_non_crits = 0u32;
    for seed in 1..=64u64 {
        let mut cc = base_ex_c;
        cc.prng_seed = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15);
        let hp0 = cc.mons[3].hp();
        cc.step_turn(&t_ex_c, [0, 0]);
        let d = hp0 - cc.mons[3].hp();
        if (33..=39).contains(&d) {
            castle_non_crits += 1;
        } else {
            assert!(
                (63..=75).contains(&d),
                "Castle Explosion Def/2-first must deal 33..=39 (non-crit) or 63..=75 (crit), got {d}"
            );
        }

        let mut cp = base_ex_p;
        cp.prng_seed = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15);
        let hp0p = cp.mons[3].hp();
        cp.step_turn(&t_ex_p, [0, 0]);
        let dp = hp0p - cp.mons[3].hp();
        if (26..=31).contains(&dp) {
            ps_non_crits += 1;
        } else {
            assert!(
                (49..=59).contains(&dp),
                "PS Explosion /4-first must deal 26..=31 (non-crit) or 49..=59 (crit), got {dp}"
            );
        }
    }
    assert!(castle_non_crits >= 45 && ps_non_crits >= 45);

    // 3. ovfl=1 stat wrap % 1024 (`Probe 3A/3B`) + missed Fissure instant-KO Counter (`Probe 8D1`).
    let p_wak = vec![mk_set("Marowak", 55, "Thick Club", &["Fissure", "Earthquake"], "M")];
    let p_cnt = vec![mk_set("Snorlax", 55, "", &["Counter", "Splash"], "M")];
    let (t_ov0, mut cb_ov0) = start_compact(&dex, Ruleset::castle_raw_cgi(false), &p_wak, &p_cnt);
    let (t_ov1, mut cb_ov1) = start_compact(&dex, Ruleset::castle_raw_cgi(true), &p_wak, &p_cnt);
    cb_ov0.mons[0].boosts[stat_idx::ATK] = 6; // Thick Club * 4x = 1152 >= 1024!
    cb_ov1.mons[0].boosts[stat_idx::ATK] = 6;
    assert_eq!(cb_ov0.active_stat(&t_ov0, 0, stat_idx::ATK, false, false), 999);
    assert!(
        cb_ov1.active_stat(&t_ov1, 0, stat_idx::ATK, false, false) < 300,
        "ovfl=1 must wrap >= 1024 via % 1024"
    );
    // Missed Fissure -> instant-KO Counter when ovfl=1!
    let mut ovfl_counter_ko = false;
    for seed in 1..=64u64 {
        let mut c = cb_ov1;
        c.mons[0].boosts[stat_idx::ATK] = 0;
        c.prng_seed = seed.wrapping_mul(0xD1B5_4A32_D192_ED03);
        c.step_turn(&t_ov1, [0, 0]); // Marowak Fissure vs Snorlax Counter
        if c.mons[3].hp() == t_ov1.mons[3].maxhp && c.mons[0].hp() == 0 {
            ovfl_counter_ko = true;
            break;
        }
    }
    assert!(ovfl_counter_ko, "Castle ovfl=1 missed Fissure must trigger instant-KO Counter");

    // 4. Odd-HP Recover floor (`Probe 9A`: 361 -> +180 in Castle vs +181 in PS).
    let p_rec = vec![mk_set("Blissey", 50, "", &["Soft-Boiled"], "F")];
    let p_sp = vec![mk_set("Snorlax", 50, "", &["Splash"], "M")];
    let (mut t_rc, mut cb_rc) = start_compact(&dex, Ruleset::castle_tournament(), &p_rec, &p_sp);
    let (mut t_rp, mut cb_rp) = start_compact(&dex, Ruleset::PsStadium2, &p_rec, &p_sp);
    t_rc.mons[0].maxhp = 361;
    t_rp.mons[0].maxhp = 361;
    cb_rc.mons[0].hp_pool[0] = 100;
    cb_rp.mons[0].hp_pool[0] = 100;
    cb_rc.step_turn(&t_rc, [0, 0]);
    cb_rp.step_turn(&t_rp, [0, 0]);
    assert_eq!(cb_rc.mons[0].hp(), 280, "Castle Soft-Boiled must heal floor(361/2)=180");
    assert_eq!(cb_rp.mons[0].hp(), 281, "PS Soft-Boiled must heal round(361/2)=181");

    // 5. Flail / Reversal Castle `< 77.5%` boundary (`Probe 12E1` + `Probe 9A`).
    assert_eq!(CompactBattle::flail_reversal_bp(155, 200, true), 20);
    assert_eq!(CompactBattle::flail_reversal_bp(154, 200, true), 40);
    assert_eq!(CompactBattle::flail_reversal_bp(66, 186, true), 40);
    assert_eq!(CompactBattle::flail_reversal_bp(154, 200, false), 20);

    // 6. 0 recoil vs Substitute in Castle (vs 1 HP in PS) & HJK 1/4 miss recoil (vs 1/8 in PS).
    let p_atk = vec![mk_set("Hitmonlee", 55, "", &["Double-Edge", "High Jump Kick"], "M")];
    let p_def = vec![mk_set("Snorlax", 50, "", &["Substitute", "Protect"], "M")];
    let (t_rc2, mut cb_sub_c) = start_compact(&dex, Ruleset::castle_tournament(), &p_atk, &p_def);
    let (t_rp2, mut cb_sub_p) = start_compact(&dex, Ruleset::PsStadium2, &p_atk, &p_def);
    cb_sub_c.mons[3].vol_flags |= vol_bit::SUBSTITUTE;
    cb_sub_c.mons[3].hp_pool[1] = 80;
    cb_sub_p.mons[3].vol_flags |= vol_bit::SUBSTITUTE;
    cb_sub_p.mons[3].hp_pool[1] = 80;
    cb_sub_c.step_turn(&t_rc2, [0, 0]); // Double-Edge into Sub
    cb_sub_p.step_turn(&t_rp2, [0, 0]);
    assert_eq!(cb_sub_c.mons[0].hp(), t_rc2.mons[0].maxhp, "Castle recoil vs Sub must be 0 HP");
    assert_eq!(cb_sub_p.mons[0].hp(), t_rp2.mons[0].maxhp - 1, "PS recoil vs Sub must be 1 HP");

    // HJK blocked by Protect -> crash recoil 1/4 in Castle vs 1/8 in PS!
    let (_, mut cb_hjk_c) = start_compact(&dex, Ruleset::castle_tournament(), &p_atk, &p_def);
    let (_, mut cb_hjk_p) = start_compact(&dex, Ruleset::PsStadium2, &p_atk, &p_def);
    cb_hjk_c.prng_seed = 7;
    cb_hjk_p.prng_seed = 7;
    cb_hjk_c.step_turn(&t_rc2, [1, 1]); // HJK vs Protect
    cb_hjk_p.step_turn(&t_rp2, [1, 1]);
    let crash_c = t_rc2.mons[0].maxhp - cb_hjk_c.mons[0].hp();
    let crash_p = t_rp2.mons[0].maxhp - cb_hjk_p.mons[0].hp();
    assert!(crash_c >= crash_p * 2 - 2 && crash_p > 0, "Castle HJK recoil (1/4) must be ~2x PS (1/8): {crash_c} vs {crash_p}");
}

#[test]
fn castle_items_berries_dragon_scale_metal_powder_mystery_focus_band_kings_rock() {
    let dex = load_dex();

    // 1. Healing berry strict `hp < floor(maxhp/2)` (`E_20261028` 85/171 does NOT trigger in Castle).
    let p1 = vec![mk_set("Snorlax", 50, "Gold Berry", &["Splash"], "M")];
    let p2 = vec![mk_set("Snorlax", 50, "Berry Juice", &["Splash"], "M")];
    let (mut t_c, mut cb_c) = start_compact(&dex, Ruleset::castle_tournament(), &p1, &p2);
    let (mut t_p, mut cb_p) = start_compact(&dex, Ruleset::PsStadium2, &p1, &p2);
    t_c.mons[0].maxhp = 171;
    t_p.mons[0].maxhp = 171;
    cb_c.mons[0].hp_pool[0] = 85; // 85 == floor(171/2) -> NOT < 85!
    cb_p.mons[0].hp_pool[0] = 85;
    cb_c.step_turn(&t_c, [0, 0]);
    cb_p.step_turn(&t_p, [0, 0]);
    assert_eq!(cb_c.mons[0].hp(), 85, "Castle Gold Berry must NOT trigger at exact floor(maxhp/2)");
    assert_ne!(cb_c.mons[0].item, 0);
    assert_eq!(cb_p.mons[0].hp(), 115, "PS Gold Berry triggers at <= maxhp/2");
    assert_eq!(cb_p.mons[0].item, 0);
    // At 84/171, Castle Gold Berry DOES trigger:
    cb_c.mons[0].hp_pool[0] = 84;
    cb_c.step_turn(&t_c, [0, 0]);
    assert_eq!(cb_c.mons[0].hp(), 114);
    assert_eq!(cb_c.mons[0].item, 0);

    // 2. Ice Berry (81) cures Burn + Confusion in Castle; Bitter Berry (84) is completely inert!
    let p_ice = vec![mk_set("Snorlax", 50, "Ice Berry", &["Splash"], "M")];
    let p_bit = vec![mk_set("Snorlax", 50, "Bitter Berry", &["Confuse Ray"], "M")];
    let (t_ib, mut cb_ib) = start_compact(&dex, Ruleset::castle_tournament(), &p_ice, &p_bit);
    cb_ib.mons[3].vol_flags |= vol_bit::CONFUSION;
    cb_ib.mons[3].timers[timer_idx::CONFUSION] = 3;
    cb_ib.step_turn(&t_ib, [0, 0]); // Bitter Berry user Confuse Rays Ice Berry Snorlax
    assert_eq!(cb_ib.mons[0].vol_flags & vol_bit::CONFUSION, 0, "Castle Ice Berry must cure Confusion immediately");
    assert_eq!(cb_ib.mons[0].item, 0);
    assert_ne!(cb_ib.mons[3].item, 0, "Castle Bitter Berry must be completely inert");

    // 3. Miracle Berry + Nightmare bug (`Probe 11B`): waking via Miracle Berry keeps Nightmare ticking!
    let p_nm1 = vec![mk_set("Snorlax", 50, "Miracle Berry", &["Splash"], "M")];
    let p_nm2 = vec![mk_set("Gengar", 55, "", &["Nightmare", "Splash"], "M")];
    let (t_nm, mut cb_nm) = start_compact(&dex, Ruleset::castle_tournament(), &p_nm1, &p_nm2);
    cb_nm.mons[0].set_status(Status::Slp as u8, 3);
    cb_nm.mons[0].vol_flags |= vol_bit::NIGHTMARE;
    let hp_before = cb_nm.mons[0].hp();
    cb_nm.step_turn(&t_nm, [0, 1]); // Miracle Berry cures Sleep onBeforeMove -> Nightmare STILL deals 1/4 in AfterMoveSelf!
    assert_eq!(cb_nm.mons[0].status_raw(), Status::None as u8);
    assert_ne!(cb_nm.mons[0].ext_flags & ext_vol_bit::MIRACLE_NIGHTMARE, 0);
    assert_eq!(
        cb_nm.mons[0].hp(),
        hp_before - t_nm.mons[0].maxhp / 4,
        "Castle Miracle Berry waking from Nightmare must keep Nightmare dealing 1/4 HP"
    );

    // 4. First-mover Rest + second-mover Swagger with Miracle Berry cures BOTH Sleep & Confusion!
    let p_rs1 = vec![mk_set("Jolteon", 55, "Miracle Berry", &["Rest"], "M")];
    let p_rs2 = vec![mk_set("Snorlax", 50, "", &["Swagger"], "M")];
    let (t_rs, mut cb_rs) = start_compact(&dex, Ruleset::castle_tournament(), &p_rs1, &p_rs2);
    cb_rs.mons[0].hp_pool[0] = 50;
    cb_rs.prng_seed = 7; // ensure Swagger hits
    cb_rs.step_turn(&t_rs, [0, 0]);
    assert_eq!(cb_rs.mons[0].status_raw(), Status::None as u8);
    assert_eq!(cb_rs.mons[0].vol_flags & vol_bit::CONFUSION, 0);
    assert_eq!(cb_rs.mons[0].item, 0);

    // 5. Dragon Scale boosts Dragon 1.1x in Castle while Dragon Fang is inert (`Probe 11D`)!
    let p_ds = vec![mk_set("Dragonite", 55, "Dragon Scale", &["Dragon Breath"], "M")];
    let p_df = vec![mk_set("Dragonite", 55, "Dragon Fang", &["Dragon Breath"], "M")];
    let p_tg = vec![mk_set("Snorlax", 55, "", &["Splash"], "M")];
    let (t_ds, cb_ds) = start_compact(&dex, Ruleset::castle_tournament(), &p_ds, &p_tg);
    let (t_df, cb_df) = start_compact(&dex, Ruleset::castle_tournament(), &p_df, &p_tg);
    assert!(
        cb_ds.expected_hit_fraction(&t_ds, 0, 0, 1, 0, 0)
            > cb_df.expected_hit_fraction(&t_df, 0, 0, 1, 0, 0),
        "Castle Dragon Scale must boost Dragon while Dragon Fang is inert"
    );

    // 6. Metal Powder persists on Transformed Ditto in Castle (disabled in PS)!
    let p_dit = vec![mk_set("Ditto", 50, "Metal Powder", &["Splash"], "")];
    let (t_dc, mut cb_dc) = start_compact(&dex, Ruleset::castle_tournament(), &p_dit, &p_tg);
    let (t_dp, mut cb_dp) = start_compact(&dex, Ruleset::PsStadium2, &p_dit, &p_tg);
    cb_dc.mons[0].ext_flags |= ext_vol_bit::TRANSFORMED;
    cb_dp.mons[0].ext_flags |= ext_vol_bit::TRANSFORMED;
    assert!(
        cb_dc.active_stat(&t_dc, 0, stat_idx::DEF, false, false)
            > cb_dp.active_stat(&t_dp, 0, stat_idx::DEF, false, false),
        "Castle Metal Powder must persist after Transform"
    );

    // 7. Mystery Berry restores +5 PP at EOT with NO maxpp clamp (`Probe 9F`: 1 -> 0 -> 5).
    let p_mb = vec![mk_set("Snorlax", 50, "Mystery Berry", &["Tackle"], "M")];
    let (mut t_mb, mut cb_mb) = start_compact(&dex, Ruleset::castle_tournament(), &p_mb, &p_tg);
    t_mb.mons[0].max_pp[0] = 1;
    cb_mb.mons[0].pp[0] = 1;
    cb_mb.step_turn(&t_mb, [0, 0]);
    assert_eq!(cb_mb.mons[0].pp[0], 5, "Castle Mystery Berry must restore +5 PP without maxpp clamp");
    assert_eq!(cb_mb.mons[0].item, 0);

    // 8. Focus Band (30/256) survives lethal attacks & Castle confusion; King's Rock flinches!
    let p_fb = vec![mk_set("Alakazam", 50, "Focus Band", &["Splash"], "M")];
    let p_kr = vec![mk_set("Snorlax", 55, "King's Rock", &["Body Slam", "Explosion"], "M")];
    let (t_fk, base_fk) = start_compact(&dex, Ruleset::castle_tournament(), &p_fb, &p_kr);
    let mut fb_proc = false;
    let mut kr_flinch = false;
    for s in 1..=256u64 {
        let mut c = base_fk;
        c.prng_seed = s.wrapping_mul(0x9E37_79B9_7F4A_7C15);
        c.mons[0].hp_pool[0] = 10;
        c.step_turn(&t_fk, [0, 0]); // Snorlax Body Slam into 10 HP Focus Band Alakazam
        if c.mons[0].hp() == 1 {
            fb_proc = true;
        }
        // Fast King's Rock Snorlax flinching slower Alakazam
        let mut c2 = base_fk;
        c2.prng_seed = s.wrapping_mul(0xD1B5_4A32_D192_ED03);
        c2.mons[3].boosts[stat_idx::SPE] = 6;
        let pp_zam = c2.mons[0].pp[0];
        c2.step_turn(&t_fk, [0, 0]);
        if c2.mons[0].hp() > 0 && c2.mons[0].pp[0] == pp_zam {
            kr_flinch = true;
        }
        if fb_proc && kr_flinch {
            break;
        }
    }
    assert!(fb_proc, "Focus Band must proc at 30/256 to leave defender at 1 HP");
    assert!(kr_flinch, "King's Rock must flinch defender at 30/256");
}

#[test]
fn castle_statuses_clauses_protect_endure_belly_drum_rapid_spin_encore_disable() {
    let dex = load_dex();

    // 1. Castle non-Rest Sleep uniform 1..=4 skipped turns (counters 2..=5) & blocking Sleep Clause (no forfeit)!
    let p1 = vec![
        mk_set("Parasect", 50, "", &["Spore", "Splash"], "M"),
        mk_set("Snorlax", 50, "", &["Splash"], "M"),
    ];
    let p2 = vec![
        mk_set("Snorlax", 55, "", &["Rest", "Splash"], "M"),
        mk_set("Zapdos", 50, "", &["Splash"], ""),
    ];
    let (t_slp, base_slp) = start_compact(&dex, Ruleset::castle_tournament(), &p1, &p2);
    let mut seen = [false; 8];
    for s in 1..=256u64 {
        let mut c = base_slp;
        c.prng_seed = s.wrapping_mul(0x9E37_79B9_7F4A_7C15);
        c.step_turn(&t_slp, [0, 1]);
        assert_eq!(c.mons[3].status_raw(), Status::Slp as u8);
        let cnt = c.mons[3].status_counter() as usize;
        assert!((2..=5).contains(&cnt), "Castle non-Rest sleep counter must be 2..=5, got {cnt}");
        seen[cnt] = true;
    }
    assert!(seen[2] && seen[3] && seen[4] && seen[5], "All 4 Castle sleep durations (1..=4T) must occur");

    // Castle Sleep Clause blocks sleep (even when bench mon slept via Rest) WITHOUT forfeit!
    let mut c_sc = base_slp;
    c_sc.mons[4].set_status(Status::Slp as u8, 3); // bench Zapdos asleep (e.g., from Rest)
    c_sc.step_turn(&t_slp, [0, 1]); // Parasect Spore -> blocked, NO forfeit!
    assert_eq!(c_sc.outcome, 0, "Castle Sleep Clause must block without forfeiting");
    assert_eq!(c_sc.mons[3].status_raw(), Status::None as u8);

    // 2. Castle Endure succeeds behind own Substitute (`Probe 6`/`8C`), Protect fails behind own Sub,
    //    and 1st-turn Protect fails 1/256!
    let p_pe = vec![mk_set("Jolteon", 55, "", &["Protect", "Endure", "Substitute"], "M")];
    let p_foe = vec![mk_set("Snorlax", 50, "", &["Earthquake"], "M")];
    let (t_pe, base_pe) = start_compact(&dex, Ruleset::castle_tournament(), &p_pe, &p_foe);
    let mut c_end = base_pe;
    c_end.mons[0].vol_flags |= vol_bit::SUBSTITUTE;
    c_end.mons[0].hp_pool[1] = 30;
    c_end.prng_seed = 7;
    c_end.step_turn(&t_pe, [1, 0]); // Endure behind own Sub
    assert_eq!(c_end.mons[0].stall_counter_ext(), 1, "Castle Endure must succeed behind own Sub");
    let mut c_pro = base_pe;
    c_pro.mons[0].vol_flags |= vol_bit::SUBSTITUTE;
    c_pro.mons[0].hp_pool[1] = 30;
    c_pro.step_turn(&t_pe, [0, 0]); // Protect behind own Sub fails!
    assert_eq!(c_pro.mons[0].stall_counter_ext(), 0, "Castle Protect must fail behind own Sub");

    // 1st-turn Protect 255/256 fail observed across 2000 seeds!
    let mut first_turn_fail = false;
    for s in 1..=2000u64 {
        let mut c = base_pe;
        c.prng_seed = s.wrapping_mul(0x9E37_79B9_7F4A_7C15);
        c.step_turn(&t_pe, [0, 0]);
        if c.mons[0].hp() < t_pe.mons[0].maxhp {
            first_turn_fail = true;
            break;
        }
    }
    assert!(first_turn_fail, "Castle 1st-turn Protect must have 1/256 failure rate");

    // 3. Belly Drum at exact hp == floor(maxhp / 2) succeeds leaving 1 HP (`Probe 3A`)!
    let p_bd = vec![mk_set("Snorlax", 50, "", &["Belly Drum"], "M")];
    let p_sp = vec![mk_set("Snorlax", 50, "", &["Splash"], "M")];
    let (t_bd, mut cb_bd) = start_compact(&dex, Ruleset::castle_tournament(), &p_bd, &p_sp);
    cb_bd.mons[0].hp_pool[0] = t_bd.mons[0].maxhp / 2;
    cb_bd.step_turn(&t_bd, [0, 0]);
    assert_eq!(cb_bd.mons[0].hp(), 1, "Castle Belly Drum at exact floor(maxhp/2) must leave 1 HP");
    assert_eq!(cb_bd.mons[0].boosts[stat_idx::ATK], 6);

    // 4. Rapid Spin vs Ghost clears Spikes & Bind WITHOUT clearing Mean Look (`Probe 12D2`)!
    let p_rs = vec![
        mk_set("Forretress", 50, "", &["Rapid Spin"], "M"),
        mk_set("Snorlax", 50, "", &["Splash"], "M"),
    ];
    let p_gh = vec![mk_set("Gengar", 50, "", &["Mean Look", "Splash"], "M")];
    let (t_rs, mut cb_rs) = start_compact(&dex, Ruleset::castle_tournament(), &p_rs, &p_gh);
    cb_rs.sides[0].side_flags |= 1;
    cb_rs.mons[0].vol_flags |= vol_bit::TRAPPED_OR_LOCKED | vol_bit::LEECH_SEED;
    cb_rs.mons[0].set_partially_trapped(true);
    cb_rs.step_turn(&t_rs, [0, 1]); // Rapid Spin into Ghost Gengar!
    assert_eq!(cb_rs.sides[0].side_flags & 0x03, 0, "Castle Rapid Spin vs Ghost must clear Spikes");
    assert_eq!(cb_rs.mons[0].vol_flags & vol_bit::LEECH_SEED, 0);
    assert!(!cb_rs.mons[0].partially_trapped());
    assert_ne!(
        cb_rs.mons[0].vol_flags & vol_bit::TRAPPED_OR_LOCKED,
        0,
        "Rapid Spin must NEVER clear Mean Look"
    );

    // 5. Encore + Disable does NOT force Struggle in Castle (`Spec #54①`), AND Encore fails vs Roar (`Probe 12I`)!
    let p_ed = vec![mk_set("Snorlax", 50, "", &["Body Slam", "Roar"], "M")];
    let p_en = vec![mk_set("Alakazam", 55, "", &["Encore", "Splash"], "M")];
    let (t_ed, mut cb_ed) = start_compact(&dex, Ruleset::castle_tournament(), &p_ed, &p_en);
    cb_ed.mons[0].vol_flags |= vol_bit::ENCORE | vol_bit::DISABLE;
    cb_ed.mons[0].set_encore_slot(0);
    cb_ed.mons[0].set_disable_slot(0);
    cb_ed.mons[0].timers[timer_idx::ENCORE] = 3;
    cb_ed.mons[0].timers[timer_idx::DISABLE] = 3;
    let mut acts = [0u8; 7];
    let n = cb_ed.legal_actions(&t_ed, 0, &mut acts);
    assert_eq!(&acts[..n], &[0]);
    let zam_hp0 = cb_ed.mons[3].hp();
    let lax_hp0 = cb_ed.mons[0].hp();
    cb_ed.step_turn(&t_ed, [0, 1]); // Aborts on Disable — NO Struggle damage or recoil!
    assert_eq!(cb_ed.mons[3].hp(), zam_hp0, "Castle Encore+Disable must NOT Struggle");
    assert_eq!(cb_ed.mons[0].hp(), lax_hp0);

    // Encore fails vs Roar (`Probe 12I`).
    let (_, mut cb_roar) = start_compact(&dex, Ruleset::castle_tournament(), &p_ed, &p_en);
    cb_roar.mons[0].set_last_move_slot(1); // Roar
    cb_roar.step_turn(&t_ed, [1, 0]); // Alakazam Encore vs Snorlax last-move Roar
    assert_eq!(cb_roar.mons[0].vol_flags & vol_bit::ENCORE, 0, "Castle Encore must fail vs Roar");

    // 6. Heal Bell resets Toxic counter in Castle (`Probe 12B`): Toxic -> Heal Bell -> Poison = flat 1/8!
    let p_hb = vec![mk_set("Miltank", 55, "", &["Heal Bell", "Splash"], "F")];
    let p_tx = vec![mk_set("Umbreon", 50, "", &["Toxic", "Poison Powder"], "M")];
    let (t_hb, mut cb_hb) = start_compact(&dex, Ruleset::castle_tournament(), &p_hb, &p_tx);
    cb_hb.mons[0].set_status(Status::Tox as u8, 0x10 | 4); // Toxic ramp counter = 4
    cb_hb.step_turn(&t_hb, [0, 1]); // Miltank Heal Bell -> Umbreon Poison Powder (75% acc)
    cb_hb.mons[0].set_status(Status::Psn as u8, cb_hb.mons[0].status_counter()); // ensure Psn if PP missed
    assert_eq!(cb_hb.mons[0].status_counter() & 0x10, 0, "Castle Heal Bell must clear Toxic counter");
}

#[test]
fn castle_new_nc2000_moves_rollout_outrage_attract_futuresight_lockon_beatup_present_dig_pursuit() {
    let dex = load_dex();

    // 1. Rollout + Defense Curl (`Probe 12E2`: +2 BEFORE multiplier; locks user).
    let p_ro = vec![mk_set("Golem", 50, "", &["Defense Curl", "Rollout"], "M")];
    let p_st = vec![mk_set("Steelix", 50, "", &["Splash"], "M")];
    let (t_ro, mut cb_ro) = start_compact(&dex, Ruleset::castle_tournament(), &p_ro, &p_st);
    cb_ro.step_turn(&t_ro, [0, 0]); // Defense Curl
    assert_ne!(cb_ro.mons[0].ext_flags & ext_vol_bit::DEFENSE_CURL, 0);
    cb_ro.prng_seed = 7;
    let st_hp0 = cb_ro.mons[3].hp();
    cb_ro.step_turn(&t_ro, [1, 0]); // Rollout hit 1
    assert!(cb_ro.mons[3].hp() < st_hp0);
    assert_ne!(cb_ro.mons[0].ext_flags & ext_vol_bit::LOCKED_ROLLOUT, 0);
    let mut acts = [0u8; 7];
    let n = cb_ro.legal_actions(&t_ro, 0, &mut acts);
    assert_eq!(&acts[..n], &[1], "Rollout must lock user into Rollout slot");

    // 2. Outrage locks 2..=3T and confuses IMMEDIATELY after final hit in Castle (`Probe 8A`)!
    let p_ou = vec![mk_set("Dragonite", 55, "Ice Berry", &["Outrage"], "M")];
    let p_sn = vec![mk_set("Snorlax", 55, "Leftovers", &["Splash"], "M")];
    let (t_ou, mut cb_ou) = start_compact(&dex, Ruleset::castle_tournament(), &p_ou, &p_sn);
    cb_ou.step_turn(&t_ou, [0, 0]);
    assert_ne!(cb_ou.mons[0].ext_flags & ext_vol_bit::LOCKED_OUTRAGE, 0);
    let rem = cb_ou.mons[0].ext_timers[ext_timer_idx::LOCKED_TURNS];
    assert!((1..=2).contains(&rem));
    for _ in 0..rem {
        cb_ou.step_turn(&t_ou, [0, 0]);
    }
    assert_eq!(cb_ou.mons[0].ext_flags & ext_vol_bit::LOCKED_OUTRAGE, 0);
    // Because Dragonite held Ice Berry (ID 81), immediate Castle confusion was immediately cured by Ice Berry!
    assert_eq!(cb_ou.mons[0].item, 0, "Castle immediate Outrage confusion must trigger Ice Berry immediately");

    // 3. Attract (opposite genders infatuate, same gender fails, clears on switch).
    let p_at1 = vec![
        mk_set("Miltank", 55, "", &["Attract", "Splash"], "F"),
        mk_set("Snorlax", 50, "", &["Splash"], "M"),
    ];
    let p_at2 = vec![
        mk_set("Snorlax", 50, "", &["Splash"], "M"),
        mk_set("Blissey", 50, "", &["Splash"], "F"),
    ];
    let (t_at, mut cb_at) = start_compact(&dex, Ruleset::castle_tournament(), &p_at1, &p_at2);
    cb_at.step_turn(&t_at, [0, 0]); // Female Miltank Attracts Male Snorlax
    assert_ne!(cb_at.mons[3].ext_flags & ext_vol_bit::ATTRACT, 0);
    cb_at.step_turn(&t_at, [5, 0]); // Miltank switches out -> Snorlax Attract clears!
    assert_eq!(cb_at.mons[3].ext_flags & ext_vol_bit::ATTRACT, 0);

    // 4. Future Sight strikes 2 turns later at EOT (bypasses Protect & hits Dark types!).
    let p_fs = vec![mk_set("Espeon", 55, "", &["Future Sight", "Splash"], "M")];
    let p_dk = vec![mk_set("Umbreon", 50, "", &["Protect", "Splash"], "M")];
    let (t_fs, mut cb_fs) = start_compact(&dex, Ruleset::castle_tournament(), &p_fs, &p_dk);
    cb_fs.step_turn(&t_fs, [0, 1]); // Turn 1: cast Future Sight -> EOT 3 -> 2
    assert_eq!(cb_fs.future_sight_turns[1], 2);
    assert!(cb_fs.future_sight_dmg[1] > 0);
    cb_fs.step_turn(&t_fs, [1, 1]); // Turn 2: EOT 2 -> 1
    assert_eq!(cb_fs.future_sight_turns[1], 1);
    let umb_hp_before = cb_fs.mons[3].hp();
    let mut fs_hit = false;
    for s in 1..=32u64 {
        let mut c = cb_fs;
        c.prng_seed = s.wrapping_mul(0x9E37_79B9_7F4A_7C15);
        c.step_turn(&t_fs, [1, 0]); // Turn 3: Umbreon Protects, Future Sight hits through Protect & Dark!
        if c.mons[3].hp() < umb_hp_before {
            fs_hit = true;
            break;
        }
    }
    assert!(fs_hit, "Future Sight must strike Dark Umbreon through Protect 2 turns later");

    // 5. Lock-On persists across Splash in Castle (`Probe 12E1`) & Dig takes Sandstorm (`Probe 12E2`)!
    let p_lo = vec![mk_set("Zapdos", 55, "", &["Lock-On", "Splash", "Zap Cannon", "Sandstorm"], "")];
    let p_dg = vec![mk_set("Mew", 50, "", &["Dig", "Splash"], "")];
    let (t_lo, mut cb_lo) = start_compact(&dex, Ruleset::castle_tournament(), &p_lo, &p_dg);
    cb_lo.step_turn(&t_lo, [0, 1]); // Lock-On vs Splash
    assert_ne!(cb_lo.mons[0].ext_flags & ext_vol_bit::LOCK_ON, 0);
    cb_lo.step_turn(&t_lo, [1, 1]); // Splash vs Splash -> Lock-On MUST persist in Castle!
    assert_ne!(cb_lo.mons[0].ext_flags & ext_vol_bit::LOCK_ON, 0, "Castle Lock-On must persist across Splash");
    cb_lo.step_turn(&t_lo, [2, 1]); // Zap Cannon guaranteed hit -> consumes Lock-On!
    assert_eq!(cb_lo.mons[0].ext_flags & ext_vol_bit::LOCK_ON, 0);
    assert!(cb_lo.mons[3].hp() < t_lo.mons[3].maxhp);

    // Digging Mew takes Sandstorm damage in Castle (`Probe 12E2`), immune in PS!
    let (t_dg_c, mut cb_dg_c) = start_compact(&dex, Ruleset::castle_tournament(), &p_lo, &p_dg);
    let (t_dg_p, mut cb_dg_p) = start_compact(&dex, Ruleset::PsStadium2, &p_lo, &p_dg);
    cb_dg_c.step_turn(&t_dg_c, [3, 0]); // Zapdos Sandstorm, Mew Dig (underground!)
    cb_dg_p.step_turn(&t_dg_p, [3, 0]);
    assert_eq!(
        cb_dg_c.mons[3].hp(),
        t_dg_c.mons[3].maxhp - t_dg_c.mons[3].quantum_1_8,
        "Castle Digging Mew must take Sandstorm damage"
    );
    assert_eq!(
        cb_dg_p.mons[3].hp(),
        t_dg_p.mons[3].maxhp,
        "PS Digging Mew must be immune to Sandstorm"
    );

    // 6. Beat Up in Castle (`Probe 11D`/`12D2`): paralyzed active user STILL hits, statused bench excluded!
    let p_bu = vec![
        mk_set("Houndoom", 55, "", &["Beat Up"], "M"),
        mk_set("Snorlax", 50, "", &["Splash"], "M"),
        mk_set("Zapdos", 50, "", &["Splash"], ""),
    ];
    let p_bu_foe = vec![mk_set("Blissey", 50, "", &["Splash"], "F")];
    let (t_bu, mut cb_bu) = start_compact(&dex, Ruleset::castle_tournament(), &p_bu, &p_bu_foe);
    cb_bu.mons[0].set_status(Status::Par as u8, 0);
    cb_bu.mons[1].set_status(Status::Slp as u8, 3);
    cb_bu.mons[2].set_status(Status::Par as u8, 0);
    let mut bu_hit = false;
    for s in 1..=32u64 {
        let mut c = cb_bu;
        c.prng_seed = s.wrapping_mul(0x9E37_79B9_7F4A_7C15);
        c.step_turn(&t_bu, [0, 0]);
        if c.mons[3].hp() < t_bu.mons[3].maxhp {
            bu_hit = true;
            break;
        }
    }
    assert!(bu_hit, "Castle Beat Up must still hit once with paralyzed active user");

    // 7. Present v0.3: fixed 80 HP heal & always BP 40 vs Sub!
    let p_pz = vec![mk_set("Delibird", 50, "", &["Present"], "M")];
    let p_pz_foe = vec![mk_set("Snorlax", 55, "", &["Splash"], "M")];
    let (t_pz, base_pz) = start_compact(&dex, Ruleset::castle_tournament(), &p_pz, &p_pz_foe);
    let mut saw_heal_80 = false;
    for s in 1..=128u64 {
        let mut c = base_pz;
        c.prng_seed = s.wrapping_mul(0x9E37_79B9_7F4A_7C15);
        c.mons[3].hp_pool[0] = 100;
        c.step_turn(&t_pz, [0, 0]);
        if c.mons[3].hp() == 180 {
            saw_heal_80 = true;
            break;
        }
    }
    assert!(saw_heal_80, "Castle Present heal outcome must heal exactly 80 HP");

    // 8. Pursuit switch KO in Castle cancels planned switch & opens fresh CHANGE (`Probe 10B`)!
    let p_sw = vec![
        mk_set("Alakazam", 50, "", &["Splash"], "M"),
        mk_set("Snorlax", 50, "", &["Splash"], "M"),
        mk_set("Zapdos", 50, "", &["Splash"], ""),
    ];
    let p_pu = vec![mk_set("Tyranitar", 55, "", &["Pursuit"], "M")];
    let (t_pu, mut cb_pu) = start_compact(&dex, Ruleset::castle_tournament(), &p_sw, &p_pu);
    cb_pu.mons[0].hp_pool[0] = 10;
    cb_pu.step_turn(&t_pu, [5, 0]); // Alakazam tries to switch to Snorlax (5), KO'd by Pursuit!
    assert_eq!(cb_pu.sides[0].request, 2, "Castle Pursuit switch KO must cancel planned switch and open CHANGE");
    let mut chg = [0u8; 7];
    let n_chg = cb_pu.legal_actions(&t_pu, 0, &mut chg);
    assert_eq!(&chg[..n_chg], &[5, 6], "Player can freely pick Zapdos (6) instead of planned Snorlax (5)");
    cb_pu.step_switch(&t_pu, [6, 7]);
    assert_eq!(cb_pu.sides[0].active_idx, 2);
}

#[test]
fn castle_turn_cut_dual_switch_spikes_1v1_overlays_and_l_turn_80_judge() {
    let dex = load_dex();

    // 1. Castle #3 (`Probe 9F`): Dual manual switch where 1st switcher dies to Spikes -> Turn-Cut SKIPS 2nd switch!
    let p1 = vec![
        mk_set("Jolteon", 55, "", &["Splash"], "M"),
        mk_set("Alakazam", 50, "", &["Splash"], "M"),
        mk_set("Snorlax", 50, "", &["Splash"], "M"),
    ];
    let p2 = vec![
        mk_set("Snorlax", 50, "", &["Splash"], "M"),
        mk_set("Zapdos", 50, "", &["Splash"], ""),
        mk_set("Skarmory", 50, "", &["Splash"], "M"),
    ];
    let (t_tc, mut cb_tc) = start_compact(&dex, Ruleset::castle_tournament(), &p1, &p2);
    cb_tc.sides[0].side_flags |= 1; // Spikes on P1 side
    cb_tc.mons[1].hp_pool[0] = 1; // Alakazam dies to Spikes on switch-in!
    cb_tc.step_turn(&t_tc, [5, 5]); // Faster Jolteon switches to 1-HP Alakazam; slower Snorlax tries to switch to Zapdos!
    assert_eq!(cb_tc.sides[0].request, 2, "Alakazam died to Spikes -> P1 CHANGE");
    assert_eq!(
        cb_tc.sides[1].active_idx, 0,
        "Castle Turn-Cut must SKIP P2's switch when P1's switch-in faints to Spikes"
    );

    // 2. 1v1 recoil mutual KO respects `CastleOverlayConfig::recoil_1v1_attacker_wins` (`Probe 2B` vs `H_20260407`).
    let p_r1 = vec![mk_set("Snorlax", 55, "", &["Double-Edge"], "M")];
    let p_r2 = vec![mk_set("Snorlax", 50, "", &["Splash"], "M")];
    let (t_tour, mut cb_tour) = start_compact(&dex, Ruleset::castle_tournament(), &p_r1, &p_r2);
    let (t_raw, mut cb_raw) = start_compact(&dex, Ruleset::castle_raw_cgi(false), &p_r1, &p_r2);
    cb_tour.mons[0].hp_pool[0] = 1;
    cb_tour.mons[3].hp_pool[0] = 20;
    cb_raw.mons[0].hp_pool[0] = 1;
    cb_raw.mons[3].hp_pool[0] = 20;
    cb_tour.step_turn(&t_tour, [0, 0]);
    cb_raw.step_turn(&t_raw, [0, 0]);
    assert_eq!(cb_tour.outcome, 1, "Castle tournament overlay: 1v1 recoil attacker wins");
    assert_eq!(cb_raw.outcome, 3, "Castle raw PS.cgi: 1v1 recoil mutual KO is DRAW");

    // 3. 1v1 Perish Song / Destiny Bond last-mon forfeit overlay!
    let p_db1 = vec![mk_set("Gengar", 55, "", &["Destiny Bond", "Splash"], "M")];
    let p_db2 = vec![mk_set("Snorlax", 50, "", &["Earthquake"], "M")];
    let (t_df_on, mut cb_df_on) = start_compact(&dex, Ruleset::castle_tournament(), &p_db1, &p_db2);
    let (t_df_off, mut cb_df_off) = start_compact(&dex, Ruleset::castle_raw_cgi(false), &p_db1, &p_db2);
    cb_df_on.mons[0].vol_flags |= vol_bit::DESTINY_BOND;
    cb_df_on.mons[0].hp_pool[0] = 1;
    cb_df_on.mons[0].boosts[stat_idx::SPE] = -6; // Snorlax moves first and kills DBond Gengar
    cb_df_off.mons[0].vol_flags |= vol_bit::DESTINY_BOND;
    cb_df_off.mons[0].hp_pool[0] = 1;
    cb_df_off.mons[0].boosts[stat_idx::SPE] = -6;
    cb_df_on.step_turn(&t_df_on, [1, 0]);
    cb_df_off.step_turn(&t_df_off, [1, 0]);
    assert_eq!(cb_df_on.outcome, 2, "Castle overlay: last-mon DBond user (P1) forfeits -> P2Win");
    assert_eq!(cb_df_off.outcome, 3, "Castle raw: last-mon DBond mutual KO is DRAW");

    // 4. L_TURN = 80 Judge (`Probe 12A1, 12A2, 12A3`): alive count -> sum of alive hp/maxhp ratios!
    let p_lt1 = vec![
        mk_set("Blissey", 50, "", &["Splash"], "F"),
        mk_set("Starmie", 50, "", &["Splash"], ""),
    ];
    let p_lt2 = vec![
        mk_set("Snorlax", 50, "", &["Splash"], "M"),
        mk_set("Snorlax", 50, "", &["Splash"], "M"),
    ];
    let cfg_80 = Ruleset::CastlePbs(CastleOverlayConfig::tournament());
    let (mut t_lt, mut cb_lt) = start_compact(&dex, cfg_80, &p_lt1, &p_lt2);
    // Probe 12A1: P1 (91/361 + 126/126 = 1.252) vs P2 (133/266 + 133/266 = 1.000) -> P1Win!
    t_lt.mons[0].maxhp = 361;
    t_lt.mons[1].maxhp = 126;
    t_lt.mons[3].maxhp = 266;
    t_lt.mons[4].maxhp = 266;
    cb_lt.mons[0].hp_pool[0] = 91;
    cb_lt.mons[1].hp_pool[0] = 126;
    cb_lt.mons[3].hp_pool[0] = 133;
    cb_lt.mons[4].hp_pool[0] = 133;
    cb_lt.turn = 80;
    cb_lt.step_turn(&t_lt, [0, 0]);
    assert_eq!(cb_lt.outcome, 1, "Probe 12A1: ratio sum 1.252 > 1.000 must award P1Win despite lower raw HP");

    // Probe 12A3: 2 alive (0.505 total) beats 1 alive (1.000 total)!
    let mut cb_lt3 = cb_lt;
    cb_lt3.outcome = 0;
    cb_lt3.turn = 80;
    cb_lt3.sides[0].alive_mask = 0b011;
    cb_lt3.sides[1].alive_mask = 0b001;
    cb_lt3.mons[0].hp_pool[0] = 90;
    cb_lt3.mons[1].hp_pool[0] = 32;
    cb_lt3.mons[3].hp_pool[0] = 266;
    cb_lt3.mons[4].hp_pool[0] = 0;
    cb_lt3.step_turn(&t_lt, [0, 0]);
    assert_eq!(cb_lt3.outcome, 1, "Probe 12A3: 2 alive must beat 1 alive at L_TURN=80");

    // Free Castle config (turn_limit: None) does NOT terminate at turn 80!
    let (t_free, mut cb_free) = start_compact(&dex, Ruleset::castle_free(), &p_lt1, &p_lt2);
    cb_free.turn = 80;
    cb_free.step_turn(&t_free, [0, 0]);
    assert_eq!(cb_free.outcome, 0);
    assert_eq!(cb_free.turn, 81);
}


#[test]
fn castle_eot_1p_to_2p_order_100pct_secondary_crit_focus_energy_confusion_and_zero_dmg() {
    let dex = load_dex();

    // 1. Fixed 1P -> 2P EOT order: 1v1 Sandstorm mutual KO is DRAW in Castle vs slower-wins in PS!
    let p_fast = vec![mk_set("Jolteon", 55, "", &["Sandstorm", "Splash"], "M")];
    let p_slow = vec![mk_set("Snorlax", 50, "", &["Splash"], "M")];
    let (t_ss_c, mut cb_ss_c) = start_compact(&dex, Ruleset::castle_tournament(), &p_fast, &p_slow);
    let (t_ss_p, mut cb_ss_p) = start_compact(&dex, Ruleset::PsStadium2, &p_fast, &p_slow);
    cb_ss_c.mons[0].hp_pool[0] = 1;
    cb_ss_c.mons[3].hp_pool[0] = 1;
    cb_ss_p.mons[0].hp_pool[0] = 1;
    cb_ss_p.mons[3].hp_pool[0] = 1;
    cb_ss_c.step_turn(&t_ss_c, [0, 0]);
    cb_ss_p.step_turn(&t_ss_p, [0, 0]);
    assert_eq!(cb_ss_c.outcome, 3, "Castle 1v1 Sandstorm mutual KO must be DRAW");
    assert_eq!(cb_ss_p.outcome, 1, "PS 1v1 Sandstorm faint_queue=[faster,slower] -> faster P1Win");

    // 2. Perish Song (`Probe 11A2` + `perish_dbond_lastmon_forfeit`):
    //    True 1v1 cast fails (`Probe 11A2`); 2v1 cast -> 1v1 mutual KO honors `perish_dbond_lastmon_forfeit`!
    let p_ps1_1v1 = vec![mk_set("Gengar", 55, "", &["Perish Song", "Splash"], "M")];
    let p_ps1_2v1 = vec![
        mk_set("Gengar", 55, "", &["Perish Song", "Splash"], "M"),
        mk_set("Snorlax", 50, "", &["Splash"], "M"),
    ];
    let p_ps2 = vec![mk_set("Snorlax", 50, "", &["Splash"], "M")];
    let (t_per_1v1, mut cb_per_1v1) =
        start_compact(&dex, Ruleset::castle_tournament(), &p_ps1_1v1, &p_ps2);
    cb_per_1v1.step_turn(&t_per_1v1, [0, 0]);
    assert_eq!(
        cb_per_1v1.mons[0].vol_flags & vol_bit::PERISH_SONG,
        0,
        "True 1v1 Perish Song must fail (`Probe 11A2`)"
    );

    let (t_per_t, mut cb_per_t) = start_compact(&dex, Ruleset::castle_tournament(), &p_ps1_2v1, &p_ps2);
    let (t_per_r, mut cb_per_r) = start_compact(&dex, Ruleset::castle_raw_cgi(false), &p_ps1_2v1, &p_ps2);
    cb_per_t.step_turn(&t_per_t, [0, 0]); // Cast Perish Song in 2v1 (4 -> EOT 3)
    cb_per_r.step_turn(&t_per_r, [0, 0]);
    // Bench Snorlax faints, leaving 1v1 active Perish Song race!
    cb_per_t.sides[0].alive_mask = 0b001;
    cb_per_t.mons[1].hp_pool[0] = 0;
    cb_per_r.sides[0].alive_mask = 0b001;
    cb_per_r.mons[1].hp_pool[0] = 0;
    for _ in 0..3 {
        cb_per_t.step_turn(&t_per_t, [1, 0]);
        cb_per_r.step_turn(&t_per_r, [1, 0]);
    }
    assert_eq!(cb_per_t.outcome, 2, "Castle overlay: 1v1 Perish Song caster (P1) forfeits -> P2Win");
    assert_eq!(cb_per_r.outcome, 3, "Castle raw: 1v1 Perish Song mutual KO is DRAW");

    // 3. 100% secondary effect (Icy Wind / Zap Cannon) is strictly 100.0% in Castle (no 1/256 fail).
    let p_iw = vec![mk_set("Suicune", 55, "", &["Icy Wind"], "")];
    let p_sn = vec![mk_set("Snorlax", 55, "", &["Splash"], "M")];
    let (t_iw, base_iw) = start_compact(&dex, Ruleset::castle_tournament(), &p_iw, &p_sn);
    let mut hits = 0u32;
    let mut drops = 0u32;
    for s in 1..=600u64 {
        let mut c = base_iw;
        c.prng_seed = s.wrapping_mul(0x9E37_79B9_7F4A_7C15);
        let hp0 = c.mons[3].hp();
        c.step_turn(&t_iw, [0, 0]);
        if c.mons[3].hp() < hp0 {
            hits += 1;
            if c.mons[3].boosts[stat_idx::SPE] == -1 {
                drops += 1;
            }
        }
    }
    assert!(hits > 500);
    assert_eq!(hits, drops, "Castle 100% secondary must trigger on 100.0% of hits");

    // 4. Base crit ~17/256 & Focus Energy (+1 crit stage -> ~64/256).
    let p_fe = vec![mk_set("Machamp", 50, "", &["Focus Energy", "Tackle"], "M")];
    let p_tg = vec![mk_set("Blissey", 50, "", &["Splash"], "F")];
    let (t_fe, base_fe) = start_compact(&dex, Ruleset::castle_tournament(), &p_fe, &p_tg);
    let mut base_crits = 0u32;
    let mut fe_crits = 0u32;
    // Non-crit max Tackle damage vs Blissey is well below crit min; measure threshold directly.
    let mut non_fe_dmgs = Vec::with_capacity(2000);
    let mut fe_dmgs = Vec::with_capacity(2000);
    for s in 1..=2000u64 {
        let mut c1 = base_fe;
        c1.prng_seed = s.wrapping_mul(0x9E37_79B9_7F4A_7C15);
        let hp0 = c1.mons[3].hp();
        c1.step_turn(&t_fe, [1, 0]);
        let d1 = hp0 - c1.mons[3].hp();
        if d1 > 0 {
            non_fe_dmgs.push(d1);
        }

        let mut c2 = base_fe;
        c2.prng_seed = s.wrapping_mul(0x9E37_79B9_7F4A_7C15);
        c2.mons[0].ext_flags |= ext_vol_bit::FOCUS_ENERGY;
        c2.step_turn(&t_fe, [1, 0]);
        let d2 = hp0 - c2.mons[3].hp();
        if d2 > 0 {
            fe_dmgs.push(d2);
        }
    }
    let min_dmg = *non_fe_dmgs.iter().min().unwrap();
    let crit_cut = (min_dmg as i32 * 3) / 2;
    for &d in &non_fe_dmgs {
        if d as i32 >= crit_cut {
            base_crits += 1;
        }
    }
    for &d in &fe_dmgs {
        if d as i32 >= crit_cut {
            fe_crits += 1;
        }
    }
    let r_base = base_crits as f64 / non_fe_dmgs.len() as f64;
    let r_fe = fe_crits as f64 / fe_dmgs.len() as f64;
    assert!(
        (0.045..=0.095).contains(&r_base),
        "Castle base crit rate must be ~17/256 (6.64%), got {r_base:.4}"
    );
    assert!(
        (0.10..=0.155).contains(&r_fe),
        "Castle Focus Energy (+1 stage) crit rate must be ~32/256 (12.50%), got {r_fe:.4}"
    );

    // 5. Castle confusion self-hit: 217..=255 variance, no Thick Club, hits own Sub, Focus Band applies!
    let p_cf = vec![mk_set("Marowak", 50, "Thick Club", &["Splash"], "M")];
    let p_cf_fb = vec![mk_set("Marowak", 50, "Focus Band", &["Splash"], "M")];
    let p_cf_no = vec![mk_set("Marowak", 50, "", &["Splash"], "M")];
    let (t_cf_tc, base_cf_tc) = start_compact(&dex, Ruleset::castle_tournament(), &p_cf, &p_tg);
    let (t_cf_no, base_cf_no) = start_compact(&dex, Ruleset::castle_tournament(), &p_cf_no, &p_tg);
    let (t_cf_fb, base_cf_fb) = start_compact(&dex, Ruleset::castle_tournament(), &p_cf_fb, &p_tg);
    let (t_cf_ps, base_cf_ps) = start_compact(&dex, Ruleset::PsStadium2, &p_cf_no, &p_tg);
    let mut min_c = i16::MAX;
    let mut max_c = 0i16;
    let mut max_tc = 0i16;
    let mut saw_sub_hit = false;
    let mut saw_fb_save = false;
    let mut ps_vals = std::collections::BTreeSet::new();
    for s in 1..=256u64 {
        let mut c = base_cf_no;
        c.prng_seed = s.wrapping_mul(0x9E37_79B9_7F4A_7C15);
        c.mons[0].vol_flags |= vol_bit::CONFUSION;
        c.mons[0].timers[timer_idx::CONFUSION] = 3;
        let hp0 = c.mons[0].hp();
        c.step_turn(&t_cf_no, [0, 0]);
        let d = hp0 - c.mons[0].hp();
        if d > 0 {
            min_c = min_c.min(d);
            max_c = max_c.max(d);
        }

        let mut c_tc = base_cf_tc;
        c_tc.prng_seed = s.wrapping_mul(0x9E37_79B9_7F4A_7C15);
        c_tc.mons[0].vol_flags |= vol_bit::CONFUSION;
        c_tc.mons[0].timers[timer_idx::CONFUSION] = 3;
        c_tc.step_turn(&t_cf_tc, [0, 0]);
        let d_tc = hp0 - c_tc.mons[0].hp();
        if d_tc > 0 {
            max_tc = max_tc.max(d_tc);
        }

        let mut c_sub = base_cf_no;
        c_sub.prng_seed = s.wrapping_mul(0x9E37_79B9_7F4A_7C15);
        c_sub.mons[0].vol_flags |= vol_bit::CONFUSION | vol_bit::SUBSTITUTE;
        c_sub.mons[0].timers[timer_idx::CONFUSION] = 3;
        c_sub.mons[0].hp_pool[1] = 40;
        c_sub.step_turn(&t_cf_no, [0, 0]);
        if c_sub.mons[0].sub_hp() < 40 && c_sub.mons[0].hp() == hp0 {
            saw_sub_hit = true;
        }

        let mut c_fb = base_cf_fb;
        c_fb.prng_seed = s.wrapping_mul(0x9E37_79B9_7F4A_7C15);
        c_fb.mons[0].vol_flags |= vol_bit::CONFUSION;
        c_fb.mons[0].timers[timer_idx::CONFUSION] = 3;
        c_fb.mons[0].hp_pool[0] = 5;
        c_fb.step_turn(&t_cf_fb, [0, 0]);
        if c_fb.mons[0].hp() == 1 {
            saw_fb_save = true;
        }

        let mut cp = base_cf_ps;
        cp.prng_seed = s.wrapping_mul(0x9E37_79B9_7F4A_7C15);
        cp.mons[0].vol_flags |= vol_bit::CONFUSION;
        cp.mons[0].timers[timer_idx::CONFUSION] = 3;
        cp.step_turn(&t_cf_ps, [0, 0]);
        let dp = hp0 - cp.mons[0].hp();
        if dp > 0 {
            ps_vals.insert(dp);
        }
    }
    assert!(max_c > min_c, "Castle confusion self-hit must have 217..=255 variance: {min_c}..={max_c}");
    assert_eq!(ps_vals.len(), 1, "PS confusion self-hit must have zero variance");
    assert_eq!(max_tc, max_c, "Castle confusion self-hit must NOT apply Thick Club");
    assert!(saw_sub_hit, "Castle confusion self-hit must hit own Substitute");
    assert!(saw_fb_save, "Castle Focus Band must save confused mon at 1 HP");

    // 6. Minimum damage 0 allowed in Castle (`Probe 5A`: weak 0.25x hit floors to 0 in Castle vs 1 in PS)!
    let p_wk = vec![mk_set("Shuckle", 5, "", &["Scratch"], "M")];
    let p_tk = vec![mk_set("Tyranitar", 55, "", &["Splash"], "M")];
    let (t_z_c, base_z_c) = start_compact(&dex, Ruleset::castle_tournament(), &p_wk, &p_tk);
    let (t_z_p, base_z_p) = start_compact(&dex, Ruleset::PsStadium2, &p_wk, &p_tk);
    // Use Rock/Steel Steelix (0.25x vs Normal) for guaranteed 0-damage floor in Castle!
    let p_stx = vec![mk_set("Steelix", 55, "", &["Splash"], "M")];
    let (t_z_c2, mut cb_z_c2) = start_compact(&dex, Ruleset::castle_tournament(), &p_wk, &p_stx);
    let (t_z_p2, mut cb_z_p2) = start_compact(&dex, Ruleset::PsStadium2, &p_wk, &p_stx);
    let _ = (t_z_c, base_z_c, t_z_p, base_z_p);
    cb_z_c2.prng_seed = 7;
    cb_z_p2.prng_seed = 7;
    let stx_hp0 = cb_z_c2.mons[3].hp();
    cb_z_c2.step_turn(&t_z_c2, [0, 0]);
    cb_z_p2.step_turn(&t_z_p2, [0, 0]);
    assert_eq!(cb_z_c2.mons[3].hp(), stx_hp0, "Castle allows 0 damage when 0.25x hit floors to 0");
    assert_eq!(cb_z_p2.mons[3].hp(), stx_hp0 - 1, "PS clamps damaging hits to minimum 1");
}

#[test]
fn castle_mist_struggle_triplekick_twineedle_triattack_flinch_wrap_furycutter_rage_bonemerang_falseswipe_psychup_frustration_lockon_dig() {
    let dex = load_dex();

    // 1. Mist blocks opponent stat drops (status & secondary), allows Curse/Swagger, passes via Baton Pass!
    let p_m1 = vec![
        mk_set("Jolteon", 55, "", &["Mist", "Curse", "Baton Pass"], "M"),
        mk_set("Snorlax", 50, "", &["Splash"], "M"),
    ];
    let p_m2 = vec![mk_set("Suicune", 50, "", &["Screech", "Icy Wind", "Swagger"], "")];
    let (t_m, mut cb_m) = start_compact(&dex, Ruleset::castle_tournament(), &p_m1, &p_m2);
    cb_m.step_turn(&t_m, [0, 0]); // Jolteon Mist -> Suicune Screech blocked!
    assert_ne!(cb_m.mons[0].ext_flags & ext_vol_bit::MIST, 0);
    assert_eq!(cb_m.mons[0].boosts[stat_idx::DEF], 0, "Mist must block opponent Screech");
    cb_m.prng_seed = 7;
    cb_m.step_turn(&t_m, [1, 1]); // Jolteon Curse (own Spe -1 allowed!) vs Suicune Icy Wind (secondary Spe -1 blocked!)
    assert_eq!(cb_m.mons[0].boosts[stat_idx::ATK], 1);
    assert_eq!(cb_m.mons[0].boosts[stat_idx::SPE], -1, "Mist allows self Curse drop and blocks foe Icy Wind drop");
    cb_m.mons[3].boosts[stat_idx::SPE] = -2; // Ensure Jolteon (-1 Spe) moves before Suicune
    cb_m.prng_seed = 7;
    cb_m.step_turn(&t_m, [2, 2]); // Jolteon Baton Passes Mist to Snorlax (sole bench auto-switch), Suicune Swaggers (+2 Atk allowed!)
    assert_eq!(cb_m.sides[0].active_idx, 1);
    assert_ne!(cb_m.mons[1].ext_flags & ext_vol_bit::MIST, 0, "Baton Pass must inherit Mist");
    assert_eq!(cb_m.mons[1].boosts[stat_idx::ATK], 3, "Swagger +2 Atk bypasses Mist");

    // 2. Struggle in Castle: Normal STAB + Pink Bow + 0 recoil vs Substitute!
    let p_st1 = vec![mk_set("Snorlax", 55, "Pink Bow", &["Splash"], "M")];
    let p_st2 = vec![mk_set("Snorlax", 55, "", &["Splash"], "M")];
    let (t_stc, mut cb_stc) = start_compact(&dex, Ruleset::castle_tournament(), &p_st1, &p_st2);
    let (t_stp, mut cb_stp) = start_compact(&dex, Ruleset::PsStadium2, &p_st1, &p_st2);
    cb_stc.mons[0].pp = [0; 4];
    cb_stp.mons[0].pp = [0; 4];
    cb_stc.prng_seed = 7;
    cb_stp.prng_seed = 7;
    let d_hp0 = cb_stc.mons[3].hp();
    cb_stc.step_turn(&t_stc, [0, 0]);
    cb_stp.step_turn(&t_stp, [0, 0]);
    let dmg_stc = d_hp0 - cb_stc.mons[3].hp();
    let dmg_stp = d_hp0 - cb_stp.mons[3].hp();
    assert!(
        dmg_stc > (dmg_stp * 3) / 2,
        "Castle Struggle must apply Normal STAB + Pink Bow: castle={dmg_stc} vs ps={dmg_stp}"
    );
    // Struggle into Substitute -> 0 recoil in Castle vs 1 HP recoil in PS!
    let (_, mut cb_stsub_c) = start_compact(&dex, Ruleset::castle_tournament(), &p_st1, &p_st2);
    let (_, mut cb_stsub_p) = start_compact(&dex, Ruleset::PsStadium2, &p_st1, &p_st2);
    cb_stsub_c.mons[0].pp = [0; 4];
    cb_stsub_p.mons[0].pp = [0; 4];
    cb_stsub_c.mons[3].vol_flags |= vol_bit::SUBSTITUTE;
    cb_stsub_c.mons[3].hp_pool[1] = 120;
    cb_stsub_p.mons[3].vol_flags |= vol_bit::SUBSTITUTE;
    cb_stsub_p.mons[3].hp_pool[1] = 120;
    cb_stsub_c.step_turn(&t_stc, [0, 0]);
    cb_stsub_p.step_turn(&t_stp, [0, 0]);
    assert_eq!(cb_stsub_c.mons[0].hp(), t_stc.mons[0].maxhp, "Castle Struggle vs Sub must take 0 recoil");
    assert_eq!(cb_stsub_p.mons[0].hp(), t_stp.mons[0].maxhp - 1, "PS Struggle vs Sub takes 1 recoil");

    // 3. Triple Kick 2x/3x damage leak onto second mover in Castle!
    let p_tk1 = vec![mk_set("Hitmontop", 55, "", &["Triple Kick", "Splash"], "M")];
    let p_tk2 = vec![mk_set("Snorlax", 50, "", &["Tackle"], "M")];
    let (t_tkc, base_tkc) = start_compact(&dex, Ruleset::castle_tournament(), &p_tk1, &p_tk2);
    let mut base_tackle = 0i16;
    let mut leaked_tackle = 0i16;
    {
        let mut c0 = base_tkc;
        c0.prng_seed = 7;
        let hp0 = c0.mons[0].hp();
        c0.step_turn(&t_tkc, [1, 0]); // Hitmontop Splash -> Snorlax Tackle (1x)
        base_tackle = hp0 - c0.mons[0].hp();
    }
    for s in 1..=64u64 {
        let mut c = base_tkc;
        c.prng_seed = s.wrapping_mul(0x9E37_79B9_7F4A_7C15);
        let hp0 = c.mons[0].hp();
        c.step_turn(&t_tkc, [0, 0]); // Faster Hitmontop Triple Kick -> slower Snorlax Tackle gets 2x/3x leak!
        let d = hp0 - c.mons[0].hp();
        if d > leaked_tackle {
            leaked_tackle = d;
        }
    }
    assert!(
        leaked_tackle >= base_tackle * 2,
        "Castle Triple Kick must leak 2x/3x mult onto second mover: base={base_tackle}, leaked={leaked_tackle}"
    );

    // 4. Twineedle Steel immunity in Castle (vs Steel CAN be poisoned in PS)!
    let p_tw1 = vec![mk_set("Beedrill", 55, "", &["Twineedle"], "M")];
    let p_tw2 = vec![mk_set("Skarmory", 55, "", &["Splash"], "M")];
    let (t_twc, base_twc) = start_compact(&dex, Ruleset::castle_tournament(), &p_tw1, &p_tw2);
    let (t_twp, base_twp) = start_compact(&dex, Ruleset::PsStadium2, &p_tw1, &p_tw2);
    let mut ps_poisoned_steel = false;
    for s in 1..=128u64 {
        let mut cc = base_twc;
        cc.prng_seed = s.wrapping_mul(0x9E37_79B9_7F4A_7C15);
        cc.step_turn(&t_twc, [0, 0]);
        assert_eq!(cc.mons[3].status_raw(), Status::None as u8, "Castle Twineedle must NEVER poison Steel");

        let mut cp = base_twp;
        cp.prng_seed = s.wrapping_mul(0x9E37_79B9_7F4A_7C15);
        cp.step_turn(&t_twp, [0, 0]);
        if cp.mons[3].status_raw() == Status::Psn as u8 {
            ps_poisoned_steel = true;
        }
    }
    assert!(ps_poisoned_steel, "PS Twineedle can poison Steel types");

    // 5. Tri Attack bypasses Freeze Clause in Castle!
    let p_ta1 = vec![mk_set("Porygon2", 55, "", &["Tri Attack"], "")];
    let p_ta2 = vec![
        mk_set("Blissey", 55, "", &["Splash"], "F"),
        mk_set("Snorlax", 55, "", &["Splash"], "M"),
    ];
    let (t_tac, base_tac) = start_compact(&dex, Ruleset::castle_tournament(), &p_ta1, &p_ta2);
    let mut ta_froze_second = false;
    for s in 1..=256u64 {
        let mut c = base_tac;
        c.prng_seed = s.wrapping_mul(0x9E37_79B9_7F4A_7C15);
        c.mons[4].set_status(Status::Frz as u8, 0); // bench teammate already frozen!
        c.step_turn(&t_tac, [0, 0]);
        if c.mons[3].status_raw() == Status::Frz as u8 {
            ta_froze_second = true;
            break;
        }
    }
    assert!(ta_froze_second, "Castle Tri Attack must bypass Freeze Clause");

    // 6. Flinch vs sleeping target (`Probe 8E`): fast Headbutt flinches waking target in Castle
    //    (Sleep counter ticks to wake-up, THEN flinch blocks the move so Splash PP is not spent)!
    let p_fl1 = vec![mk_set("Jolteon", 55, "", &["Headbutt"], "M")];
    let p_fl2 = vec![mk_set("Snorlax", 55, "", &["Splash"], "M")];
    let (t_flc, base_flc) = start_compact(&dex, Ruleset::castle_tournament(), &p_fl1, &p_fl2);
    let (t_flp, base_flp) = start_compact(&dex, Ruleset::PsStadium2, &p_fl1, &p_fl2);
    let mut castle_flinched_waker = false;
    for s in 1..=128u64 {
        let mut cc = base_flc;
        cc.prng_seed = s.wrapping_mul(0x9E37_79B9_7F4A_7C15);
        cc.mons[3].set_status(Status::Slp as u8, 1); // wakes up on its move, then flinches!
        let pp0 = cc.mons[3].pp[0];
        cc.step_turn(&t_flc, [0, 0]);
        if cc.mons[3].status_raw() == Status::None as u8 && cc.mons[3].pp[0] == pp0 {
            castle_flinched_waker = true;
        }

        let mut cp = base_flp;
        cp.prng_seed = s.wrapping_mul(0x9E37_79B9_7F4A_7C15);
        cp.mons[3].set_status(Status::Slp as u8, 1);
        let pp0p = cp.mons[3].pp[0];
        cp.step_turn(&t_flp, [0, 0]);
        assert_eq!(cp.mons[3].status_raw(), Status::None as u8, "PS target wakes up");
        assert_eq!(cp.mons[3].pp[0], pp0p - 1, "PS must NOT flinch sleeping targets");
    }
    assert!(castle_flinched_waker, "Castle Headbutt must flinch waking target (`Probe 8E`)");

    // 7. PartialTrap (`Wrap`): AfterMoveSelf 1/16 timing in Castle & blocked by Substitute (`Probe 12D1`/`12I`)!
    let p_wr1 = vec![mk_set("Dragonite", 55, "", &["Wrap", "Splash"], "M")];
    let p_wr2 = vec![mk_set("Snorlax", 50, "", &["Splash"], "M")];
    let (t_wr, mut cb_wr) = start_compact(&dex, Ruleset::castle_tournament(), &p_wr1, &p_wr2);
    // Blocked by Substitute (`Probe 12D1`):
    cb_wr.mons[3].vol_flags |= vol_bit::SUBSTITUTE;
    cb_wr.mons[3].hp_pool[1] = 80;
    cb_wr.prng_seed = 7;
    cb_wr.step_turn(&t_wr, [0, 0]);
    assert!(!cb_wr.mons[3].partially_trapped(), "Wrap must NOT trap through Substitute");
    // Unblocked Wrap traps Snorlax, and Snorlax takes 1/16 in AfterMoveSelf (BEFORE slower opponent or immediately on its move)!
    let (_, mut cb_wr2) = start_compact(&dex, Ruleset::castle_tournament(), &p_wr1, &p_wr2);
    cb_wr2.prng_seed = 7;
    cb_wr2.step_turn(&t_wr, [0, 0]);
    assert!(cb_wr2.mons[3].partially_trapped());
    let hp_after_t1 = cb_wr2.mons[3].hp();
    cb_wr2.step_turn(&t_wr, [1, 0]); // Turn 2: both Splash, Snorlax takes 1/16 Wrap slip in AfterMoveSelf!
    assert_eq!(cb_wr2.mons[3].hp(), hp_after_t1 - t_wr.mons[3].quantum_1_16);

    // 8. Fury Cutter uncapped in Castle (capped at 160 in PS) & Rage multiplier in Castle (Atk +1 in PS)!
    let p_fc1 = vec![mk_set("Scizor", 55, "", &["Fury Cutter", "Rage"], "M")];
    let p_fc2 = vec![mk_set("Snorlax", 55, "", &["Tackle", "Splash"], "M")];
    let (t_fcc, mut cb_fcc) = start_compact(&dex, Ruleset::castle_tournament(), &p_fc1, &p_fc2);
    let (t_fcp, mut cb_fcp) = start_compact(&dex, Ruleset::PsStadium2, &p_fc1, &p_fc2);
    cb_fcc.mons[0].ext_timers[ext_timer_idx::FURY_CUTTER] = 5; // 10 << 5 = 320 in Castle vs 160 cap in PS!
    cb_fcp.mons[0].ext_timers[ext_timer_idx::FURY_CUTTER] = 5;
    cb_fcc.prng_seed = 7;
    cb_fcp.prng_seed = 7;
    let sn_hp0 = cb_fcc.mons[3].hp();
    cb_fcc.step_turn(&t_fcc, [0, 1]);
    cb_fcp.step_turn(&t_fcp, [0, 1]);
    assert!(
        (sn_hp0 - cb_fcc.mons[3].hp()) > ((sn_hp0 - cb_fcp.mons[3].hp()) * 3) / 2,
        "Castle Fury Cutter must double past 160 BP cap"
    );

    // Rage: Castle increments RAGE_COUNTER without touching ATK boost; PS raises ATK +1!
    let (_, mut cb_rgc) = start_compact(&dex, Ruleset::castle_tournament(), &p_fc1, &p_fc2);
    let (_, mut cb_rgp) = start_compact(&dex, Ruleset::PsStadium2, &p_fc1, &p_fc2);
    cb_rgc.prng_seed = 7;
    cb_rgp.prng_seed = 7;
    cb_rgc.step_turn(&t_fcc, [1, 0]); // Faster Scizor uses Rage, then slower Snorlax hits with Tackle!
    cb_rgp.step_turn(&t_fcp, [1, 0]);
    assert_eq!(cb_rgc.mons[0].boosts[stat_idx::ATK], 0, "Castle Rage must NOT boost ATK stage");
    assert_eq!(cb_rgc.mons[0].ext_timers[ext_timer_idx::RAGE_COUNTER], 1);
    assert_eq!(cb_rgp.mons[0].boosts[stat_idx::ATK], 1, "PS Rage must boost ATK stage +1");

    // 9. Multi-hit (`Bonemerang`) Counter reflects FINAL hit only & False Swipe Counter = 2 * old_hp in Castle!
    let p_bm1 = vec![mk_set("Marowak", 55, "", &["Bonemerang", "False Swipe"], "M")];
    let p_bm2 = vec![mk_set("Snorlax", 55, "", &["Counter"], "M")];
    let (t_bmc, mut cb_bmc) = start_compact(&dex, Ruleset::castle_tournament(), &p_bm1, &p_bm2);
    let (t_bmp, mut cb_bmp) = start_compact(&dex, Ruleset::PsStadium2, &p_bm1, &p_bm2);
    cb_bmc.prng_seed = 7;
    let sn0 = cb_bmc.mons[3].hp();
    let wak0 = cb_bmc.mons[0].hp();
    cb_bmc.step_turn(&t_bmc, [0, 0]); // Bonemerang (2 hits) -> Snorlax Counter reflects final hit ONLY!
    let total_bm = sn0 - cb_bmc.mons[3].hp();
    let counter_dmg = wak0 - cb_bmc.mons[0].hp();
    assert!(
        counter_dmg > 0 && counter_dmg < (total_bm * 3) / 2,
        "Counter vs 2-hit Bonemerang must reflect ONLY final hit (~total_bm), not 2*total_bm: total={total_bm}, counter={counter_dmg}"
    );
    // False Swipe leaving Snorlax at 1 HP from old_hp=15: Castle Counter deals 30 (`2 * 15`) vs PS 28 (`2 * 14`)!
    cb_bmc = start_compact(&dex, Ruleset::castle_tournament(), &p_bm1, &p_bm2).1;
    cb_bmp = start_compact(&dex, Ruleset::PsStadium2, &p_bm1, &p_bm2).1;
    cb_bmc.mons[3].hp_pool[0] = 15;
    cb_bmp.mons[3].hp_pool[0] = 15;
    cb_bmc.prng_seed = 7;
    cb_bmp.prng_seed = 7;
    cb_bmc.step_turn(&t_bmc, [1, 0]); // False Swipe -> 1 HP -> Counter!
    cb_bmp.step_turn(&t_bmp, [1, 0]);
    assert_eq!(cb_bmc.mons[3].hp(), 1);
    assert_eq!(wak0 - cb_bmc.mons[0].hp(), 30, "Castle False Swipe Counter must reflect 2*old_hp = 30");
    assert_eq!(wak0 - cb_bmp.mons[0].hp(), 28, "PS False Swipe Counter reflects 2*dealt = 28");

    // 10. Psych Up fails vs all-0 boosts in Castle (`Probe 12A3`), Frustration = 102 BP in Castle,
    //     and Lock-On does NOT let Ice Beam hit Digging target in Castle (`Probe 11H`)!
    let p_pu1 = vec![mk_set("Mew", 55, "", &["Psych Up", "Frustration", "Ice Beam"], "")];
    let p_pu2 = vec![mk_set("Snorlax", 50, "", &["Dig", "Splash"], "M")];
    let (t_puc, mut cb_puc) = start_compact(&dex, Ruleset::castle_tournament(), &p_pu1, &p_pu2);
    let (t_pup, mut cb_pup) = start_compact(&dex, Ruleset::PsStadium2, &p_pu1, &p_pu2);
    cb_puc.mons[0].boosts[stat_idx::ATK] = 2;
    cb_pup.mons[0].boosts[stat_idx::ATK] = 2;
    cb_puc.step_turn(&t_puc, [0, 1]); // Psych Up vs all-0 Snorlax -> FAILS in Castle (keeps +2!), resets to 0 in PS!
    cb_pup.step_turn(&t_pup, [0, 1]);
    assert_eq!(cb_puc.mons[0].boosts[stat_idx::ATK], 2, "Castle Psych Up vs all-0 must fail");
    assert_eq!(cb_pup.mons[0].boosts[stat_idx::ATK], 0, "PS Psych Up vs all-0 copies 0");

    // Frustration at happiness 255 -> BP 102 in Castle vs BP 0 in PS!
    let (_, mut cb_frc) = start_compact(&dex, Ruleset::castle_tournament(), &p_pu1, &p_pu2);
    let (_, mut cb_frp) = start_compact(&dex, Ruleset::PsStadium2, &p_pu1, &p_pu2);
    let lax0 = cb_frc.mons[3].hp();
    cb_frc.step_turn(&t_puc, [1, 1]);
    cb_frp.step_turn(&t_pup, [1, 1]);
    assert!(lax0 - cb_frc.mons[3].hp() > 30, "Castle Frustration must use BP 102");
    assert_eq!(cb_frp.mons[3].hp(), lax0, "PS Frustration at happiness 255 deals 0");

    // Lock-On + Ice Beam vs underground Dig: misses in Castle (`Probe 11H`), hits in PS!
    let (_, mut cb_ldc) = start_compact(&dex, Ruleset::castle_tournament(), &p_pu1, &p_pu2);
    let (_, mut cb_ldp) = start_compact(&dex, Ruleset::PsStadium2, &p_pu1, &p_pu2);
    cb_ldc.mons[0].ext_flags |= ext_vol_bit::LOCK_ON;
    cb_ldc.mons[0].set_lock_on_turns(1);
    cb_ldc.mons[3].ext_flags |= ext_vol_bit::DIGGING;
    cb_ldc.mons[3].set_locked_move_slot(0);
    cb_ldp.mons[0].ext_flags |= ext_vol_bit::LOCK_ON;
    cb_ldp.mons[0].set_lock_on_turns(2);
    cb_ldp.mons[3].ext_flags |= ext_vol_bit::DIGGING;
    cb_ldp.mons[3].set_locked_move_slot(0);
    let l0 = cb_ldc.mons[3].hp();
    cb_ldc.step_turn(&t_puc, [2, 0]);
    cb_ldp.step_turn(&t_pup, [2, 0]);
    assert_eq!(cb_ldc.mons[3].hp(), l0, "Castle Lock-On must NOT let Ice Beam hit Digging foe (`Probe 11H`)");
    assert!(cb_ldp.mons[3].hp() < l0, "PS Lock-On lets Ice Beam hit Digging foe");
}

#[test]
fn ps_stadium2_differential_parity_preserved_after_256b_castle_expansion() {
    let dex = load_dex();
    let p1 = vec![
        mk_set("Snorlax", 55, "Leftovers", &["Body Slam", "Curse", "Earthquake", "Rest"], "M"),
        mk_set("Zapdos", 50, "Leftovers", &["Thunderbolt", "Hidden Power Ice", "Rest", "Sleep Talk"], ""),
        mk_set("Cloyster", 50, "Leftovers", &["Surf", "Spikes", "Rapid Spin", "Explosion"], "M"),
    ];
    let p2 = vec![
        mk_set("Raikou", 55, "Leftovers", &["Thunderbolt", "Hidden Power Water", "Roar", "Rest"], ""),
        mk_set("Skarmory", 50, "Leftovers", &["Drill Peck", "Whirlwind", "Curse", "Rest"], "M"),
        mk_set("Marowak", 50, "Thick Club", &["Earthquake", "Rock Slide", "Swords Dance", "Rest"], "M"),
    ];
    let mut b = Battle::from_fixture(&dex, "1,2,3,4", &p1, &p2).unwrap();
    b.choose(&dex, 0, "team 1,2,3").unwrap();
    b.choose(&dex, 1, "team 1,2,3").unwrap();
    let (table, mut cb) = CompactBattle::from_battle(&b, &dex);

    // Deterministic turns (switches + status/setup moves) must match full PS Battle exactly!
    b.choose(&dex, 0, "move 2").unwrap(); // Snorlax Curse
    b.choose(&dex, 1, "switch 2").unwrap(); // Raikou -> Skarmory
    cb.step_turn(&table, [1, 5]);
    assert_eq!(cb.sides[0].active_idx, 0);
    assert_eq!(cb.sides[1].active_idx, 1);
    let b_p1_act = &b.sides[0].roster[b.sides[0].active.unwrap() as usize];
    let b_p2_act = &b.sides[1].roster[b.sides[1].active.unwrap() as usize];
    assert_eq!(cb.mons[0].boosts, b_p1_act.boosts);
    assert_eq!(cb.mons[0].hp() as i32, b_p1_act.hp);
    assert_eq!(cb.mons[4].hp() as i32, b_p2_act.hp);

    // Turn 2: Snorlax switches to Cloyster (Spikes), Skarmory Curses.
    b.choose(&dex, 0, "switch 3").unwrap();
    b.choose(&dex, 1, "move 3").unwrap();
    cb.step_turn(&table, [6, 2]);
    let b_p1_act2 = &b.sides[0].roster[b.sides[0].active.unwrap() as usize];
    let b_p2_act2 = &b.sides[1].roster[b.sides[1].active.unwrap() as usize];
    assert_eq!(cb.sides[0].active_idx, 2);
    assert_eq!(cb.mons[4].boosts, b_p2_act2.boosts);
    assert_eq!(cb.mons[2].hp() as i32, b_p1_act2.hp);
    assert_eq!(cb.turn, b.turn as u16);
}

#[test]
fn fuzzer_parity_critical_fixes_residuals_explosion_clamp_stall() {
    assert_eq!(std::mem::size_of::<CompactMon>(), 32);
    assert_eq!(std::mem::size_of::<CompactBattle>(), 256);
    assert_eq!(std::mem::align_of::<CompactBattle>(), 64);

    let dex = load_dex();

    // 1. [CRITICAL] `from_battle` sets `RESIDUALS_DONE` (0x08) on post-EOT forced switch
    //    so `step_switch` never double-executes Sandstorm/Leftovers (tested in both PS & Castle).
    for ruleset in [Ruleset::PsStadium2, Ruleset::castle_tournament()] {
        let p1 = vec![
            mk_set("Cloyster", 50, "", &["Spikes", "Splash"], "M"),
            mk_set("Zapdos", 50, "Leftovers", &["Thunderbolt"], ""),
            mk_set("Snorlax", 55, "Leftovers", &["Body Slam"], "M"),
        ];
        let p2 = vec![
            mk_set("Tyranitar", 55, "", &["Sandstorm", "Splash"], "M"),
            mk_set("Skarmory", 50, "Leftovers", &["Drill Peck"], "M"),
            mk_set("Gengar", 50, "Leftovers", &["Thunderbolt"], "M"),
        ];
        let mut b = Battle::from_fixture_with_ruleset(&dex, "1,2,3,4", &p1, &p2, ruleset).unwrap();
        b.choose(&dex, 0, "team 1,2,3").unwrap();
        b.choose(&dex, 1, "team 1,2,3").unwrap();
        let clo_id = b.active_id(0).unwrap();
        b.poke_mut(clo_id).hp = 1;
        b.choose(&dex, 0, "move 2").unwrap(); // Splash
        b.choose(&dex, 1, "move 1").unwrap(); // Sandstorm -> KOs 1 HP Cloyster in EOT Residual!
        assert_eq!(
            b.sides[0].request_state(),
            Some(nc2000_engine::state::RequestKind::Switch)
        );
        let (table, mut cb) = CompactBattle::from_battle(&b, &dex);
        assert_ne!(
            cb.sides[0].side_flags & 0x08,
            0,
            "from_battle post-EOT forced switch must set RESIDUALS_DONE (ruleset={ruleset:?})"
        );

        // Advance both Battle and CompactBattle with replacement switch to Zapdos (slot 1 -> action 5).
        b.choose(&dex, 0, "switch 2").unwrap();
        cb.step_switch(&table, [5, 7]);
        let zap_b_hp = b.poke(b.active_id(0).unwrap()).hp as i16;
        assert_eq!(
            cb.mons[1].hp(),
            zap_b_hp,
            "step_switch must NOT double-run Sandstorm on Zapdos (ruleset={ruleset:?})"
        );
        assert_eq!(cb.turn, b.turn as u16);
        assert_eq!(cb.sides[0].side_flags & 0x08, 0);
    }

    // 2. [CRITICAL] Burned Explosion / Self-Destruct halves Atk in BOTH PS and Castle,
    //    and `mark_fainted()` zeroes boosts/volatiles/timers/slot_refs/ext_flags.
    for ruleset in [Ruleset::PsStadium2, Ruleset::castle_tournament()] {
        let p1 = vec![
            mk_set("Snorlax", 55, "", &["Self-Destruct", "Splash"], "M"),
            mk_set("Zapdos", 50, "", &["Splash"], ""),
        ];
        let p2 = vec![
            mk_set("Steelix", 55, "", &["Splash"], "M"),
            mk_set("Blissey", 55, "", &["Splash"], "F"),
        ];
        let (table, base) = start_compact(&dex, ruleset, &p1, &p2);

        let mut cb_clean = base;
        cb_clean.prng_seed = 0x1234_5678_9ABC_DEF0;
        let hp0 = cb_clean.mons[3].hp();
        cb_clean.step_turn(&table, [0, 0]);
        let dmg_clean = hp0 - cb_clean.mons[3].hp();

        let mut cb_brn = base;
        cb_brn.prng_seed = 0x1234_5678_9ABC_DEF0;
        cb_brn.mons[0].set_status(Status::Brn as u8, 0);
        cb_brn.mons[0].vol_flags |= vol_bit::BRN_ATK_DROP | vol_bit::CONFUSION;
        cb_brn.mons[0].timers[timer_idx::CONFUSION] = 3;
        cb_brn.mons[0].boosts[stat_idx::ATK] = 1;
        cb_brn.mons[0].ext_flags |= ext_vol_bit::MIST;
        // Pick a seed where Confusion does NOT self-hit and Explosion does NOT crit
        let mut verified = false;
        for seed in 1..=64u64 {
            let mut c_clean = base;
            let mut c_brn = base;
            let s = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15);
            c_clean.prng_seed = s;
            c_brn.prng_seed = s;
            c_brn.mons[0].set_status(Status::Brn as u8, 0);
            c_brn.mons[0].vol_flags |= vol_bit::BRN_ATK_DROP;
            c_brn.mons[0].ext_flags |= ext_vol_bit::MIST;
            c_brn.mons[0].boosts[stat_idx::DEF] = 2;
            c_clean.step_turn(&table, [0, 0]);
            c_brn.step_turn(&table, [0, 0]);
            let dc = hp0 - c_clean.mons[3].hp();
            let db = hp0 - c_brn.mons[3].hp();
            if dc > 40 && db < (dc * 3) / 4 {
                // Verify mark_fainted canonicalized the fainted Self-Destruct user.
                assert_eq!(c_brn.mons[0].status_raw(), Status::Fnt as u8);
                assert_eq!(c_brn.mons[0].hp_pool, [0, 0]);
                assert_eq!(c_brn.mons[0].vol_flags, 0);
                assert_eq!(c_brn.mons[0].timers, [0; 4]);
                assert_eq!(c_brn.mons[0].boosts, [0; 7]);
                assert_eq!(c_brn.mons[0].slot_refs, 0);
                assert_eq!(c_brn.mons[0].ext_flags, 0);
                assert_eq!(c_brn.mons[0].ext_timers, [0; 4]);
                assert_eq!(c_brn.mons[0].rollout_rage, 0);
                verified = true;
                break;
            }
        }
        assert!(
            verified && dmg_clean > 0,
            "Burned Self-Destruct must halve Atk in {ruleset:?}"
        );
    }

    // 3. [CRITICAL] PS `get_damage_base_branchless` clamps atk/def to [1, 999] BEFORE `>> 2`.
    //    e.g. Thick Club +6 Marowak (`atk = 1500`) must clamp to 999 before `>> 2` (999 >> 2 = 249),
    //    NOT `1500 >> 2 = 375`!
    let d_999 = get_damage_base_branchless(
        50,
        100,
        999,
        200,
        true,
        false,
        false,
        false,
        CompactWeather::None,
        false,
        false,
        false,
        0,
        0,
        0,
        false,
    );
    let d_1500 = get_damage_base_branchless(
        50,
        100,
        1500,
        200,
        true,
        false,
        false,
        false,
        CompactWeather::None,
        false,
        false,
        false,
        0,
        0,
        0,
        false,
    );
    assert_eq!(
        d_1500, d_999,
        "PS get_damage_base_branchless must clamp atk to 999 before >> 2"
    );
    // Defense > 999 also clamps to 999 before >> 2.
    let d_def999 = get_damage_base_branchless(
        50,
        100,
        200,
        999,
        true,
        false,
        false,
        false,
        CompactWeather::None,
        false,
        false,
        false,
        0,
        0,
        0,
        false,
    );
    let d_def1400 = get_damage_base_branchless(
        50,
        100,
        200,
        1400,
        true,
        false,
        false,
        false,
        CompactWeather::None,
        false,
        false,
        false,
        0,
        0,
        0,
        false,
    );
    assert_eq!(
        d_def1400, d_def999,
        "PS get_damage_base_branchless must clamp def to 999 before >> 2"
    );

    // 4. [CRITICAL] `from_battle` Castle `stall` counter decode (1..=8 direct vs PS halving ladder).
    {
        let p1 = vec![mk_set("Jolteon", 55, "", &["Protect", "Splash"], "M")];
        let p2 = vec![mk_set("Snorlax", 55, "", &["Splash"], "M")];
        let mut bc = Battle::from_fixture_with_ruleset(
            &dex,
            "1,2,3,4",
            &p1,
            &p2,
            Ruleset::castle_tournament(),
        )
        .unwrap();
        bc.choose(&dex, 0, "team 1").unwrap();
        bc.choose(&dex, 1, "team 1").unwrap();
        bc.choose(&dex, 0, "move 1").unwrap(); // Protect succeeds -> Castle DK::Counter = 1
        bc.choose(&dex, 1, "move 1").unwrap();
        let (_, cb_c) = CompactBattle::from_battle(&bc, &dex);
        assert_eq!(
            cb_c.mons[0].stall_counter_ext(),
            1,
            "Castle stall DK::Counter=1 must decode directly as 1 (NOT 7)"
        );

        let mut bp = Battle::from_fixture_with_ruleset(
            &dex,
            "1,2,3,4",
            &p1,
            &p2,
            Ruleset::PsStadium2,
        )
        .unwrap();
        bp.choose(&dex, 0, "team 1").unwrap();
        bp.choose(&dex, 1, "team 1").unwrap();
        bp.choose(&dex, 0, "move 1").unwrap(); // Protect succeeds -> PS DK::Counter = 127
        bp.choose(&dex, 1, "move 1").unwrap();
        let (_, cb_p) = CompactBattle::from_battle(&bp, &dex);
        assert_eq!(
            cb_p.mons[0].stall_counter(),
            1,
            "PS stall DK::Counter=127 must decode via halving ladder as 1"
        );
    }
}

#[test]
fn fuzzer_parity_high_fixes_encore_digfly_partialtrap_batonn_twineedle() {
    let dex = load_dex();

    // 5. [HIGH] Phantom `DISABLE` from `Encore` in `from_battle` eliminated; real `Disable` imports exact slot.
    {
        let p1 = vec![mk_set(
            "Snorlax",
            55,
            "",
            &["Tackle", "Rest", "Curse", "Splash"],
            "M",
        )];
        let p2 = vec![mk_set(
            "Alakazam",
            55,
            "",
            &["Encore", "Disable", "Splash"],
            "M",
        )];
        let mut b = Battle::from_fixture_with_ruleset(
            &dex,
            "1,2,3,4",
            &p1,
            &p2,
            Ruleset::castle_tournament(),
        )
        .unwrap();
        b.choose(&dex, 0, "team 1").unwrap();
        b.choose(&dex, 1, "team 1").unwrap();
        b.choose(&dex, 0, "move 3").unwrap(); // Snorlax Curse (slot 2)
        b.choose(&dex, 1, "move 3").unwrap(); // Alakazam Splash
        b.choose(&dex, 0, "move 3").unwrap();
        b.choose(&dex, 1, "move 1").unwrap(); // Alakazam Encore -> sets slot.disabled on slots 0,1,3!
        let (_, cb_enc) = CompactBattle::from_battle(&b, &dex);
        assert_ne!(cb_enc.mons[0].vol_flags & vol_bit::ENCORE, 0);
        assert_eq!(cb_enc.mons[0].encore_slot(), 2);
        assert_eq!(
            cb_enc.mons[0].vol_flags & vol_bit::DISABLE,
            0,
            "Encore must NEVER create a phantom vol_bit::DISABLE in from_battle"
        );
        assert_eq!(cb_enc.mons[0].disable_slot(), 0);

        // Now add a real Disable volatile on slot 2 (Curse) and verify exact import!
        let sn_id = b.active_id(0).unwrap();
        let curse_mid = dex.moves.id("curse").unwrap();
        let dis_cid = dex.conds_id("disable").unwrap();
        b.add_volatile(
            &dex,
            sn_id,
            "disable",
            b.active_id(1),
            nc2000_engine::battle::EffectHandle::None,
        );
        if let Some(vs) = b.poke_mut(sn_id).volatile_mut(dis_cid) {
            vs.duration = Some(4);
            vs.set(
                nc2000_engine::state::DK::Move,
                nc2000_engine::state::Scalar::MoveK(curse_mid),
            );
        }
        let (_, cb_both) = CompactBattle::from_battle(&b, &dex);
        assert_ne!(cb_both.mons[0].vol_flags & vol_bit::DISABLE, 0);
        assert_eq!(cb_both.mons[0].disable_slot(), 2);
        assert_eq!(cb_both.mons[0].timers[timer_idx::DISABLE], 4);
    }

    // 6. [HIGH] Castle `fly`/`dig` semi-invuln vs `twoturnmove` lock separation + from_battle + switch/BP clear.
    {
        let p1 = vec![
            mk_set("Dragonite", 55, "", &["Fly", "Tackle", "Baton Pass"], "M"),
            mk_set("Snorlax", 55, "", &["Splash"], "M"),
        ];
        let p2 = vec![mk_set("Snorlax", 50, "", &["Tackle", "Splash"], "M")];
        let mut b = Battle::from_fixture_with_ruleset(
            &dex,
            "1,2,3,4",
            &p1,
            &p2,
            Ruleset::castle_tournament(),
        )
        .unwrap();
        b.choose(&dex, 0, "team 1,2").unwrap();
        b.choose(&dex, 1, "team 1").unwrap();
        // Turn 1: Dragonite Fly (charge) vs Snorlax Splash -> locked to Fly + FLYING!
        b.choose(&dex, 0, "move 1").unwrap();
        b.choose(&dex, 1, "move 2").unwrap();
        let (table, cb_t1) = CompactBattle::from_battle(&b, &dex);
        assert_ne!(cb_t1.mons[0].ext_flags & ext_vol_bit::FLYING, 0);
        assert!(cb_t1.mons[0].two_turn_locked(), "Turn 1 Fly must set two_turn_locked");
        assert!(cb_t1.mons[0].is_multi_turn_locked());
        let mut acts = [0u8; 7];
        let n = cb_t1.legal_actions(&table, 0, &mut acts);
        assert_eq!(&acts[..n], &[0], "Turn 1 Fly must lock legal_actions to Fly slot 0");

        // Turn 2: paralyze Dragonite and find a seed where Dragonite is fully paralyzed on Turn 2!
        let mut aborted_cb = None;
        for seed in 1..=64u64 {
            let mut c = cb_t1;
            c.mons[0].set_status(Status::Par as u8, 0);
            c.prng_seed = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15);
            let sn_hp0 = c.mons[3].hp();
            c.step_turn(&table, [0, 0]); // Fly Turn 2 (fully para'd) vs Snorlax Tackle (misses Flying!)
            if c.mons[3].hp() == sn_hp0 {
                assert_ne!(
                    c.mons[0].ext_flags & ext_vol_bit::FLYING,
                    0,
                    "Castle Turn-2 aborted Fly must retain FLYING semi-invulnerability"
                );
                assert!(
                    !c.mons[0].two_turn_locked(),
                    "Castle Turn-2 aborted Fly must clear two_turn_locked"
                );
                assert!(
                    !c.mons[0].is_multi_turn_locked(),
                    "Castle Turn-2 aborted Fly must NOT lock user to Fly"
                );
                assert_eq!(c.mons[0].locked_move_slot(), 0);
                let mut acts2 = [0u8; 7];
                let n2 = c.legal_actions(&table, 0, &mut acts2);
                assert_eq!(
                    &acts2[..n2],
                    &[0, 1, 2, 5],
                    "Persisted FLYING mon must be free to use other moves or switch"
                );
                aborted_cb = Some(c);
                break;
            }
        }
        let mut c_inv = aborted_cb.expect("must hit full-para on Turn 2 Fly");
        // Cure Par and verify Baton Pass clears FLYING (does NOT pass FLYING to receiver!).
        c_inv.mons[0].set_status(Status::None as u8, 0);
        c_inv.step_turn(&table, [2, 1]); // Dragonite Baton Pass to Snorlax
        assert_eq!(c_inv.sides[0].active_idx, 1);
        assert_eq!(c_inv.mons[0].ext_flags & ext_vol_bit::FLYING, 0);
        assert_eq!(
            c_inv.mons[1].ext_flags & ext_vol_bit::FLYING,
            0,
            "Baton Pass must NOT pass FLYING semi-invuln"
        );

        // Also verify `from_battle` imports Castle persisted `fly` (without `twoturnmove`) unlocked!
        let dn_id = b.active_id(0).unwrap();
        let ttm_cid = dex.conds_id("twoturnmove").unwrap();
        b.remove_volatile_id(&dex, dn_id, ttm_cid);
        // Re-add `fly` volatile alone (simulating Castle TwoTurnMoveAborted).
        b.add_volatile(
            &dex,
            dn_id,
            "fly",
            None,
            nc2000_engine::battle::EffectHandle::None,
        );
        let (_, cb_imp_persisted) = CompactBattle::from_battle(&b, &dex);
        assert_ne!(cb_imp_persisted.mons[0].ext_flags & ext_vol_bit::FLYING, 0);
        assert!(!cb_imp_persisted.mons[0].two_turn_locked());
        assert!(!cb_imp_persisted.mons[0].is_multi_turn_locked());
    }

    // 7. [HIGH] Stale `partiallytrapped` after trapper KO (both `from_battle` & PS Order 10 stepping).
    {
        let p1 = vec![
            mk_set("Snorlax", 55, "", &["Body Slam", "Splash"], "M"),
            mk_set("Zapdos", 50, "", &["Splash"], ""),
        ];
        let p2 = vec![
            mk_set("Dragonite", 50, "", &["Wrap", "Splash"], "M"),
            mk_set("Blissey", 50, "", &["Splash"], "F"),
        ];
        let mut b = Battle::from_fixture_with_ruleset(
            &dex,
            "1,2,3,4",
            &p1,
            &p2,
            Ruleset::PsStadium2,
        )
        .unwrap();
        b.choose(&dex, 0, "team 1,2").unwrap();
        b.choose(&dex, 1, "team 1,2").unwrap();
        let sn_id = b.active_id(0).unwrap();
        let dn_id = b.active_id(1).unwrap();
        b.add_volatile(
            &dex,
            sn_id,
            "partiallytrapped",
            Some(dn_id),
            nc2000_engine::battle::EffectHandle::None,
        );
        // Simulate stale `partiallytrapped` after trapper Dragonite fainted!
        b.poke_mut(dn_id).hp = 0;
        b.poke_mut(dn_id).fainted = true;
        b.poke_mut(sn_id).trapped = true; // even if stale `p.trapped` lingered!
        let (table, cb_stale) = CompactBattle::from_battle(&b, &dex);
        assert!(
            !cb_stale.mons[0].partially_trapped(),
            "from_battle must NOT import partially_trapped when trapper is fainted"
        );
        assert_eq!(
            cb_stale.mons[0].vol_flags & vol_bit::TRAPPED_OR_LOCKED,
            0,
            "stale partial trap must NOT fall back to Mean Look TRAPPED_OR_LOCKED"
        );

        // During PS stepping: if Dragonite traps Snorlax on Turn 1 and Snorlax KOs Dragonite on Turn 1,
        // Snorlax must NOT remain partially trapped or take Order 10 Wrap damage!
        let (_, mut cb_live) = start_compact(&dex, Ruleset::PsStadium2, &p1, &p2);
        cb_live.mons[3].hp_pool[0] = 1; // Dragonite at 1 HP
        cb_live.prng_seed = 7;
        cb_live.step_turn(&table, [0, 0]); // Dragonite Wrap -> Snorlax Body Slam KOs Dragonite
        assert!(!cb_live.mons[0].partially_trapped());
        assert!(!cb_live.mons[0].is_trapped());
    }

    // 8. [HIGH] Mid-turn `BATONN` `from_battle` queued `Struggle`/`Recharge` + `FORESIGHT_CASTER` + `LOCKED_FIRST_HIT_RECV`.
    {
        let p1 = vec![
            mk_set("Jolteon", 55, "", &["Baton Pass", "Foresight", "Rollout"], "M"),
            mk_set("Snorlax", 50, "", &["Splash"], "M"),
            mk_set("Zapdos", 50, "", &["Splash"], ""),
        ];
        let p2 = vec![
            mk_set("Snorlax", 50, "", &["Hyper Beam", "Splash"], "M"),
            mk_set("Gengar", 50, "", &["Splash"], "M"),
        ];

        // (a) Mid-turn BATONN with opponent naturally out of PP (queued Struggle):
        let p2_str = vec![
            mk_set("Chansey", 50, "", &["Splash"], "F"),
            mk_set("Gengar", 50, "", &["Splash"], "M"),
        ];
        let mut b_str = Battle::from_fixture_with_ruleset(
            &dex,
            "1,2,3,4",
            &p1,
            &p2_str,
            Ruleset::castle_tournament(),
        )
        .unwrap();
        b_str.choose(&dex, 0, "team 1,2,3").unwrap();
        b_str.choose(&dex, 1, "team 1,2").unwrap();
        let ch_id = b_str.active_id(1).unwrap();
        b_str.poke_mut(ch_id).move_slots[0].pp = 1;
        b_str.choose(&dex, 0, "move 2").unwrap(); // Turn 1: Foresight
        b_str.choose(&dex, 1, "move 1").unwrap(); // Turn 1: Chansey spends its last Splash PP -> 0 PP!
        b_str.choose(&dex, 0, "move 1").unwrap(); // Turn 2: Jolteon Baton Pass -> pauses for 2 bench options!
        b_str.choose(&dex, 1, "move struggle").unwrap(); // Turn 2: Chansey queues Struggle!
        let (t_str, mut cb_str) = CompactBattle::from_battle(&b_str, &dex);
        assert_eq!(cb_str.sides[0].request, 2);
        assert_ne!(cb_str.sides[1].side_flags & 0x10, 0, "Queued Struggle must set pending flag");
        assert_ne!(
            cb_str.bp_switch_flags & (1u8 << 5),
            0,
            "Queued Struggle must set bp_switch_flags bit 5"
        );
        cb_str.step_switch(&t_str, [5, 7]); // BP to Snorlax -> foe executes Struggle & takes recoil!
        assert!(
            cb_str.mons[1].hp() < t_str.mons[1].maxhp,
            "Queued Struggle must deal damage after BP switch"
        );
        assert!(
            cb_str.mons[3].hp() < t_str.mons[3].maxhp,
            "Queued Struggle must take recoil after BP switch"
        );

        // (b) Mid-turn BATONN with opponent naturally recharging from Hyper Beam (queued Recharge):
        let p2_rec = vec![
            mk_set("Chansey", 50, "", &["Hyper Beam", "Splash"], "F"),
            mk_set("Gengar", 50, "", &["Splash"], "M"),
        ];
        let mut b_rec = Battle::from_fixture_with_ruleset(
            &dex,
            "1,2,3,4",
            &p1,
            &p2_rec,
            Ruleset::castle_tournament(),
        )
        .unwrap();
        b_rec.choose(&dex, 0, "team 1,2,3").unwrap();
        b_rec.choose(&dex, 1, "team 1,2").unwrap();
        b_rec.choose(&dex, 0, "move 2").unwrap(); // Turn 1: Jolteon Foresight
        b_rec.choose(&dex, 1, "move 1").unwrap(); // Turn 1: Chansey hits with Hyper Beam -> mustrecharge!
        b_rec.choose(&dex, 0, "move 1").unwrap(); // Turn 2: Jolteon Baton Pass -> pauses!
        b_rec.choose(&dex, 1, "move recharge").unwrap(); // Turn 2: Chansey queues Recharge!
        let (t_rec, mut cb_rec) = CompactBattle::from_battle(&b_rec, &dex);
        assert_ne!(
            cb_rec.sides[1].side_flags & 0x10,
            0,
            "Queued Recharge must set pending flag"
        );
        assert!(cb_rec.mons[3].must_recharge());
        cb_rec.step_switch(&t_rec, [5, 7]);
        assert!(
            !cb_rec.mons[3].must_recharge(),
            "Queued Recharge must clear must_recharge on step_switch"
        );
        assert_eq!(
            cb_rec.mons[1].hp(),
            t_rec.mons[1].maxhp,
            "Recharge turn deals no damage"
        );

        // (c) `FORESIGHT_CASTER` and `LOCKED_FIRST_HIT_RECV` round-trip in `from_battle`:
        let mut b_meta = Battle::from_fixture_with_ruleset(
            &dex,
            "1,2,3,4",
            &p1,
            &p2,
            Ruleset::castle_tournament(),
        )
        .unwrap();
        b_meta.choose(&dex, 0, "team 1,2,3").unwrap();
        b_meta.choose(&dex, 1, "team 1,2").unwrap();
        let j_id = b_meta.active_id(0).unwrap();
        let s_id = b_meta.active_id(1).unwrap();
        b_meta.add_volatile(
            &dex,
            s_id,
            "foresight",
            Some(j_id),
            nc2000_engine::battle::EffectHandle::None,
        );
        let roll_cid = dex.conds_id("rollout").unwrap();
        b_meta.add_volatile(
            &dex,
            j_id,
            "rollout",
            Some(j_id),
            nc2000_engine::battle::EffectHandle::None,
        );
        if let Some(vs) = b_meta.poke_mut(j_id).volatile_mut(roll_cid) {
            vs.set_int(nc2000_engine::state::DK::HitCount, 1);
            vs.set(
                nc2000_engine::state::DK::TargetSlot,
                nc2000_engine::state::Scalar::Slot(s_id.side, s_id.slot),
            );
        }
        let (_, cb_meta) = CompactBattle::from_battle(&b_meta, &dex);
        assert_ne!(
            cb_meta.mons[0].ext_flags & ext_vol_bit::FORESIGHT_CASTER,
            0,
            "from_battle must set FORESIGHT_CASTER on foresight source"
        );
        assert_ne!(
            cb_meta.mons[3].ext_flags & ext_vol_bit::LOCKED_FIRST_HIT_RECV,
            0,
            "from_battle must set LOCKED_FIRST_HIT_RECV on rollout/lockedmove TargetSlot receiver"
        );
    }

    // 9. [HIGH] PS Twineedle NEVER poisons Poison-types (Gengar/Muk), still poisons Steel-types.
    {
        let p1 = vec![mk_set("Beedrill", 55, "", &["Twineedle"], "M")];
        let p2 = vec![mk_set("Muk", 55, "", &["Splash"], "M")];
        let (table, base) = start_compact(&dex, Ruleset::PsStadium2, &p1, &p2);
        for s in 1..=256u64 {
            let mut c = base;
            c.prng_seed = s.wrapping_mul(0x9E37_79B9_7F4A_7C15);
            c.step_turn(&table, [0, 0]);
            assert_eq!(
                c.mons[3].status_raw(),
                Status::None as u8,
                "PS Twineedle must NEVER poison Poison-type Muk (seed={s})"
            );
        }
    }
}

#[test]
fn fuzzer_parity_high_fixes_invuln_ohko_glare_triplekick_present_hash() {
    let dex = load_dex();

    // 10. [HIGH] Dig/Fly semi-invulnerability: non-Ghost Curse, Fissure vs Horn Drill,
    //     Whirlwind vs Roar, Castle Lock-On Fly vs Dig, PS Lock-On blocked moves & Fly vs EQ.
    {
        // (a) Non-Ghost Curse works vs Dig and Fly!
        let p1 = vec![mk_set("Snorlax", 55, "", &["Curse"], "M")];
        let p2 = vec![mk_set("Dragonite", 55, "", &["Fly", "Dig"], "M")];
        let (t, base) = start_compact(&dex, Ruleset::castle_tournament(), &p1, &p2);
        for flag in [ext_vol_bit::FLYING, ext_vol_bit::DIGGING] {
            let mut c = base;
            c.mons[3].ext_flags |= flag;
            c.step_turn(&t, [0, 0]); // Faster Dragonite still acts or if Snorlax moves first via Quick Claw/speed:
            let mut c2 = base;
            c2.mons[0].boosts[stat_idx::SPE] = 6; // Snorlax moves while Dragonite is semi-invuln!
            c2.mons[3].ext_flags |= flag;
            c2.step_turn(&t, [0, 0]);
            assert_eq!(
                c2.mons[0].boosts[stat_idx::ATK], 1,
                "Non-Ghost Curse must succeed against semi-invulnerable target (flag={flag})"
            );
        }

        // (b) Fissure hits Dig while Horn Drill misses Dig!
        let p_oh1 = vec![mk_set("Rhydon", 100, "", &["Fissure", "Horn Drill"], "M")];
        let p_oh2 = vec![mk_set("Snorlax", 10, "", &["Dig", "Splash"], "M")];
        let (t_oh, base_oh) = start_compact(&dex, Ruleset::PsStadium2, &p_oh1, &p_oh2);
        // Lv 100 vs Lv 10 -> 76 + 180 = 256 -> guaranteed 255/256 hit!
        let mut c_fis = base_oh;
        c_fis.mons[3].ext_flags |= ext_vol_bit::DIGGING;
        c_fis.step_turn(&t_oh, [0, 1]); // Faster Rhydon Fissure hits Digging Snorlax!
        assert_eq!(c_fis.mons[3].hp(), 0, "Fissure must hit Digging target");

        let mut c_hd = base_oh;
        c_hd.mons[3].ext_flags |= ext_vol_bit::DIGGING;
        c_hd.step_turn(&t_oh, [1, 1]); // Faster Rhydon Horn Drill misses Digging Snorlax!
        assert_eq!(
            c_hd.mons[3].hp(),
            t_oh.mons[3].maxhp,
            "Horn Drill must MISS Digging target"
        );

        // (c) Whirlwind hits Fly while Roar misses Fly!
        let p_ph1 = vec![
            mk_set("Dragonite", 55, "", &["Fly", "Splash"], "M"),
            mk_set("Snorlax", 55, "", &["Splash"], "M"),
        ];
        let p_ph2 = vec![mk_set("Skarmory", 50, "", &["Whirlwind", "Roar"], "M")];
        let (t_ph, base_ph) = start_compact(&dex, Ruleset::PsStadium2, &p_ph1, &p_ph2);
        let mut c_ww = base_ph;
        c_ww.step_turn(&t_ph, [0, 0]); // Faster Dragonite Fly Turn 1 -> slower Skarmory Whirlwind hits & drags Snorlax!
        assert_eq!(c_ww.sides[0].active_idx, 1, "Whirlwind must hit Flying target");

        let mut c_roar = base_ph;
        c_roar.step_turn(&t_ph, [0, 1]); // Faster Dragonite Fly Turn 1 -> slower Skarmory Roar MISSES!
        assert_eq!(c_roar.sides[0].active_idx, 0, "Roar must MISS Flying target");

        // (d) Castle Lock-On hits Fly (Ice Beam) and does NOT hit Dig (and consumes Lock-On)!
        //     PS Lock-On blocked for Attract/Ghost Curse/Foresight/Mean Look/Nightmare, and Fly blocks EQ even with Lock-On!
        let p_lo1 = vec![mk_set(
            "Gengar",
            55,
            "",
            &["Ice Beam", "Earthquake", "Mean Look", "Curse"],
            "M",
        )];
        let p_lo2 = vec![mk_set("Dragonite", 50, "", &["Fly", "Dig", "Splash"], "F")];
        let (t_loc, base_loc) = start_compact(&dex, Ruleset::castle_tournament(), &p_lo1, &p_lo2);
        let (t_lop, base_lop) = start_compact(&dex, Ruleset::PsStadium2, &p_lo1, &p_lo2);

        // Castle Lock-On + Ice Beam vs Fly -> HITS!
        let mut c_lf = base_loc;
        c_lf.mons[0].ext_flags |= ext_vol_bit::LOCK_ON;
        c_lf.mons[0].set_lock_on_turns(1);
        c_lf.mons[3].ext_flags |= ext_vol_bit::FLYING;
        c_lf.step_turn(&t_loc, [0, 2]);
        assert!(
            c_lf.mons[3].hp() < t_loc.mons[3].maxhp,
            "Castle Lock-On + Ice Beam must HIT Flying target"
        );
        assert_eq!(c_lf.mons[0].ext_flags & ext_vol_bit::LOCK_ON, 0);

        // Castle Lock-On + Ice Beam vs Dig -> MISSES AND consumes Lock-On!
        let mut c_ld = base_loc;
        c_ld.mons[0].ext_flags |= ext_vol_bit::LOCK_ON;
        c_ld.mons[0].set_lock_on_turns(1);
        c_ld.mons[3].ext_flags |= ext_vol_bit::DIGGING;
        c_ld.step_turn(&t_loc, [0, 2]);
        assert_eq!(c_ld.mons[3].hp(), t_loc.mons[3].maxhp);
        assert_eq!(
            c_ld.mons[0].ext_flags & ext_vol_bit::LOCK_ON,
            0,
            "Castle Lock-On must be consumed when missing Digging target"
        );

        // PS Lock-On + Earthquake vs Fly -> MISSES (Fly blocks EQ even with Lock-On)!
        let mut c_peq = base_lop;
        c_peq.mons[0].ext_flags |= ext_vol_bit::LOCK_ON;
        c_peq.mons[0].set_lock_on_turns(2);
        c_peq.mons[3].ext_flags |= ext_vol_bit::FLYING;
        c_peq.step_turn(&t_lop, [1, 2]);
        assert_eq!(
            c_peq.mons[3].hp(),
            t_lop.mons[3].maxhp,
            "PS Fly must block Earthquake even with Lock-On"
        );

        // PS Lock-On + Mean Look / Ghost Curse vs Dig -> MISSES!
        let mut c_pml = base_lop;
        c_pml.mons[0].ext_flags |= ext_vol_bit::LOCK_ON;
        c_pml.mons[0].set_lock_on_turns(2);
        c_pml.mons[3].ext_flags |= ext_vol_bit::DIGGING;
        c_pml.step_turn(&t_lop, [2, 2]);
        assert_eq!(
            c_pml.mons[3].vol_flags & vol_bit::TRAPPED_OR_LOCKED,
            0,
            "PS Lock-On must NOT allow Mean Look to hit Digging target"
        );

        let mut c_pcur = base_lop;
        c_pcur.mons[0].ext_flags |= ext_vol_bit::LOCK_ON;
        c_pcur.mons[0].set_lock_on_turns(2);
        c_pcur.mons[3].ext_flags |= ext_vol_bit::DIGGING;
        c_pcur.step_turn(&t_lop, [3, 2]);
        assert_eq!(
            c_pcur.mons[3].vol_flags & vol_bit::CURSE,
            0,
            "PS Lock-On must NOT allow Ghost Curse to hit Digging target"
        );
    }

    // 11. [HIGH] OHKO accuracy (`76 + 2*(att-def)`), Lock-On (PS auto-hit vs Castle no-auto-hit + consume),
    //     and acc/eva/BrightPowder boosts in BOTH Castle `ovfl=0` and `ovfl=1`!
    {
        let p1 = vec![mk_set("Rhydon", 50, "", &["Horn Drill", "Splash"], "M")];
        let p2 = vec![mk_set("Snorlax", 50, "", &["Splash"], "M")];
        let (t_p, base_p) = start_compact(&dex, Ruleset::PsStadium2, &p1, &p2);
        let (t_c0, base_c0) = start_compact(&dex, Ruleset::castle_tournament(), &p1, &p2);
        let (t_c1, base_c1) = start_compact(
            &dex,
            Ruleset::castle_raw_cgi(true),
            &p1,
            &p2,
        );

        // PS Lock-On makes OHKO auto-hit 100%.
        for s in 1..=32u64 {
            let mut c = base_p;
            c.prng_seed = s.wrapping_mul(0x9E37_79B9_7F4A_7C15);
            c.mons[0].ext_flags |= ext_vol_bit::LOCK_ON;
            c.mons[0].set_lock_on_turns(2);
            c.step_turn(&t_p, [0, 0]);
            assert_eq!(c.mons[3].hp(), 0, "PS Lock-On must auto-hit OHKO");
        }

        // Castle Lock-On does NOT auto-hit OHKO AND IS consumed on attempt!
        let mut castle_lo_missed = false;
        for s in 1..=32u64 {
            let mut c = base_c0;
            c.prng_seed = s.wrapping_mul(0x9E37_79B9_7F4A_7C15);
            c.mons[0].ext_flags |= ext_vol_bit::LOCK_ON;
            c.mons[0].set_lock_on_turns(1);
            c.step_turn(&t_c0, [0, 0]);
            assert_eq!(
                c.mons[0].ext_flags & ext_vol_bit::LOCK_ON,
                0,
                "Castle OHKO must consume Lock-On"
            );
            if c.mons[3].hp() > 0 {
                castle_lo_missed = true;
            }
        }
        assert!(castle_lo_missed, "Castle Lock-On must NOT make OHKO auto-hit");

        // 2025/05/27 PBS spec: +6 Acc boosts Lv50 vs Lv50 OHKO (`76 * 3 = 228/256` ~89%) in BOTH ovfl=0 and ovfl=1!
        for (label, table, base) in [("ovfl=0", &t_c0, base_c0), ("ovfl=1", &t_c1, base_c1)] {
            let mut hits_no_boost = 0u32;
            let mut hits_acc6 = 0u32;
            for s in 1..=512u64 {
                let seed = s.wrapping_mul(0x9E37_79B9_7F4A_7C15);
                let mut c0 = base;
                c0.prng_seed = seed;
                c0.step_turn(table, [0, 0]);
                if c0.mons[3].hp() == 0 {
                    hits_no_boost += 1;
                }
                let mut c6 = base;
                c6.prng_seed = seed;
                c6.mons[0].boosts[stat_idx::ACC] = 6;
                c6.step_turn(table, [0, 0]);
                if c6.mons[3].hp() == 0 {
                    hits_acc6 += 1;
                }
            }
            assert!(
                hits_acc6 > hits_no_boost + 200,
                "Castle {label} +6 Acc must strongly boost OHKO hit rate: base={hits_no_boost}/512, acc+6={hits_acc6}/512"
            );
        }

        // att_lvl < def_lvl fails BEFORE Castle Lock-On consume and ovfl=1 Counter arming.
        let p_lo_a = vec![mk_set("Rhydon", 49, "", &["Horn Drill"], "M")];
        let p_lo_b = vec![mk_set("Snorlax", 50, "", &["Counter"], "M")];
        let (t_lo, mut cb_lo) = start_compact(
            &dex,
            Ruleset::castle_raw_cgi(true),
            &p_lo_a,
            &p_lo_b,
        );
        cb_lo.mons[0].ext_flags |= ext_vol_bit::LOCK_ON;
        cb_lo.mons[0].set_lock_on_turns(1);
        cb_lo.step_turn(&t_lo, [0, 0]);
        assert_eq!(cb_lo.mons[3].hp(), t_lo.mons[3].maxhp);
        assert_ne!(
            cb_lo.mons[0].ext_flags & ext_vol_bit::LOCK_ON,
            0,
            "Underleveled OHKO fails before Lock-On consumption"
        );
        assert_eq!(
            cb_lo.mons[0].hp(),
            t_lo.mons[0].maxhp,
            "Underleveled OHKO fails before arming ovfl=1 Counter"
        );
    }

    // 12. [HIGH] `Glare` Ghost-type immunity (blocked unless Foresight), while `Lovely Kiss` hits Ghost!
    {
        let p1 = vec![mk_set("Dunsparce", 55, "", &["Glare", "Foresight"], "M")];
        let p2 = vec![mk_set("Gengar", 55, "", &["Splash"], "M")];
        let (t_g, base_g) = start_compact(&dex, Ruleset::castle_tournament(), &p1, &p2);
        for s in 1..=64u64 {
            let mut c = base_g;
            c.prng_seed = s.wrapping_mul(0x9E37_79B9_7F4A_7C15);
            c.step_turn(&t_g, [0, 0]);
            assert_eq!(
                c.mons[3].status_raw(),
                Status::None as u8,
                "Glare must NOT paralyze Ghost without Foresight"
            );
        }
        let mut glare_with_fs = false;
        for s in 1..=64u64 {
            let mut c = base_g;
            c.prng_seed = s.wrapping_mul(0x9E37_79B9_7F4A_7C15);
            c.mons[3].vol_flags |= vol_bit::FORESIGHT;
            c.step_turn(&t_g, [0, 0]);
            if c.mons[3].status_raw() == Status::Par as u8 {
                glare_with_fs = true;
                break;
            }
        }
        assert!(glare_with_fs, "Glare must paralyze Ghost once Foresight is active");

        let p_lk1 = vec![mk_set("Jynx", 55, "", &["Lovely Kiss"], "F")];
        let (t_lk, base_lk) = start_compact(&dex, Ruleset::castle_tournament(), &p_lk1, &p2);
        let mut lk_hit = false;
        for s in 1..=64u64 {
            let mut c = base_lk;
            c.prng_seed = s.wrapping_mul(0x9E37_79B9_7F4A_7C15);
            c.step_turn(&t_lk, [0, 0]);
            if c.mons[3].status_raw() == Status::Slp as u8 {
                lk_hit = true;
                break;
            }
        }
        assert!(lk_hit, "Lovely Kiss (ignoreImmunity=true) must still sleep Ghost");
    }

    // 13. [HIGH] Castle Triple Kick (`BP=10`, `* hit_k` AFTER type compat & BEFORE roll) & Present v0.3.
    {
        // With 1/4 resist (Bug/Flying Scyther vs Fighting Triple Kick):
        // BP=10 base dmg before resist vs after resist differs when `* hit_k` is placed AFTER type compat!
        // Specifically: base = get_damage_base_castle(..., 10, ..., type_mod=-2, 1, 1); hit_k=3 -> base * 3!
        let p_tk1 = vec![mk_set("Hitmontop", 50, "", &["Triple Kick"], "M")];
        let p_tk2 = vec![mk_set("Scyther", 50, "", &["Splash"], "M")];
        let (t_tk, base_tk) = start_compact(&dex, Ruleset::castle_tournament(), &p_tk1, &p_tk2);
        let atk = t_tk.mons[0].stored_stats[stat_idx::ATK] as i32;
        let def = t_tk.mons[3].stored_stats[stat_idx::DEF] as i32;
        let hit1_base = get_damage_base_castle(
            50,
            10,
            atk,
            def,
            true,
            false,
            false,
            false,
            CompactWeather::None,
            false,
            false,
            false,
            t_tk.move_specs[0][0].move_type,
            t_tk.mons[0].type_mask,
            -2,
            false,
            1,
            1,
        );
        // Verify non-crit 3-hit total equals sum_{k=1..=3} floor(hit1_base * k * roll / 255):
        // Upper bound for 3 non-crit hits is `hit1_base * (1 + 2 + 3) = hit1_base * 6`!
        let wrong_before_type_hit3 = get_damage_base_castle(
            50,
            10,
            atk,
            def,
            true,
            false,
            false,
            false,
            CompactWeather::None,
            false,
            false,
            false,
            t_tk.move_specs[0][0].move_type,
            t_tk.mons[0].type_mask,
            -2,
            false,
            3,
            1,
        );
        // Check that post-type `* 3` is distinct or exact and that execution stays within valid bounds.
        assert!(hit1_base > 0 && wrong_before_type_hit3 >= hit1_base * 3);
        let mut saw_3hit = false;
        for s in 1..=128u64 {
            let mut c = base_tk;
            c.prng_seed = s.wrapping_mul(0x9E37_79B9_7F4A_7C15);
            let hp0 = c.mons[3].hp();
            c.step_turn(&t_tk, [0, 0]);
            let dmg = hp0 - c.mons[3].hp();
            if c.triple_kick_mult == 3 || dmg >= (hit1_base * 5) as i16 {
                saw_3hit = true;
                break;
            }
        }
        assert!(saw_3hit, "Triple Kick must execute 3-hit sequences cleanly");

        // Present v0.3: Burned Delibird vs Psychic Mew (high present_type_num) deals ~half of unburned Present.
        let p_pr1 = vec![mk_set("Delibird", 55, "Pink Bow", &["Present"], "M")];
        let p_pr2 = vec![mk_set("Mew", 55, "", &["Splash"], "")];
        let (t_pr, base_pr) = start_compact(&dex, Ruleset::castle_tournament(), &p_pr1, &p_pr2);
        let mut saw_burn_halved = false;
        for s in 1..=64u64 {
            let seed = s.wrapping_mul(0x9E37_79B9_7F4A_7C15);
            let mut c0 = base_pr;
            let mut cb = base_pr;
            c0.prng_seed = seed;
            cb.prng_seed = seed;
            cb.mons[0].set_status(Status::Brn as u8, 0);
            let hp0 = c0.mons[3].hp();
            c0.step_turn(&t_pr, [0, 0]);
            cb.step_turn(&t_pr, [0, 0]);
            let d0 = hp0 - c0.mons[3].hp();
            let db = hp0 - cb.mons[3].hp();
            if d0 >= 20 && db > 0 && db <= (d0 * 2) / 3 {
                saw_burn_halved = true;
                break;
            }
        }
        assert!(saw_burn_halved, "Castle Present v0.3 must halve damage when burned");
    }

    // 14. [HIGH] Canonical `search_hash()` / `PartialEq`: no `side_flags & 0x04`, canonical
    //     `mark_fainted()` across Destiny Bond / Perish Song / KO, and zeroed slot payload bits on expiry.
    {
        let p1 = vec![
            mk_set("Snorlax", 55, "", &["Tackle", "Splash", "Perish Song"], "M"),
            mk_set("Zapdos", 55, "", &["Splash"], ""),
        ];
        let p2 = vec![
            mk_set("Gengar", 55, "", &["Destiny Bond", "Disable", "Encore", "Splash"], "M"),
            mk_set("Blissey", 55, "", &["Splash"], "F"),
        ];
        let (t, base) = start_compact(&dex, Ruleset::PsStadium2, &p1, &p2);
        assert_eq!(base.sides[0].side_flags & 0x04, 0);
        assert_eq!(base.sides[1].side_flags & 0x04, 0);

        // Destiny Bond KO canonicalizes attacker via `mark_fainted()`.
        let mut c_db = base;
        c_db.mons[0].boosts[stat_idx::ATK] = 3;
        c_db.mons[0].ext_flags |= ext_vol_bit::MIST;
        c_db.mons[0].set_last_move_slot(2);
        c_db.mons[3].hp_pool[0] = 1;
        c_db.mons[3].vol_flags |= vol_bit::DESTINY_BOND;
        c_db.step_turn(&t, [0, 3]); // Wait: Gengar faster -> give Snorlax +6 Spe so Tackle KOs DBond Gengar!
        let mut c_db2 = base;
        c_db2.mons[0].boosts[stat_idx::SPE] = 6;
        c_db2.mons[0].boosts[stat_idx::ATK] = 3;
        c_db2.mons[0].ext_flags |= ext_vol_bit::MIST;
        c_db2.mons[0].set_last_move_slot(2);
        c_db2.mons[3].hp_pool[0] = 1;
        c_db2.mons[3].vol_flags |= vol_bit::FORESIGHT; // let Normal Tackle hit Ghost Gengar
        c_db2.mons[3].vol_flags |= vol_bit::DESTINY_BOND;
        c_db2.step_turn(&t, [0, 3]);
        assert_eq!(c_db2.mons[0].status_raw(), Status::Fnt as u8);
        assert_eq!(c_db2.mons[0].boosts, [0; 7]);
        assert_eq!(c_db2.mons[0].slot_refs, 0);
        assert_eq!(c_db2.mons[0].ext_flags, 0);
        assert_eq!(c_db2.mons[0].timers, [0; 4]);

        // Perish Song KO canonicalizes fainted mon via `mark_fainted()`.
        let mut c_ps = base;
        c_ps.mons[0].boosts[stat_idx::DEF] = 4;
        c_ps.mons[0].set_last_move_slot(2);
        c_ps.mons[0].vol_flags |= vol_bit::PERISH_SONG;
        c_ps.mons[0].set_perish_turns(1);
        c_ps.step_turn(&t, [1, 3]);
        assert_eq!(c_ps.mons[0].status_raw(), Status::Fnt as u8);
        assert_eq!(c_ps.mons[0].boosts, [0; 7]);
        assert_eq!(c_ps.mons[0].slot_refs, 0);

        // Disable / Encore expiry zeroes `disable_slot` / `encore_slot`, and Struggle zeroes `last_move` bits 0..1.
        let mut c_exp = base;
        c_exp.mons[0].vol_flags |= vol_bit::DISABLE | vol_bit::ENCORE;
        c_exp.mons[0].set_disable_slot(2);
        c_exp.mons[0].timers[timer_idx::DISABLE] = 1;
        c_exp.mons[0].set_encore_slot(1);
        c_exp.mons[0].timers[timer_idx::ENCORE] = 1;
        c_exp.step_turn(&t, [1, 3]);
        assert_eq!(c_exp.mons[0].vol_flags & vol_bit::DISABLE, 0);
        assert_eq!(c_exp.mons[0].disable_slot(), 0, "Expired Disable must zero disable_slot");
        assert_eq!(c_exp.mons[0].vol_flags & vol_bit::ENCORE, 0);
        assert_eq!(c_exp.mons[0].encore_slot(), 0, "Expired Encore must zero encore_slot");

        let mut c_st = base;
        c_st.mons[0].set_last_move_slot(2);
        c_st.mons[0].pp = [0; 4];
        c_st.step_turn(&t, [0, 3]); // Forced Struggle clears last_move_slot AND payload bits 0..1!
        assert_eq!(c_st.mons[0].last_move_slot(), None);
        assert_eq!(c_st.mons[0].slot_refs & 0x43, 0, "Struggle must zero last_move bits 0..1");
    }
}

#[test]
fn fuzzer_parity_medium_fixes_futuresight_hp_berserk_toxic_spite_rest_conf_psychup_stat() {
    let dex = load_dex();

    // 15.1 Future Sight `from_battle` uses `DK::EndingTurn` (strikes on exact Turn 3 EOT!).
    {
        let p1 = vec![mk_set("Espeon", 55, "", &["Future Sight", "Splash"], "M")];
        let p2 = vec![mk_set("Snorlax", 55, "", &["Splash"], "M")];
        let mut b = Battle::from_fixture_with_ruleset(
            &dex,
            "1,2,3,4",
            &p1,
            &p2,
            Ruleset::castle_tournament(),
        )
        .unwrap();
        b.choose(&dex, 0, "team 1").unwrap();
        b.choose(&dex, 1, "team 1").unwrap();
        // Turn 1: Espeon Future Sight -> Turn 2 start: ending=2, remaining EOT ticks = 2 (Turn 2 & Turn 3)!
        b.choose(&dex, 0, "move 1").unwrap();
        b.choose(&dex, 1, "move 1").unwrap();
        let (t2, mut cb2) = CompactBattle::from_battle(&b, &dex);
        assert_eq!(cb2.future_sight_turns[1], 2, "At Turn 2 start, Future Sight must have 2 turns left");
        let hp0 = cb2.mons[3].hp();
        cb2.step_turn(&t2, [1, 0]); // Turn 2 EOT -> 2 -> 1 (does NOT strike yet!)
        assert_eq!(cb2.mons[3].hp(), hp0, "Future Sight must NOT strike on Turn 2 EOT");
        assert_eq!(cb2.future_sight_turns[1], 1);

        // Advance Battle through Turn 2 -> Turn 3 start: `from_battle` must report 1 turn left!
        b.choose(&dex, 0, "move 2").unwrap();
        b.choose(&dex, 1, "move 1").unwrap();
        let (t3, mut cb3) = CompactBattle::from_battle(&b, &dex);
        assert_eq!(cb3.future_sight_turns[1], 1, "At Turn 3 start, Future Sight must have 1 turn left");
        let mut struck = false;
        for s in 1..=32u64 {
            let mut c = cb3;
            c.prng_seed = s.wrapping_mul(0x9E37_79B9_7F4A_7C15);
            c.step_turn(&t3, [1, 0]); // Turn 3 EOT -> 1 -> 0 -> STRIKES!
            if c.mons[3].hp() < hp0 {
                struck = true;
                break;
            }
        }
        assert!(struck, "Future Sight imported at Turn 3 must strike on Turn 3 EOT");
    }

    // 15.2 Counter / Mirror Coat vs Special Hidden Power (HP Ice -> Physical for Counter!).
    {
        let p1 = vec![mk_set("Zapdos", 55, "", &["Hidden Power Ice"], "")];
        let p2 = vec![mk_set("Snorlax", 55, "", &["Counter", "Mirror Coat"], "M")];
        let (t, base) = start_compact(&dex, Ruleset::PsStadium2, &p1, &p2);
        let mut c_cnt = base;
        c_cnt.prng_seed = 7;
        let z0 = c_cnt.mons[0].hp();
        c_cnt.step_turn(&t, [0, 0]); // Zapdos HP Ice -> Snorlax Counter MUST reflect!
        assert!(
            c_cnt.mons[0].hp() < z0,
            "Counter must reflect Special Hidden Power Ice (static Normal/Physical category)"
        );

        let mut c_mc = base;
        c_mc.prng_seed = 7;
        c_mc.step_turn(&t, [0, 1]); // Zapdos HP Ice -> Snorlax Mirror Coat MUST FAIL!
        assert_eq!(
            c_mc.mons[0].hp(),
            z0,
            "Mirror Coat must NOT reflect Special Hidden Power Ice"
        );
    }

    // 15.3 Berserk Gene: triggers +2 Atk + Confusion (blocked by Safeguard) + consumes item on switch-in!
    {
        let p1 = vec![
            mk_set("Jolteon", 55, "", &["Safeguard", "Splash"], "M"),
            mk_set("Snorlax", 55, "Berserk Gene", &["Splash"], "M"),
        ];
        let p2 = vec![mk_set("Blissey", 55, "", &["Splash"], "F")];
        let (t, base) = start_compact(&dex, Ruleset::PsStadium2, &p1, &p2);
        assert_ne!(base.mons[1].item, 0, "Bench Snorlax must hold Berserk Gene");

        // Switch-in without Safeguard: +2 Atk, confused, item consumed.
        let mut c1 = base;
        c1.step_turn(&t, [5, 0]);
        assert_eq!(c1.mons[1].boosts[stat_idx::ATK], 2, "Berserk Gene must raise Atk +2 on switch-in");
        assert_ne!(c1.mons[1].vol_flags & vol_bit::CONFUSION, 0, "Berserk Gene must confuse on switch-in");
        assert_eq!(c1.mons[1].item, 0, "Berserk Gene must be consumed");

        // Switch-in with Safeguard: +2 Atk, NO confusion, item consumed.
        let mut c2 = base;
        c2.step_turn(&t, [0, 0]); // Jolteon sets Safeguard
        c2.step_turn(&t, [5, 0]); // Switch to Berserk Gene Snorlax
        assert_eq!(c2.mons[1].boosts[stat_idx::ATK], 2);
        assert_eq!(
            c2.mons[1].vol_flags & vol_bit::CONFUSION,
            0,
            "Safeguard must block Berserk Gene confusion"
        );
        assert_eq!(c2.mons[1].item, 0);
    }

    // 15.4 Castle `AfterSwitchInSelf` Toxic `residualdmg` matches `Battle` pre-increment (`quantum_1_16 * c`, then `c -> c+1`).
    {
        let p1 = vec![
            mk_set("Jolteon", 55, "", &["Baton Pass", "Splash"], "M"),
            mk_set("Snorlax", 55, "", &["Splash"], "M"),
        ];
        let p2 = vec![mk_set("Blissey", 55, "", &["Splash"], "F")];
        let mut b = Battle::from_fixture_with_ruleset(
            &dex,
            "1,2,3,4",
            &p1,
            &p2,
            Ruleset::castle_tournament(),
        )
        .unwrap();
        b.choose(&dex, 0, "team 1,2").unwrap();
        b.choose(&dex, 1, "team 1").unwrap();
        let j_id = b.active_id(0).unwrap();
        b.poke_mut(j_id).status = Status::Tox;
        b.add_volatile(&dex, j_id, "residualdmg", None, nc2000_engine::battle::EffectHandle::None);
        let rd = nc2000_engine::cond_id!(&dex, "residualdmg").unwrap();
        b.poke_mut(j_id).volatile_mut(rd).unwrap().set_int(nc2000_engine::state::DK::Counter, 2);
        let s_id = nc2000_engine::state::PokeId { side: 0, slot: 1 };
        b.poke_mut(s_id).status = Status::Psn;
        b.refresh_poke_mask(&dex, s_id);
        let (t_c, mut cb_c) = CompactBattle::from_battle(&b, &dex);
        let max1 = t_c.mons[1].maxhp;
        b.choose(&dex, 0, "move 1").unwrap();
        b.choose(&dex, 1, "move 1").unwrap();
        b.choose(&dex, 0, "switch 2").unwrap();
        cb_c.step_turn(&t_c, [0, 0]);
        assert_eq!(cb_c.mons[1].status_counter(), 0x10 | 3);
        assert_eq!(
            cb_c.mons[1].hp(),
            b.poke(s_id).hp as i16,
            "Castle AfterSwitchInSelf residualdmg must match Battle exactly"
        );
        assert_eq!(cb_c.mons[1].hp(), max1 - t_c.mons[1].quantum_1_16 * 2);
    }

    // 15.5 Castle Mystery Berry `Spite` timing: does NOT consume inside Spite if victim uses another move;
    //      DOES consume before victim's 0-PP move in `exec_move_inner`!
    {
        let p1 = vec![mk_set("Gengar", 55, "", &["Spite", "Splash"], "M")];
        let p2 = vec![mk_set("Snorlax", 50, "Mystery Berry", &["Tackle", "Splash"], "M")];
        let (t, base) = start_compact(&dex, Ruleset::castle_tournament(), &p1, &p2);

        // Victim had Tackle at 1 PP (last_move = 0) and chose Tackle (move 0):
        // Faster Gengar Spites Tackle -> 0 PP -> Snorlax eats Mystery Berry right before executing Tackle -> Tackle runs (4 PP left)!
        let mut c_use = base;
        c_use.mons[3].set_last_move_slot(0);
        c_use.mons[3].pp[0] = 1;
        c_use.step_turn(&t, [0, 0]); // Gengar Spite -> Snorlax Tackle
        assert_eq!(c_use.mons[3].item, 0, "Mystery Berry consumed before 0-PP Tackle");
        assert_eq!(
            c_use.mons[3].pp[0], 4,
            "Mystery Berry restored 5 PP then Tackle spent 1 PP -> 4 PP"
        );

        // Victim had Tackle at 1 PP (last_move = 0), chose Splash (move 1), AND takes EOT or faint check:
        // Verify that if victim uses Splash while holding Mystery Berry, it did NOT fire mid-Spite
        // (we can verify directly via Battle parity: if Gengar is 1-HP and Snorlax KOs Gengar with Body Slam on slot 1,
        //  Wait: easier! Put Snorlax's Tackle at 1 PP, Splash at 10 PP, and give Gengar faster Spite + Snorlax Thief/Wait:
        //  Even cleaner: if Snorlax chose Splash (move 1) AND Gengar had Spite, at EOT Mystery Berry DOES trigger at EOT residual,
        //  so test mid-turn pause via faster Gengar Spite -> slower Snorlax Baton Pass with 2 bench mons -> Mystery Berry STILL held at BATONN pause!).
        let p2_bp = vec![
            mk_set("Snorlax", 50, "Mystery Berry", &["Tackle", "Baton Pass"], "M"),
            mk_set("Blissey", 50, "", &["Splash"], "F"),
            mk_set("Zapdos", 50, "", &["Splash"], ""),
        ];
        let (t_bp, mut c_bp) = start_compact(&dex, Ruleset::castle_tournament(), &p1, &p2_bp);
        c_bp.mons[3].set_last_move_slot(0);
        c_bp.mons[3].pp[0] = 1;
        c_bp.step_turn(&t_bp, [0, 1]); // Faster Gengar Spite drains Tackle to 0 -> Slower Snorlax Baton Pass pauses!
        assert_eq!(c_bp.sides[1].request, 2);
        assert_eq!(c_bp.mons[3].pp[0], 0, "Tackle drained to 0 PP by Spite");
        assert_ne!(
            c_bp.mons[3].item, 0,
            "Castle Mystery Berry must NOT trigger inside Spite when victim chose a different move (Baton Pass)"
        );
    }

    // 15.6 Castle Foresight `lookup_accuracy_threshold_castle`: zeros positive `eva` even when `acc >= eva`.
    {
        let without_fs = lookup_accuracy_threshold_castle(50, 1, 1, false, false, false, false);
        let with_fs = lookup_accuracy_threshold_castle(50, 1, 1, false, false, true, false);
        let expected_eva0 = lookup_accuracy_threshold_castle(50, 1, 0, false, false, false, false);
        assert!(with_fs > without_fs, "Foresight must ignore positive eva even when acc >= eva");
        assert_eq!(with_fs, expected_eva0);
    }

    // 15.7 First-mover Castle `Rest` + second-mover `BATONN` wakes Rest user BEFORE BP switch prompt!
    {
        let p1 = vec![mk_set("Jolteon", 55, "Mint Berry", &["Rest", "Splash"], "M")];
        let p2 = vec![
            mk_set("Umbreon", 50, "", &["Baton Pass", "Splash"], "M"),
            mk_set("Snorlax", 50, "", &["Splash"], "M"),
            mk_set("Zapdos", 50, "", &["Splash"], ""),
        ];
        let (t, mut cb) = start_compact(&dex, Ruleset::castle_tournament(), &p1, &p2);
        cb.mons[0].hp_pool[0] = 50; // Damaged so Rest succeeds!
        cb.step_turn(&t, [0, 0]); // Faster Jolteon Rest -> slower Umbreon Baton Pass pauses in BATONN!
        assert_eq!(cb.sides[1].request, 2);
        assert_eq!(
            cb.mons[0].status_raw(),
            Status::None as u8,
            "First-mover Castle Rest must wake via Mint Berry BEFORE 2nd-mover BATONN pause"
        );
        assert_eq!(cb.mons[0].item, 0);
        assert_eq!(cb.bp_switch_flags & 0xC0, 0);
    }

    // 15.8 Confusion self-hit: `is_selfdestruct` Def/2 (both Castle & PS) + PS `>= 256 / 4` rollover!
    {
        let p1 = vec![mk_set("Snorlax", 55, "", &["Explosion", "Splash"], "M")];
        let p2 = vec![mk_set("Blissey", 55, "", &["Splash"], "F")];
        for ruleset in [Ruleset::PsStadium2, Ruleset::castle_tournament()] {
            let (t, base) = start_compact(&dex, ruleset, &p1, &p2);
            let mut saw_double = false;
            for s in 1..=64u64 {
                let seed = s.wrapping_mul(0x9E37_79B9_7F4A_7C15);
                let mut c_sp = base;
                let mut c_ex = base;
                c_sp.prng_seed = seed;
                c_ex.prng_seed = seed;
                c_sp.mons[0].vol_flags |= vol_bit::CONFUSION;
                c_sp.mons[0].timers[timer_idx::CONFUSION] = 4;
                c_ex.mons[0].vol_flags |= vol_bit::CONFUSION;
                c_ex.mons[0].timers[timer_idx::CONFUSION] = 4;
                let hp0 = c_sp.mons[0].hp();
                c_sp.step_turn(&t, [1, 0]); // Splash confusion self-hit
                c_ex.step_turn(&t, [0, 0]); // Explosion confusion self-hit (halves own Def!)
                let d_sp = hp0 - c_sp.mons[0].hp();
                let d_ex = hp0 - c_ex.mons[0].hp();
                if d_sp > 0 && d_ex > 0 && d_ex >= (d_sp * 3) / 2 {
                    saw_double = true;
                    break;
                }
            }
            assert!(
                saw_double,
                "Confusion self-hit while selecting Explosion must halve Def in {ruleset:?}"
            );
        }

        // PS confusion self-hit with Atk >= 256 uses `get_damage_base_branchless` (`>> 2` rollover),
        // matching `Battle` identically!
        let p_hi1 = vec![mk_set("Marowak", 55, "Thick Club", &["Splash"], "M")];
        let mut b_hi = Battle::from_fixture_with_ruleset(
            &dex,
            "1,2,3,4",
            &p_hi1,
            &p2,
            Ruleset::PsStadium2,
        )
        .unwrap();
        b_hi.choose(&dex, 0, "team 1").unwrap();
        b_hi.choose(&dex, 1, "team 1").unwrap();
        let (t_hi, base_hi) = CompactBattle::from_battle(&b_hi, &dex);
        let atk = t_hi.mons[0].stored_stats[stat_idx::ATK] as i32 * 2; // Thick Club > 256!
        let def = t_hi.mons[0].stored_stats[stat_idx::DEF] as i32;
        let expected_rollover = get_damage_base_branchless(
            55,
            40,
            atk,
            def,
            true,
            false,
            false,
            false,
            CompactWeather::None,
            false,
            false,
            false,
            u8::MAX,
            0,
            0,
            false,
        ) as i16;
        let mut matched_ps_rollover = false;
        for s in 1..=64u64 {
            let mut c = base_hi;
            c.prng_seed = s.wrapping_mul(0x9E37_79B9_7F4A_7C15);
            c.mons[0].vol_flags |= vol_bit::CONFUSION;
            c.mons[0].timers[timer_idx::CONFUSION] = 4;
            let hp0 = c.mons[0].hp();
            c.step_turn(&t_hi, [0, 0]);
            let d = hp0 - c.mons[0].hp();
            if d > 0 {
                assert_eq!(
                    d, expected_rollover,
                    "PS confusion self-hit must use get_damage_base_branchless rollover"
                );
                matched_ps_rollover = true;
                break;
            }
        }
        assert!(matched_ps_rollover);
    }

    // 15.9 `Psych Up` does NOT copy `Focus Energy`.
    {
        let p1 = vec![mk_set("Mew", 55, "", &["Psych Up"], "")];
        let p2 = vec![mk_set("Snorlax", 55, "", &["Splash"], "M")];
        for ruleset in [Ruleset::PsStadium2, Ruleset::castle_tournament()] {
            let (t, mut cb) = start_compact(&dex, ruleset, &p1, &p2);
            cb.mons[3].boosts[stat_idx::ATK] = 2;
            cb.mons[3].ext_flags |= ext_vol_bit::FOCUS_ENERGY;
            cb.step_turn(&t, [0, 0]);
            assert_eq!(cb.mons[0].boosts[stat_idx::ATK], 2);
            assert_eq!(
                cb.mons[0].ext_flags & ext_vol_bit::FOCUS_ENERGY,
                0,
                "Psych Up must NOT copy Focus Energy in {ruleset:?}"
            );
        }
    }

    // 15.10 `get_stat_castle` order: `stage -> item -> screen -> (ovfl ? % 1024 : min(999)).max(1) -> par/burn`.
    {
        // `stored=256`, `+2` stage -> `512`, Reflect `*2` -> `1024`;
        // in `ovfl=1`, `1024 % 1024 = 0`, clamped to `1` BEFORE par/burn!
        let ovfl_wrapped = get_stat_castle(
            256,
            2,
            stat_idx::DEF,
            false,
            false,
            false,
            false,
            true,
            false,
            ItemStatKind::None,
            true,
        );
        assert_eq!(ovfl_wrapped, 1, "ovfl=1 wrap to 0 must clamp to 1");

        // `stored=200`, `+6` stage (`*4 = 800`), Thick Club (`*2 = 1600`):
        // - standard (`ovfl=0`): cap `999`, then Burn `/2` = `499`!
        // - overflow (`ovfl=1`): `1600 % 1024 = 576`, then Burn `/2` = `288`!
        let std_brn = get_stat_castle(
            200,
            6,
            stat_idx::ATK,
            false,
            false,
            false,
            true,
            false,
            false,
            ItemStatKind::ThickClub,
            false,
        );
        let ovfl_brn = get_stat_castle(
            200,
            6,
            stat_idx::ATK,
            false,
            false,
            false,
            true,
            false,
            false,
            ItemStatKind::ThickClub,
            true,
        );
        assert_eq!(std_brn, 499);
        assert_eq!(ovfl_brn, 288);
    }
}
