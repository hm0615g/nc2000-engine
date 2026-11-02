# Scope of the blind-boundary repair

Scope review at `8c00c13e` (2026-11-03). This document describes required and
conditional work; it does not implement a repair, authorize deployment, or
claim that any existing policy is an equilibrium for a corrected bot.

Evidence: [boundary and protocol audits](../data/blind-boundary-audit-v1/README.md).

## Required implementation

| Area | Required change | Main locations |
| --- | --- | --- |
| Observation boundary | Provide own player-visible information and observed opponent facts to the bot. Stop sending the opponent's true team, battle RNG, full battle object and hidden choice sequence to the live search worker. Keep construction of trusted observations in a narrow adapter. | `player.rs`, `import.rs`, `observe.rs`, WASM bindings, `bot-worker.ts`, `evaluate-worker.ts` |
| Types and API | Distinguish observations, accumulated knowledge and sampled simulation worlds. Keep their constructors private and prevent a real `Battle` from being passed directly to the product search API. Separate open-sheet/research injection APIs from blind play. A wrapper in the bot layer can preserve the engine's battle representation. | `blind.rs`, `belief.rs`, `import.rs`, WASM bindings and their callers |
| HP reconstruction | Retain the announced numerator and denominator independently of any guessed maximum. Reconstruct HP for every sampled candidate maximum; preserve full/fainted semantics and both /100 and legacy /48 formats. | `import.rs`, `belief.rs`, observation/knowledge representation |
| Hidden runtime state | Generate hidden sleep/volatile counters, Substitute HP, hidden selection and pending actions from public history and independent model randomness. Cover both sides: own HP is visible, but own random sleep/confusion durations and the upcoming Quick Claw result are not automatically visible. | `import.rs`, `belief.rs`, search-world construction |
| Root exclusions | Evaluate claims about useless actions against public facts and all admitted uncertainty. Do not turn one guessed item, speed, duration or chance result into a permanent exclusion. Retain a move when the proof does not hold. Use the same eligibility in final selection, execution distributions and early-stop/endgame decisions. | `smmcts.rs`, `blind.rs`, `root_policy.rs`, `decision.rs`, `endgame.rs` |
| All product entry points | Apply the same observation and eligibility contract to Web play, evaluation, PS play and native measurement. Preserve historical open-sheet/replay semantics explicitly. Updating only the Web worker would leave the shared PS modeling errors and native measurement path unresolved. | `bot-worker.ts`, `evaluate-worker.ts`, `ProtocolAgent`, `BlindAgent`, `team_eval.rs`, `teamgen.rs` |

Root exclusions need not require enumeration of every possible world. Public
bounds and conservative rules can prove an exclusion. A single posterior draw
cannot establish a universal claim. Sampling a simulation world remains
legitimate; the restriction is on using its guesses as fixed root-level facts.

Keep battle mechanics, legal-action generation, team canonicalization, the
catalog and prior contents unchanged unless a separate defect requires a
change. The research solver may still operate on full hypothetical states;
those states must be sampled or explicitly supplied under a research API.

## Required verification

1. **Independent observation oracle.** Compare complete player frames rather
   than defining agreement through determinization. With fixed observation
   history, configuration and search randomness, varying hidden truth must
   not change the model, exclusions, values, early returns or decisions.
2. **Adversarial cases.** Cover both sides, HP values within one public bucket,
   unknown maximum HP, hidden moves/items/stats and bench selections, cancelled
   unannounced choices, hidden counters and actual battle/Quick Claw RNG.
   Include public-fact changes as positive controls.
3. **PS-specific regressions.** For every admitted candidate, reconstructed HP
   must reannounce the observed percentage. Exclusions must remain valid when
   another supported item or speed hypothesis is substituted. Merely changing
   candidate order must not change the logical exclusion. Exact sampled
   trajectories need not be order-invariant if RNG draws map to reordered
   candidates; that is distinct from exclusion validity.
4. **API checks.** Compile-fail tests should reject real-battle input to blind
   search and unauthorized construction of observation/sampled-world types.
   Runtime adapter tests must check side selection, split-log filtering,
   validation and public/private field classification.
5. **Integration.** Verify native/WASM agreement for the corrected policy,
   persistent observation state, switches/Baton Pass/Transform, replay/fork
   behavior, worker cancel/commit behavior and request legality. Run the
   engine/conformance checks to preserve mechanics independently of policy.
6. **Performance and policy measurements.** Measure search latency, memory,
   model-sampling/cache cost and the extra work from conservative exclusions.
   Old-versus-new action equality is not a repair gate: decisions are expected
   to change. Strength claims need fresh paired measurements.

