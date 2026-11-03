# Measurement validation before policy diagnosis

Status: **initial audit executed; confirmation stopped at the first failing
information-boundary gate**. See `data/measurement-validation-v1/README.md`.
The replacement runner and remaining gates are not implemented/executed.
Baseline source revision: `69e0f83f`. The design was committed at `47a86092`
before this audit. This document is not a record of passed gates or a final
preregistration. Executable/data hashes, the seed
commitment, and the unchanged protocol must be frozen before confirmation.

## Question and non-claims

Primary question: can a fixed, rules-compliant policy-response attacker beat
the product decision policy in a symmetric APEX4 party-draw game, when an
independent runner gives each player only its allowed information and scores
actual engine terminal outcomes?

Two separate measurement claims require separate evidence:

1. The historical 42–22 attacker result is a valid measurement of a legal
   policy defeating the intended product policy.
2. A diagnostic root payoff matrix identifies the cause of that defeat, or
   identifies a necessary equilibrium mixture.

The second claim does not follow from the first. The frozen-search matrices
in `data/policy-oracle-cases-v1/` measure heuristic continuation returns,
mostly without reaching terminal states. They are not win probabilities or
optimal-play bounds. More samples do not repair that estimand mismatch.
This phase validates match measurement first. It does not retune the bot,
optimize party weights, select a new attacker, or classify full-game Nash
equilibria from those matrices.

## Explicit assumptions and decisions

| ID | Assumption or decision | Treatment |
|---|---|---|
| A1 | The target game is the repository's documented NC2000 regulation, including its operational sleep-forfeit rule, legal team selection and engine turn-limit tie. | Freeze rule/data revision. Check rule-sensitive fixtures independently; do not equate this automatically with the older PS prevention clause. |
| A2 | Payoffs are constant-sum: win 1, engine tie 0.5, loss 0. | Independently recompute side/role scoring from terminal logs. Report W/L/draw separately. |
| A3 | Both roles draw independently from the same four APEX4 types at weights 0.30/0.25/0.25/0.20; seats are balanced. | Freeze canonical full sets and draw law. Mirror complete assignments, not just names. This is a restricted, symmetric party-draw game. |
| A4 | Pool types, draw weights, rules, public game history and algorithm descriptions may be known. The opponent's realized hidden type, unrevealed picks/sets, private seed and committed simultaneous action may not be supplied. | Explicit input allowlist and isolated worker transport. Knowing all possible types is not knowing the realized type. |
| A5 | Each player may receive its own submitted team and its own normal request fields, including own exact HP/PP and legal inputs. Hidden clocks and game RNG are not added as conveniences. | Compare against the player observation contract; audit fields rather than trusting the adapter's name. |
| A6 | More computation for the attacker is legal. | Preserve the attacker's independent policy-model searches. No equal-total-time requirement or compute-based dismissal of a win. Record resource use. |
| A7 | "Product bot" means the actual execution path and data, not merely the same search algorithm or iteration count. | Include preview routing, seed truncation, UCB configuration, early/forced decisions, request projection, and `commitDecision("legacy")` in parity gates. |
| A8 | The shipped APEX4 prior is uniform, while actual draws are not. Correcting it creates a distinct configuration. | Keep deployed (D) and known-pool (K) arms separate. No prior change is silently described as an unchanged product measurement. |
| A9 | Changing shell weights can invalidate a compiled preview. | Recompute the four APEX4 self-shell policies for K under the frozen corrected prior and the same producer/settings; validate dependencies. Do not relabel or reuse incompatible choices. |
| A10 | A policy predictor estimates the opponent using allowed observations and its own randomness. It need not predict perfectly. | Test its access boundaries and input/action mappings. Never give it the opponent's actual agent instance, seed, chosen input or hidden request. |
| A11 | Root visits, a point-valued estimated matrix, and a single action conditional on a search seed are not the deployed seed-marginal behavioral strategy. | Record the committed input separately. Do not call a visit fraction an execution probability or assume all argmax searches are globally deterministic. |
| A12 | The historical T2 wall-clock solver is not deterministic under different logging/load. | Audit that version separately. Use a distinctly named, work-limited T2-W for confirmation; never claim it is an exact reproduction of the historical attacker. |
| A13 | Unobserved simultaneous actions cannot influence the other decision. | Both workers commit against the same pre-step observations. Apply neither action until both are committed. Reverse invocation order as a control. |
| A14 | Match blocks are independent; games within a mirrored block are correlated. | Independent master draws across blocks, separated RNG domains, fresh agent state per game. Analyze blocks, not turns, decisions or individual paired games as independent observations. |
| A15 | Common random numbers improve pairing but do not guarantee identical future chance events after different actions. | Preserve seed assignments; do not force events to coincide or count a single fork reversal as a causal win-rate estimate. |
| A16 | A timeout, crash, illegal-input exception or infrastructure cap is not an engine draw. | Mark a technical failure, retain all records, suspend aggregate inference and repair/rerun the affected block under a declared policy. No silent dropping or award of wins. |
| A17 | Old trials, selected losses and attacker-selection records are development data. | New committed seeds only for confirmation. Freeze attacker, settings, sample size and endpoints before labels are exposed. |
| A18 | Passing finite tests cannot eliminate every possible measurement error. | Report exactly which gates passed and residual trusted components. Do not claim universal noninterference or full mechanics correctness. |
| A19 | Native and WASM share implementation and therefore can share bugs. | Cross-runner agreement checks plumbing. Separate rule fixtures/reference comparisons address mechanics where the regulations actually agree. |
| A20 | A non-significant win-rate result is not parity, and disappearance of an effect after a code/configuration change does not identify its cause by itself. | Report inconclusive outcomes; use matched replay/interventions to attribute identified discrepancies. No optional extension until significance. |
| A21 | The frozen workers and runner obey their reviewed transport/file-access contract. | Minimize the trusted host, exclude truth logs/seed manifests from worker access, and audit worker inputs/IO. Process separation alone is not a proof against an arbitrary malicious binary or OS side channel. |
| A22 | The statistical bound assumes independent block sampling from the frozen draw law and bounded terminal scores. | Use independent seed domains with no worker-visible route to recover hidden seeds or type draws. A finite PRNG implementation approximates this sampling assumption; tests do not establish mathematical independence. |

