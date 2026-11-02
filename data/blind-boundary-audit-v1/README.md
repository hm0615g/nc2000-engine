# Blind information boundary audit — 2026-11-03

The HP suspicion was confirmed for the audited baseline's ordinary Web play and
evaluation path. Opponent **absolute current HP survives determinization** and
can change the selected action at production budgets. Sleep counters and
pre-determinization action exclusions expose additional private state.

Baseline: `07bba87fd22f4fb0993a044326d2cfbac69405cb`. This audit adds a
reproducer and evidence only. It changes no production source, settings,
preview artifacts or deployment. The hosted binary was not inspected; the
claim concerns this checkout running the production configuration.

The subsequent [repair report](../blind-boundary-repair-v1/README.md) records the
implementation and new verification. The reproducers below assert historical
vulnerabilities and are not acceptance tests for the repaired product.

## Reproduction and independent observations

From the repository root:

```sh
cargo run --release -p nc2000-bot --example audit_blind_boundary -- target/blind-boundary.json
cmp target/blind-boundary.json data/blind-boundary-audit-v1/results.json
cargo test --release -j 2 -p nc2000-bot --test blind --test player --test claw_information
```

The reproducer reads the active catalog, opponent prior, exploration constant
and both live battle budgets. It uses `SelRule::Ucb`, 16 search-key HP buckets
and all other `RmConfig` defaults, matching the WASM `skuct_config` used by
`BlindSearcher` and `ProtocolSearcher`. It exercises the native implementation
behind those bindings, rather than running a browser or the WASM artifact.

The comparison oracle is `PlayerChannel`'s complete player frame: filtered
protocol lines, the observing player's exact request, and legal actions. It
does **not** use the suspect determinizer to decide whether observations
match. Within each pair, the public frame, search seed and iteration budget
are fixed. Results include full action counts and values, model snapshots,
public frames, and source/data hashes.

The full reproducer was run twice with byte-identical JSON. The 13 existing
tests in `blind`, `player` and `claw_information` all pass. See
[verification.json](verification.json) for the record and native executable
hashes. The reproducer deliberately asserts the known violations; its passing
is evidence that they were reproduced, not that privacy was repaired.

These are constructed snapshot interventions. They are not a demonstrated
pair of complete reachable game histories, an incident replay, or a sample
of naturally occurring positions. The HP snapshots use the actual six-member
catalog/prior teams with ordinary three-member selection. Sleep, item and
coin probes use small fixtures to isolate the relevant field.

## Findings

### 1. Exact opponent HP affects decisions

The opponent Gengar has 166 maximum HP. Both 164 and 165 HP are announced as
`99/100`; both also have the same rounded Web label, `99%`. The complete
player frames are equal. Nevertheless, the direct path preserves 164 and
165 as different search HP values.

| Bot side | Iterations | At 164 HP | At 165 HP |
| --- | ---: | --- | --- |
| P1 | 27,000 | Thunderbolt | Thunderbolt |
| P1 | 81,000 | Reflect | Thunderbolt |
| P2 (ordinary Web bot side) | 27,000 | Reflect | Thunderbolt |
| P2 | 81,000 | Reflect | Thunderbolt |

All four comparisons change search values. Feeding the same frames through
`ProtocolAgent` produces identical synthesized HP, visit counts, values and
selected actions in every pair. A separate positive control changes the
public HP to 50%; the protocol path then changes its model and values, so
the equality is not explained by ignoring HP.

The cause is [impute_mon](../../crates/bot/src/belief.rs): `hp` is explicitly
classified as a public amount; only `maxhp` is replaced. An appeared mon
keeps `min(actual_hp, candidate_maxhp)`. Search-key HP buckets aggregate
transpositions; they do not remove exact HP from simulated battle mechanics.
The preview cache also does not sanitize subsequent battle searches.

### 2. Replacing maximum HP while retaining current HP contradicts public HP

Using zero-DV variants of each catalog/prior lead as hidden truth, the audit
checks all integral current HP values: 46,498 of 50,465 constructed trials
produce a modeled percentage different from the public one. All 254 tested
full-health leads are modeled below full health. These are stress-test
counts, not an estimate of real-game frequency.

For example, an Electrode at **151/151 (public 100%)** becomes **151/166
(modeled 91%)**. This both exposes the hidden true maximum through the
retained current amount and corrupts the bot's model. Simply dividing by
the candidate maximum after copying absolute HP cannot repair this.

The protocol path avoids reading true opponent HP, but its synthesis chooses
one reference maximum before the shared determinizer replaces candidates.
The follow-up below confirms that this can also contradict public HP on the
PS path, without reading any true opponent secrets.

### 3. Hidden sleep counters survive into search

`impute_mon` keeps `status_state` and explicitly calls hidden sleep counters
a non-goal. With identical player frames and non-Rest sleep counters of 1
versus 3, the 81,000-iteration runs retain the two true counters and produce
different values and visit counts. The selected action remains Thunderbolt
in this fixture. Protocol-only controls match exactly. Rest's publicly known
clock is a different case and is not evidence of a leak.

Other hidden volatile durations and Substitute HP are also preserved by the
same implementation. Their behavior was not separately tested here.

### 4. Root exclusions read private state before it is overwritten

[BlindSearch::with_rng](../../crates/bot/src/blind.rs) computes
`my_dominated` from the cloned true battle **before** the belief overwrites
private fields. [faster_than_foe](../../crates/bot/src/smmcts.rs) reads actual
items, action speed and the current Quick Claw coin.