The input boundary and its constructors remain trusted code. Type separation
prevents ordinary search callers from accessing real state; it does not prove
that the boundary selected the correct player, or that a posterior model is
statistically correct. Both need independent tests.

## Three different uses of “Nash”

### In-battle action distributions

The root policy and optional estimated mixtures use candidate-dependent
payoffs and eligibility. Correcting information and exclusions changes those
inputs and can change supports and probabilities. The RM+/matrix solution
algorithm need not be replaced simply because its inputs change. The ordinary
WASM profile uses `SelRule::Ucb`; a root estimator is not a certificate of a
full-game imperfect-information equilibrium.

### Catalog team-preview bake

The active producer is
[`meta_eco/catalog_preview.rs`](../crates/bot/examples/meta_eco/catalog_preview.rs),
documented in [preview-training-v3](../data/preview-training-v3/README.md).
It constructs lineups through the native engine, evaluates compact rollouts,
and solves prior-conditioned Bayesian preview matrices. The current artifact
has 990 keys (33 own teams x 30 public rosters), backed by 1,128 unordered
type-pair matrices. It does **not** generate these values by playing the live
`BlindSearch` policy. The older `compile_preview.rs` live-search bake is not
the producer of the active artifact.

Consequently:

- Boundary and root-exclusion changes alone do not establish that the active
  rollout payoff matrices or quantized selections change. The compact rollout
  path does not call the live root-exclusion routine.
- Prior filtering/weights, canonical teams, legal selections, engine
  initialization, compact rollout policy, evaluation or Bayesian preview
  solver changes do affect the bake's semantic inputs and require recomputation
  of affected matrices and downstream policies.
- Any bot/engine source change still changes the broad build fingerprint and
  stored source hashes. Production build validation rejects stale artifacts.
  Either recompute or make a separately audited, exact-behavior migration,
  checking generator dependencies, candidate membership/weights, keys, legal
  actions, selected policies and actual worker lookup. Updating hashes alone
  is not valid evidence.
- Preserve the table lookup when changing the worker API; routing everything
  through the existing protocol branch without reconnecting lookup would
  replace instant table responses with online preview searches.

Even a fresh bake solves an approximate **rollout game**, not the corrected
live bot's whole-game equilibrium. Measuring whether to optimize preview for
the corrected live bot is a separate, more expensive research objective.

### Party-draw mixture across teams

The distribution used by `?nash` is a different artifact from the per-matchup
preview policy. Changing live decisions changes the game whose entries are
team-versus-team payoffs. To claim a mixture is an equilibrium for the fixed
catalog under the corrected bot, remeasure the payoff matrix with the same
bot, preview policy, prior, budget, termination/scoring and side/seed pairing
as the intended product; then resolve weights and run uncertainty and
best-response/challenger checks. Old simulation outcomes cannot just receive
a new source fingerprint. Historical full-state/open-sheet experiments also
do not become blind-product evidence after relabeling.

The three shipped mixture files already mark their weights/payoffs as
**historical and unremeasured after training/prior normalization**:

- `data/kodoku-r3/conclusion-equilibrium-deme.json`
- `data/kodoku-r3/conclusion-equilibrium-r1.json`
- `data/meta-nash-v3/pool-artifact.json`

They may remain historical team-draw distributions while the runtime is
repaired, with that status preserved. Re-establishing an equilibrium claim
requires fresh measurements. Re-running all past team evolution or replacing
the catalog is not a prerequisite to closing the information leaks.

## Staged delivery and dependency boundary

1. Implement the observation/types, reconstruction and exclusion changes as
   one policy correction. Complete correctness and integration checks across
   Web, PS and native measurement; give the corrected policy a new identity.
2. Resolve preview artifact compatibility through demonstrated equivalence
   or recomputation, then verify complete production lookup and latency.
   Rebuild WASM/Web and version policy-dependent caches and records. Physical
   battle replays can remain compatible if mechanics are unchanged; search
   traces and measurement conditions must retain their original identity.
3. Separately remeasure party payoffs, solve the team mixture and challenge it
   before presenting new weights as an equilibrium of the corrected policy.
   Until then, preserve historical labels rather than delaying the privacy
   repair on a large equilibrium computation.

If the implementation stays in bot/adapter layers and does not alter battle
physics or compact state layout, it does not inherently require an engine
mechanics rewrite, replay-codec migration, TPU-kernel port, new teams or
retraining. Any expansion into those shared components must be justified by
an actual dependency rather than by the fact that they share a repository.