## Why the existing runner is insufficient as its own validator

`omniscient_audit` owns the true `Battle`, both legal-action lists, both
chosen actions, and clairvoyant research variants in one function. The
corrected T2 path appears to use reconstructions, but the `rules_compliant`
flag is only a declaration. It is not an independent access guarantee.
Its Blue-view shadow tracker is constructed from the true Blue side even
though only a restricted derived view is intended to affect Red's model.
Precomputed oracle records also contain both legal public mixtures and
clairvoyant fields. These need structural separation, not reassuring names.

The `T0BaselineBlind400` branch uses `BlindAgent` and supplied preview/pick
policies, while corrected Blue uses `ProtocolAgent` and compiled preview
choices. It is not an exact product-vs-product control. A mirrored 50% result
can also be built into the pairing and therefore is not by itself validation.

The earlier replay changed the number of time-limited solver interventions
in one of six games while preserving its winner/terminal turn. Matching
those two outputs is weaker than matching the decisions. The historical
record did not save all decisions, so exact historical trace reproduction
cannot now be asserted.

## Gate 0: freeze the target and retain the evidence

Create a manifest of source/data revisions, native binary and WASM hashes,
toolchain/build flags, actual web worker bundle, rules, exact parties, both
priors, preview artifact/dependency hashes, execution configuration, RNG
conversions and termination/scoring. Read budgets from versioned data:
currently strong battle 81,000 and uncached preview 729,000, UCB c=0.4 and
legacy execution. Record enabled/disabled optional features explicitly.

Restore all 64 historical records from `872d8ed8` and independently recompute
42–22, the 32 block assignments, seed/seat/type swaps and uncertainty inputs.
Use all records rather than the six selected defeats. Attempt whole-run
historical reproduction only with its exact source/data and declared clock
conditions. Retain mismatches; do not tune until the old total reappears.

Technical pilots use a separate seed set and expose timing/completion/gate
results only for resource planning. They do not contribute to confirmation.
Estimate compute from measured cost; no assumption about hardware throughput
or authorization for cloud resources is made by this design.

## Gate 1: independent referee and information boundary

Use a separate referee with pipe messages to two worker processes. The
referee alone owns the real game and its PRNG. Each worker receives:

- its own submitted team, own search configuration and own private RNG seed;
- the frozen allowed pool/weight metadata for the arm;
- its player frames and legal inputs, plus a decision identifier.

No true opponent object, opponent request, actual draw index, hidden state
hash, opponent seed, pending action, realized current-turn chance draw or
future engine trace is sent. Red constructs its model of Blue from Red's
allowed history and sampled hypotheses; no Blue-side adapter or agent
instance is available to Red. Preview lookup is keyed by own information
and the public opponent shell, never the actual opponent type.

Truth diagnostics are written after commitments by the referee to storage
unavailable to workers. Histories/IPC messages are versioned and hashed.
The two commits share an opaque pre-step sequence ID, not a hash of hidden
state. Repeated polling cannot redraw a
committed decision; cancelled/stale results cannot apply to another state.