Two independent 81,000-iteration probes have equal complete player frames:

- Changing only the observing side's unannounced Quick Claw coin changes
  Rest's final-selection exclusion from false to true.
- Changing the opponent's unrevealed held item from Leftovers to Quick Claw
  changes Recover's exclusion from true to false. Item presence is identical,
  and the sampled model overwrites both items with the same identity. The
  protocol control is invariant.

These fixtures retain identical raw search counts/values and choose the same
final action; the demonstrated leak is **different eligibility for final
selection**, not an observed action flip in these two probes. `best()` uses
these cached exclusions. Replacing private state later in each search
iteration does not undo the earlier read. `unseen_claw_root` defaults to
false; enabling it alone would not fix the unrevealed-item or HP problems.

## PS follow-up: public inputs do not make every exclusion sound

`ProtocolAgent::install` synthesizes the search root from the **first alive
candidate**, then constructs the shared `BlindSearch`. Consequently its
cached final-selection exclusions can rely on one candidate's item or speed,
even while the belief retains another candidate under which the exclusion
does not hold. PS request legality projection only prevents illegal
submissions; it does not restore legal moves excluded by this heuristic.

The additional [reproducer](../../crates/bot/examples/audit_protocol_exclusions.rs)
uses the production configuration at 81,000 iterations. It runs two
constructed one-mon fixtures with identical player frames and equal-weight
two-candidate priors. Reversing candidate order preserves the distribution:

| Hidden hypothesis | First candidate | Recover excluded? |
| --- | --- | --- |
| Snorlax item | Leftovers | Yes |
| Snorlax item | Quick Claw | No |
| Opposing Starmie speed | 135 (own speed 151) | Yes |
| Opposing Starmie speed | 166 (own speed 151) | No |

Both candidates remain alive in every run. Recover is a request-legal move.
The final choice is Surf in all four runs, so these fixtures demonstrate
unsupported hard exclusions, not a final-action flip or a measured loss.
The unknown true world is never supplied to the protocol searcher. This is
a modeling/decision problem, distinct from the Web path's private-truth
exposure. These custom two-candidate priors are counterexamples to the generic
exclusion rule, not a frequency estimate for the active production prior.

A third probe supplies public opponent HP of `50/100` and two equally weighted
Snorlax candidates. Synthesis uses the first candidate's 266 maximum HP and
sets current HP to 132. Determinizing with the other candidate replaces the
maximum with 235, retaining 132 current HP and announcing 57%, contradicting
the observed 50%. The first projection is correct; the subsequent shared
imputation is not. This confirms a PS-side model error without true-HP access.

Run from the repository root:

```sh
cargo run --release -p nc2000-bot --example audit_protocol_exclusions -- target/protocol-exclusions.json
cmp target/protocol-exclusions.json data/blind-boundary-audit-v1/protocol-exclusions.json
cargo test --release -p nc2000-bot --test import
```

[protocol-exclusions.json](protocol-exclusions.json) records the source/data
hashes, public frames, synthesized roots, exclusions, action counts and values.
The `pinOpponent` and hand-entered-position APIs are not used in these probes.
The complete follow-up ran twice with byte-identical JSON; both existing
`import` tests passed. See [protocol-verification.json](protocol-verification.json).

## Affected paths and rejected overclaims

| Path | Input to search | Result of this audit |
| --- | --- | --- |
| Ordinary `web/src/bot-worker.ts` | Full mirror `Battle` → `BlindSearcher` | Affected |
| `web/src/evaluate-worker.ts` | Full `Battle` → `BlindSearcher` for each side | Affected |
| `tools/ps-client.js` / `tools/ps-arena.js` | Player protocol → `ProtocolSearcher` | These private-HP interventions are invariant |
| Worker's optional endgame branch | `PlayerChannel` → `ProtocolSearcher` | Uses the protocol boundary; not the default ordinary-play branch |

Exact **own** HP is legitimate: it is in the player's request. A search model
using absolute integers is also legitimate when those integers are imputed
from public HP and a sampled maximum. The issue is retaining the opponent's
true integers and other private state.

The Web renderer is not a security boundary: `stateView` includes true state,
and the worker receives both true teams, the seed and applied choices. The
numeric opponent HP label is rounded, but the HP bar uses an unrounded true
fraction. Thus equality of protocol frames and numeric labels is not a claim
that the current full rendered DOM is identical. This audit investigates
unfair bot information, not external transmission of user data.

The existing `blind` tests define their agreement region through
determinized-state equality and stop comparing when those states differ.
That can exclude the exact HP/counter differences under investigation.
Passing those tests is therefore not a proof of this information boundary.

## Repair direction

Use an independent player-protocol boundary for ordinary play and evaluation,
then audit all consumers of the synthesized state, including final action
exclusions and any early-return rules. Preserve the announced HP numerator
and denominator and reconstruct HP for each sampled candidate maximum.
Sample hidden counters from public history rather than retaining the true
rolls. Keep observable-frame comparisons independent of determinization and
require invariant decisions for fixed observations, settings and randomness.

These are proposed repairs; no repair or publication is part of this audit.
The [repair scope](../../docs/BLIND-BOUNDARY-REPAIR-SCOPE.md) separates runtime
changes, required verification, preview compatibility and equilibrium remeasurement.
