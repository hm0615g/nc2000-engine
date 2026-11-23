# Gym Leader's Castle (`PS.cgi`) Bot — Design Decisions

> **Status**: Design decision recorded (`2026-11-23`); **not yet implemented in code**. Update this document and the referenced modules together when implementing Castle bot defaults and unlimited-turn handling.

---

## Decision 1: Default Turn Limit = `0` (Truly Unlimited)

- **Default rule**: Unless a turn limit is explicitly specified (e.g., tournament/meetup `L_TURN = 80`, explicit CLI flag, or room configuration), the Castle bot starts with **`L_TURN = 0` (`turn_limit: None`, unlimited)**.
- **Semantics of `0`**: **`0` is truly unlimited** — the engine and bot MUST NOT synthesize any hidden rule-level hard turn cap (such as Showdown's 1000-turn auto-tie, a fallback 80-turn judge, or a 200/500/1000-turn cutoff).
- **Explicit nonzero `L_TURN`**: When a positive limit `N` is explicitly specified (standard Castle tournaments and weekly meetups use `N = 80`), `PS.cgi` adjudicates immediately after turn `N` finishes (`TURN-(N-1)` is the last command input; after turn `N`'s end-of-turn effects resolve, judge by alive Pokémon count -> sum of alive `hp / maxhp` ratios -> `DRAW`).

---

## Rationale & Real-World Evidence (`2026-11-23` Investigation)

1. **`PS.cgi` software default**: All 13 live PBS rooms (`C`–`O` on `psense.lib.net`) present `<input name="L_TURN" size="2" value="0">(０で制限無し）` and `<input name="TLIMIT" size="2" value="0">(０で制限無し）`. No room enforces a server-side turn limit; players enter `L_TURN` manually per match.
2. **Two-tier community practice** (verified across `1,234` real PBS logs + official Castle manuals and tournament records):
   - **Casual / free play (`野試合`, Discord rooms `C`–`H`)**: Default is `L_TURN = 0` (explicitly documented in *Gym Leaders' Castle Manual* §2.1). Across `711` unique logs in [`data/pbs-castle-logs/`](../data/pbs-castle-logs/README.md), `9` of `10` games reaching turn 80 continued past turn 80 (up to `134` turns) and finished by normal KO; the sole turn-80 adjudication was a 3rd Yokumonsai tournament match (`moth vs 117`).
   - **Tournaments & weekly meetups (`翼門祭`, `突発対戦会`, BO3 practice)**: Uniformly `80` turns / `80` seconds (`14 / 523` archived tournament/meetup games = `2.68%` hit automatic turn-80 adjudication, including `1` live `DRAW` scored as `0.5` wins each in Swiss). Even in tournaments, manual entry errors occasionally leave `L_TURN = 0` (e.g., 3rd Yokumonsai LS R3 `きっしー vs りず`, decided at turn `92` by mutual agreement).
3. **No evidence of a hidden `PS.cgi` hard cap**: Closed-source Perl 5 CGI stores turn count as a normal scalar integer (no 8-bit `255` overflow), live logs run past `134` turns without issue, and 25 years of community bug reports / Wiki / BBS records contain zero reports of forced termination under `L_TURN = 0`.
4. **Mechanics admit true infinite games under `L_TURN = 0`**: Gen 2 / PBS has no 50-move rule or Endless Battle Clause — switching costs 0 PP, PBS Struggle against Ghost-types deals 0 damage and 0 recoil, and Leftovers can out-heal Struggle recoil.

---

## Implementation Touchpoints (Deferred — Do Not Assume Done Yet)

When implementing this decision in code, audit and align every layer below:

1. **Default ruleset / CLI / adapter wiring ([`crates/engine/src/rules.rs`](../crates/engine/src/rules.rs), downstream Castle adapter)**:
   - Currently `CastleOverlayConfig::default()` and `Ruleset::parse_id("castle" | "castle-pbs" | "pbs" | "pbs-nc2000")` resolve to `CastleOverlayConfig::tournament()` (`turn_limit: Some(80)`).
   - Flip the bot/default Castle entry point so unspecified Castle play uses `turn_limit: None` (`CastleOverlayConfig::free()` / `L_TURN = 0`), requiring explicit opt-in (e.g., `castle-tournament` or `--turn-limit 80`) for `Some(80)`.
   - When the bot joins or creates a `PS.cgi` room, default `L_TURN` to `0` unless explicitly configured, and mirror any explicitly specified nonzero `L_TURN` into `CastleOverlayConfig::turn_limit`.
2. **Engine turn advancement ([`crates/engine/src/battle/turn.rs`](../crates/engine/src/battle/turn.rs), [`crates/engine/src/compact.rs`](../crates/engine/src/compact.rs))**:
   - Keep `turn_limit: None` strictly non-terminating (no 1000-turn PS tie, no Castle judge).
   - Audit `turn` counter (`u16`) handling (`saturating_add(1)` in `compact.rs` vs `Battle`) so saturating at `u16::MAX` never misbehaves in state hashing or replay codecs if a synthetic test runs arbitrarily long.
3. **Search, eval, and endgame solvers under `turn_limit: None` ([`crates/bot/src/`](../crates/bot/src/))**:
   - **Rule vs. search separation**: Never inject a fake game-rule turn limit into `eval.rs` (`turn_limit_outcome_value`, `compact_turn_limit_outcome`, `castle_turn_limit_proximity_diff` must stay inactive for `None`). Any finite rollout depth or self-play cap (`runner::effective_turn_cap` -> `GameResult::TurnCapped`) is strictly a search/evaluation horizon, not a battle adjudication rule.
   - **Cycle / non-termination safety**:
     - [`crates/bot/src/exact.rs`](../crates/bot/src/exact.rs) documents that plain memoized DFS relies on a finite `turn_limit` for acyclicity; under `turn_limit: None`, true infinite loops (switching cycles, Ghost-vs-Struggle 0-recoil, Leftovers-vs-Struggle) break that assumption and require explicit horizon/cycle handling (or disabling unbounded exact solves when `turn_limit.is_none()`).
     - [`crates/bot/src/stall.rs`](../crates/bot/src/stall.rs) currently maps `turn_limit_rank_base(None)` to `u16::MAX + 1`; verify that stall classification and monotone ranks remain sound when there is no terminal turn limit.
     - [`crates/bot/src/linear.rs`](../crates/bot/src/linear.rs), [`crates/bot/src/relay.rs`](../crates/bot/src/relay.rs), [`crates/bot/src/tempo.rs`](../crates/bot/src/tempo.rs), and [`crates/bot/src/smmcts.rs`](../crates/bot/src/smmcts.rs): verify no Castle path assumes `turn_limit.is_some()` or falls back to `80` / `1000`.
4. **Caching & replay compatibility ([`crates/engine/src/replay.rs`](../crates/engine/src/replay.rs), [`crates/bot/src/compiled_preview.rs`](../crates/bot/src/compiled_preview.rs))**:
   - `Ruleset::fingerprint_tag`, compact replay Castle trailer, and compiled preview keys already encode `turn_limit: None` separately from `Some(80)`; add regression coverage once the Castle bot default switches to `None`.