Required controls:

1. **Same-information hidden-state interventions.** For each player, retain
   the entire allowed observation history, own private information and
   agent RNG/internal state; vary compatible hidden opposing types/picks,
   unrevealed moves/items, exact HP within the public bucket, hidden clocks,
   game RNG and the opponent's pending action. For deterministic T2-W and
   the product worker, input bytes and committed input must be identical.
   Test both sides, preview, normal turns, forced switches and endgame-model
   paths. Distinguish reachable legal witness pairs from hostile synthetic
   boundary probes; do not assert every arbitrary mutation is a legal world.
2. **Useful sensitivity controls.** Change allowed observations in fixtures
   designed to change legal choices/response; show the test is not passing
   because requests or outputs are ignored. Intentionally contaminate a
   test-only worker message with a secret canary and verify that the
   transport/access checker rejects it. Canary workers never enter matches.
3. **Order controls.** Invoke Blue first, Red first, and concurrently with the
   same inputs/seeds. T2-W actions and transcripts must agree. Both sides'
   actions remain withheld until the commit barrier.
4. **Instrumentation controls.** Logs on/off, serial/concurrent scheduling
   and different worker counts must preserve T2-W/product decisions and
   referee transcripts. Record wall-time version variability separately.

All mismatches must be understood before confirmation. A finite corpus is
not the boundary proof: the restricted transport and absence of truth-fed
construction paths are also necessary.

## Gate 2: product identity, action mapping and scoring

For all decision frames in the six saved traces and an independent mechanics
fixture suite, execute both the native product adapter and the actual WASM
worker from game start. Compare preview key/slot and chosen input, belief
candidate identities, search parameters, legal input projection and committed
action, including 32-bit seed behavior. No target-specific early-search
skipping or forced historical action is allowed in this parity gate.

Use direct committed-action equality as the acceptance criterion. Investigate
any numeric diagnostic mismatch; if it can alter a decision, the native arm
cannot stand in for the product. In that case confirmation uses the actual
product worker and the identity claim is narrowed instead of tolerated away.

Every `SearchChoice`/input/party-slot/solver-row mapping must round-trip by
action identity. Comparing vector lengths is insufficient. Check Hidden
Power aliases, bench ordering, forced replacements and reply indices across
sampled worlds. All agents must submit request-legal actions; no silent
fallback to the first action is accepted as a valid intended decision.

Build the A/A control from two instances of the **same actual product
worker**, not the existing T0. Run 16 independent mirrored blocks plus the
fixed fixtures. Compare individual transcripts against the reference runner,
side/role bookkeeping, reset behavior and action inputs. The mechanically
paired 50% aggregate is a consistency check, not an independent strength test.

Replay referee action logs with a second, simple replayer that does not use
the attacker. Recompute engine outcomes and score orientation independently.
Use hand-checked win/loss/tie fixtures and intentional score-label corruption
to verify the checker detects wrong attribution. Rule-sensitive fixtures
cover damage/critical hits, speed ties, sleep/Rest/Sleep Talk, freeze, curing
berries, faint/replacement and recoil; compare with PS only where rule sets
match. Shared native/WASM engine agreement does not settle a shared bug.

## Gate 3: remove clock-dependent measurement coupling explicitly

Keep `T2-H` as the historical version with its 80ms bound-solver limit.
Record its actual work, stop reason, branches and timing conditions; trace
variation is not automatically an information leak. A recorded work-stop
tape may diagnose reproducibility, but is audit data and cannot become an
attacker input in confirmation.

Define `T2-W` for confirmation by preserving T2's policy and work/node/cell
limits while removing its wall-clock stop (`time_budget=None`). Freeze that
definition before trials. This may change the attacker and its strength;
report it as a new lawful witness, not as the historical T2 restored.
Product budgets remain unchanged. A larger legal attacker budget is allowed.

Preserve the historical predictor's K=2 independent model searches and
warmup/response allocation unless an implementation defect requires an
explicit new revision. Do not alter those to improve pilot wins. Isolated
and old in-process implementations must agree at identical frames when
given identical deterministic stopping rules; any difference is diagnosed.

## Gate 4: prospective terminal-outcome confirmation

Freeze two arms; both use T2-W in the independently validated runner:

| Arm | Blue | Red knowledge and preview model | Meaning |
|---|---|---|---|
| D | Exact product configuration, including uniform APEX4 prior and compatible current preview | Actual APEX4 draw weights; frozen response to D's actual preview policy | Tests vulnerability of the deployed configuration; knowledge weights are intentionally asymmetric as historically recorded. |
| K | Same product search/execution algorithm, with the correct known-pool prior and a freshly compatible four-key preview bake | Same known pool/weights; frozen response to K's actual preview policy | Tests the user's common-knowledge condition; explicitly a known-pool product variant. |

