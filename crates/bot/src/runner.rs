//! Drives one battle between two agents through the search API.

use nc2000_engine::battle::{EngineError, Outcome};
use nc2000_engine::dex::Dex;
use nc2000_engine::rules::Ruleset;
use nc2000_engine::state::Battle;

use crate::agent::Agent;

/// `TurnCapped` = neither side won within the effective turn cap (scored as a
/// tie by PS callers; Castle rulesets with `turn_limit` adjudicate via
/// `judge_castle_turn_limit` once `battle.turn > limit`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GameResult {
    Outcome(Outcome),
    TurnCapped,
}

/// Effective maximum turn count honoring both caller safety cap `max_turns`
/// and `ruleset.turn_limit()` (e.g. `80` in Castle tournament, `1000` on PS).
#[inline]
pub fn effective_turn_cap(ruleset: Ruleset, max_turns: u16) -> u16 {
    match ruleset.turn_limit() {
        Some(limit) => max_turns.min(limit),
        None => max_turns,
    }
}

/// Castle `L_TURN` turn-limit judge (Probe 12A1–12A3):
/// 1. More surviving picked Pokemon wins.
/// 2. Higher sum of surviving `hp / maxhp` ratios (`f64`, no seat bias) wins.
/// 3. Exact tie -> `Outcome::Tie`.
pub fn judge_castle_turn_limit(battle: &Battle) -> Outcome {
    let score_side = |s: usize| -> (usize, f64) {
        let side = &battle.sides[s];
        let mut alive = 0usize;
        let mut ratio_sum = 0.0f64;
        if !side.party.is_empty() {
            for &slot in &side.party {
                if let Some(p) = side.roster.get(slot as usize) {
                    if !p.fainted && p.hp > 0 && p.maxhp > 0 {
                        alive += 1;
                        ratio_sum += p.hp as f64 / p.maxhp as f64;
                    }
                }
            }
        } else {
            for p in &side.roster {
                if !p.fainted && p.hp > 0 && p.maxhp > 0 {
                    alive += 1;
                    ratio_sum += p.hp as f64 / p.maxhp as f64;
                }
            }
        }
        (alive, ratio_sum)
    };
    let (a0, r0) = score_side(0);
    let (a1, r1) = score_side(1);
    if a0 != a1 {
        return if a0 > a1 { Outcome::P1Win } else { Outcome::P2Win };
    }
    let diff = r0 - r1;
    if diff.abs() <= 1e-12 {
        Outcome::Tie
    } else if diff > 0.0 {
        Outcome::P1Win
    } else {
        Outcome::P2Win
    }
}

pub fn play_game(
    dex: &Dex,
    battle: &mut Battle,
    agents: &mut [&mut dyn Agent; 2],
    max_turns: u16,
) -> Result<GameResult, EngineError> {
    let turn_cap = effective_turn_cap(battle.ruleset, max_turns);
    loop {
        if let Some(o) = battle.outcome() {
            return Ok(GameResult::Outcome(o));
        }
        if battle.turn > turn_cap {
            if battle.ruleset.is_castle()
                && battle.ruleset.turn_limit().is_some_and(|lim| battle.turn > lim)
            {
                return Ok(GameResult::Outcome(judge_castle_turn_limit(battle)));
            }
            return Ok(GameResult::TurnCapped);
        }
        // Both sides pick against the same state (PS request semantics),
        // then the choices are applied together.
        let mut picks = [None, None];
        for s in 0..2 {
            let cs = battle.legal_choices(dex, s);
            if !cs.is_empty() {
                picks[s] = Some(agents[s].choose(battle, dex, s, &cs));
            }
        }
        if picks == [None, None] {
            return Err(EngineError::InvalidChoice(
                "no side owes a choice but the battle has not ended".into(),
            ));
        }
        battle.apply_choices(dex, picks)?;
    }
}
