//! Regression: the bot's threat/eval must track the gen-2 accuracy×evasion
//! stage so a Double-Team / Baton-Pass evasion wall registers as danger (and
//! phazing as relief) instead of being invisible.

use nc2000_bot::eval::{self, best_hit_fraction, expected_hit_fraction};
use nc2000_bot::mcts::Playout;
use nc2000_bot::smmcts::SelRule;
use nc2000_bot::{Agent, EvalWeights, RmAgent, RmConfig};
use nc2000_engine::battle::{PokemonSet, SearchChoice};
use nc2000_engine::dex::{toid, Dex, MoveId};
use nc2000_engine::state::Battle;

fn set(json: &str) -> PokemonSet {
    serde_json::from_str(json).unwrap()
}

/// Crobat (bot lead) carries a phazer (Whirlwind) + attacks; opponent Snorlax
/// stands in for the Baton-Pass recipient whose evasion we stack.
fn crobat_phazer() -> PokemonSet {
    set(r#"{"name":"Crobat","species":"Crobat","item":"Leftovers","ability":"No Ability","moves":["Whirlwind","Sludge Bomb","Wing Attack","Swift"],"level":50,"evs":{"hp":255,"atk":255,"def":255,"spa":255,"spd":255,"spe":255},"ivs":{"hp":31,"atk":31,"def":31,"spa":31,"spd":31,"spe":31},"gender":"M"}"#)
}
fn snorlax() -> PokemonSet {
    set(r#"{"name":"Snorlax","species":"Snorlax","item":"Leftovers","ability":"No Ability","moves":["Body Slam","Curse","Rest","Earthquake"],"level":50,"evs":{"hp":255,"atk":255,"def":255,"spa":255,"spd":255,"spe":255},"ivs":{"hp":31,"atk":31,"def":31,"spa":31,"spd":31,"spe":31},"gender":"M"}"#)
}
fn porygon2() -> PokemonSet {
    set(r#"{"name":"Porygon2","species":"Porygon2","item":"Mint Berry","ability":"No Ability","moves":["Recover","Thunderbolt","Ice Beam","Curse"],"level":50,"evs":{"hp":255,"atk":255,"def":255,"spa":255,"spd":255,"spe":255},"ivs":{"hp":31,"atk":31,"def":31,"spa":31,"spd":31,"spe":31},"gender":"N"}"#)
}
fn marowak() -> PokemonSet {
    set(r#"{"name":"Marowak","species":"Marowak","item":"Thick Club","ability":"No Ability","moves":["Earthquake","Rock Slide","Hidden Power Bug","Swords Dance"],"level":50,"evs":{"hp":255,"atk":255,"def":255,"spa":255,"spd":255,"spe":255},"ivs":{"hp":30,"atk":26,"def":26,"spa":30,"spd":30,"spe":30},"gender":"M"}"#)
}

fn start(dex: &Dex, p1: &[PokemonSet], p2: &[PokemonSet]) -> Battle {
    let mut b = Battle::from_fixture(dex, "1,2,3,4", p1, p2).unwrap();
    b.set_log_enabled(false);
    b.choose(dex, 0, "team 1,2,3").unwrap();
    b.choose(dex, 1, "team 1,2,3").unwrap();
    b
}

fn mid(dex: &Dex, name: &str) -> MoveId {
    dex.moves.id(&toid(name)).unwrap()
}

/// The evasion-wall position: bot Crobat active, opponent Snorlax active with
/// `eva`/`atk` boost stages (a Double-Team + Baton-Passed sweeper). 3v3 so the
/// eval is not material-saturated.
fn wall(dex: &Dex, eva: i8, atk: i8) -> Battle {
    let bot = vec![crobat_phazer(), porygon2(), marowak()];
    let opp = vec![snorlax(), porygon2(), marowak()];
    let mut b = start(dex, &bot, &opp);
    let snor = b.active_id(1).unwrap();
    b.poke_mut(snor).boosts[6] = eva;
    b.poke_mut(snor).boosts[0] = atk;
    b
}

// ---- 1. the engine accessor applies the gen-2 evasion stage --------------

#[test]
fn hit_probability_applies_evasion_stage() {
    let dex = super::common::dex();
    let b = wall(&dex, 0, 0);
    let cro = b.active_id(0).unwrap();
    let snor = b.active_id(1).unwrap();
    let sludge = mid(&dex, "sludgebomb"); // 100% base accuracy
    let swift = mid(&dex, "swift"); // never-miss

    // At 0 evasion a 100%-accuracy move never misses; a never-miss move is 1.0.
    let mut z = b.clone();
    z.poke_mut(snor).boosts[6] = 0;
    assert!((z.hit_probability(&dex, cro, snor, sludge) - 1.0).abs() < 1e-9);
    assert!((z.hit_probability(&dex, cro, snor, swift) - 1.0).abs() < 1e-9);

    // Hit chance falls monotonically as evasion climbs, reaching ~84/256 at +6.
    let mut prev = 1.01;
    for eva in [0i8, 2, 4, 6] {
        let mut e = b.clone();
        e.poke_mut(snor).boosts[6] = eva;
        let p = e.hit_probability(&dex, cro, snor, sludge);
        assert!(p < prev, "hit prob must fall with evasion (eva={eva})");
        prev = p;
    }
    let mut e6 = b.clone();
    e6.poke_mut(snor).boosts[6] = 6;
    assert!(
        (e6.hit_probability(&dex, cro, snor, sludge) - (84.0 / 256.0)).abs() < 5e-3,
        "gen-2 +6 evasion collapses a 100% move to 84/256"
    );
    // Never-miss ignores evasion entirely.
    assert!((e6.hit_probability(&dex, cro, snor, swift) - 1.0).abs() < 1e-9);
}

// ---- 2. the defect + contrast: the threat feature was blind to evasion ----

#[test]
fn threat_feature_collapses_under_evasion() {
    let dex = super::common::dex();
    let b0 = wall(&dex, 0, 4);
    let b2 = wall(&dex, 2, 4);
    let b6 = wall(&dex, 6, 4);
    let (cro0, snor0) = (b0.active_id(0).unwrap(), b0.active_id(1).unwrap());
    let (cro2, snor2) = (b2.active_id(0).unwrap(), b2.active_id(1).unwrap());
    let (cro6, snor6) = (b6.active_id(0).unwrap(), b6.active_id(1).unwrap());

    let h0 = best_hit_fraction(&b0, &dex, cro0, snor0);
    let h2 = best_hit_fraction(&b2, &dex, cro2, snor2);
    let h6 = best_hit_fraction(&b6, &dex, cro6, snor6);
    assert!(h0 > h2 && h2 > h6, "threat must decrease monotonically: {h0} > {h2} > {h6}");
    assert!(h6 < 0.5 * h0, "threat must collapse vs +6 evasion ({h6} vs {h0})");
}

#[test]
fn rollout_policy_stops_throwing_attacks_at_the_wall() {
    let dex = super::common::dex();
    let b = wall(&dex, 6, 4);
    let cro = b.active_id(0).unwrap();
    let snor = b.active_id(1).unwrap();
    let sludge = mid(&dex, "sludgebomb");
    let swift = mid(&dex, "swift");

    let sludge_h = expected_hit_fraction(&b, &dex, cro, snor, sludge);
    let swift_h = expected_hit_fraction(&b, &dex, cro, snor, swift);
    assert!(swift_h > sludge_h, "never-miss must outrank whiffing STAB at +6 evasion");
}

// ---- 3. the eval feels danger, and phazing value emerges -----------------

#[test]
fn coupled_eval_feels_danger_and_rewards_phazing() {
    let dex = super::common::dex();
    let w = EvalWeights::default();
    let b0 = wall(&dex, 0, 2);
    let b2 = wall(&dex, 2, 2);
    let b6 = wall(&dex, 6, 2);
    let e0 = eval::eval01(&b0, &dex, &w);
    let e2 = eval::eval01(&b2, &dex, &w);
    let e6 = eval::eval01(&b6, &dex, &w);
    assert!(e0 > e2 && e2 > e6, "eval must fall as foe evasion rises: {e0} > {e2} > {e6}");

    let mut reset = b6.clone();
    let snor = reset.active_id(1).unwrap();
    reset.poke_mut(snor).boosts[6] = 0;
    let recover = eval::eval01(&reset, &dex, &w) - e6;
    assert!(recover > 0.05, "phazing must produce a real eval jump ({recover})");
}

// ---- 4. the search discovers phazing instead of dithering ----------------

fn skuct_pick(dex: &Dex, b: &Battle, iters: u32, seed: u64) -> SearchChoice {
    let choices = b.clone().legal_choices(dex, 0);
    let cfg = RmConfig {
        iterations: iters,
        rule: SelRule::Ucb,
        playout: Playout::Heavy { eps: 0.2, turns: 8, weights: EvalWeights::default() },
        ..Default::default()
    };
    RmAgent::new(cfg, seed).choose(b, dex, 0, &choices)
}

#[test]
fn fixed_search_phazes_the_evasion_wall() {
    // With the fix the bot, facing a +6-evasion Baton-Pass sweeper and holding
    // Whirlwind, resets the evasion rather than chipping into misses. Deter-
    // ministic per seed; asserted over several so it is not a lucky draw.
    let dex = super::common::dex();
    let whirlwind = mid(&dex, "whirlwind");
    let b = wall(&dex, 6, 4);
    let seeds = [1u64, 2, 3, 4, 5, 6, 7, 8, 42];
    let phazes = seeds
        .into_iter()
        .filter(|&s| skuct_pick(&dex, &b, 1500, s) == SearchChoice::Move(whirlwind))
        .count();
    assert!(
        phazes >= 6,
        "fixed search should phaze in at least 2/3 of seeds (got {phazes}/{})",
        seeds.len()
    );
}