Both draw the **same** APEX4 mixture, independently for their own type. In K,
each player must initially admit exactly the four actual types with relative
weights 0.30/0.25/0.25/0.20 and no off-pool fallback. Verify all four actual
types, canonical sets, preview compatibility and target-model metadata before
running. If K's required preview artifacts are not ready, mark K unready;
do not quietly use D's bake, substitute live preview or call D common-knowledge.

Within each of 512 independent blocks, sample two types plus independent
engine/agent RNG streams. Run both policy-to-seat assignments while keeping
the physical team's seed allocation documented and mirrored. Reset both
agents for every game. The D/K arms may share block draws for descriptive
paired comparison, but the second game is not an independent sample. Never
derive worker-visible seeds from type IDs or the opposing secret seed.

Commit a hash of the seed manifest before exposing outcomes. Keep the actual
engine/opponent seeds inaccessible to workers. Preserve production RNG
support/conversions; do not upgrade the product's PRNG while claiming parity.
No outcome-based stopping, candidate tuning or selective reruns are permitted.

There are **512 blocks / 1,024 games per arm; 2,048 confirmation games total**.
This is chosen to check an effect of the historical magnitude, not to prove
2-percentage-point equivalence. Gate failures stop the run before inference.

Let `X_b` be T2-W's average terminal score in block b, in [0,1]. The primary
estimand per arm is `E[X_b] - 0.5`. Use a one-sided bounded-mean lower bound
with alpha=.025 per arm, controlling the two superiority claims at .05 by
Bonferroni under independent-block sampling:

    L = mean(X) - sqrt(log(1/.025) / (2 * 512))
      = mean(X) - 0.060020174...

Declare a statistically supported victory only if `L > .5`, with every gate
passed and no unresolved technical failures. For illustration only, a new
observed score of .65625 would yield L approximately .59623. Do not transfer
that bound to the historical 32 blocks. Also report paired SE/descriptive
intervals, W/L/draw counts, per-seat and per-type summaries, but no extra
significance claims from those subgroups.

Reach actual engine terminals, including the engine's own rule-defined turn
limit; remove the experiment's 250-turn tie substitution. A host watchdog
can protect resources, but its activation is a technical failure. Never use
an HP score, heuristic leaf or failed process as the terminal result.

For a transient infrastructure failure, retain the partial trace and rerun
the entire block with the same frozen configuration/seeds only after the
cause is documented; deterministic replay must agree. An algorithmic defect
requires a new revision and fresh confirmation registration/seeds. Retain
the abandoned result and reason; repeated revisions are not independent
opportunities to claim significance without disclosure.

## Interpretation and stopping rules

- **Boundary/identity/mapping/scoring gate fails:** the affected measurement
  claim is unvalidated. Produce the smallest counterexample. Do not proceed
  to a larger win-rate run or attribute the result to poor strategy.
- **D wins after all gates pass:** supports a lawful exploiter of the actual
  deployed configuration. Does not yet remove prior-knowledge asymmetry.
- **K wins after all gates pass:** supports a lawful exploiter despite common
  knowledge of the draw pool and weights. More attacker compute is irrelevant
  to this validity claim. This still does not identify mixing as the cause.
- **D wins and K does not:** not proof that weights caused the historical
  defeat; K also required a compatible derived preview and uncertainty remains.
  Any weight-only causal study is a separately registered follow-up.
- **Neither wins significantly:** inconclusive about exploitability and
  parity; also not proof that the historical measurement was erroneous.
- **Historical replays differ only under wall-clock conditions:** describe
  the timing-sensitive attacker separately; do not silently substitute T2-W
  outcomes into the old record.

Only after these gates do root-cause experiments resume. A later matrix must
state its continuation policy, information constraints and payoff definition.
Repeated production continuation to engine terminal measures a policy-specific
game, not optimal play. Necessary full-game mixing requires sound continuation
values/bounds and all relevant information-constrained responses. Frozen
rollout matrices and loss-selected root examples do not meet that requirement.

## Required output

Retain the frozen manifest, seed commitment, complete per-player input streams,
committed actions and RNG identities (in referee-only logs), engine actions,
stop/work diagnostics, terminal outcomes, all failures/retries, per-block
scores, independent rescore and gate results. Publish no aggregate "passed"
label unless every required gate is individually accounted for. Keep code,
configuration and measurements separate so a result cannot be upgraded by
changing metadata alone.
