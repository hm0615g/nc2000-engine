# Blind product and team-pool rebuild

Status: **in progress (2026-09-30)** — see [§Progress](#progress). Owner
direction updated 2026-09-30. This is the authoritative plan and handoff; update it in place.
The starting implementation is local `master` at `d3b7059`, 18 commits ahead
of the locally recorded `origin/master` (`23168bb`); the working tree was clean.
The previous rebuild is integrated locally but is not accepted as a measured
product improvement and has not been pushed or deployed.

## Owner decisions and scope

1. Retire the open-sheet product mode. Normal browser play and the ladder use
   blind information, with exploration constant **c = 0.4** throughout.
2. Remeasure the prior, team selection and Nash under that information/profile.
   Historical open results and blind c = 1.0 results cannot qualify teams.
3. The built-in parties a human selects on the play screen and those the ordinary
   bot draws must be the **same selected strong pool**, with identical exact sets.
4. Raise the strong standard. The motivating defect was weak parties in the
   product pool; merely importing more parties or preserving 79 strong labels
   does not solve it. Remove parties whose removal improves pool performance.
5. Aim for **15–30 parties** in the final ordinary pool. This is a target range,
   not permission to retain demonstrated weak parties to reach a quota.
6. This turn requests a plan for the next AI. Implementation, experiments and
   publication were not performed in this handoff. Public push/deploy still
   needs explicit authorization.

No decision here removes custom-party import or the solver/evaluator/fork tools.
Nash remains a distinct draw policy over strong parties, not an information mode
or a stronger scalar label. Consolidate `/` and `?blind`; retain `?blind` as a
compatibility alias. Keep `?nash` as a weighted blind draw unless a later product
decision removes that policy. Do not infer a redesign of every URL tool.

## Why this is consistent, and where the objective needs precision

A pool's symmetric self-play average is 0.5 regardless of its quality (ties and
caps scored 0.5). It cannot measure improvement from removing weak parties.
Freeze an **external opponent panel and its weights**, separately from the own
pool being selected. The panel may contain candidate parties, but its membership
and weights must not change when a candidate is removed from the own pool.

Let `s(t)` be party t's expected score against that fixed panel under the frozen
agent conditions. Let `q_S(t)` be the ordinary draw probability when the selected
pool is S. Measure:

`V(S) = sum[t in S] q_S(t) * s(t)`

`Delta_remove(t, S) = V(S without t) - V(S)`

Use the actual draw rule when recomputing q after removal. Under uniform team
weights this is `(V(S) - s(t)) / (|S| - 1)`; cluster-balanced draws need the full
recalculation. Report both individual party scores and pool-level effects so
small diluted effects cannot hide a plainly inferior party.

Unconstrained maximization of this average eventually keeps only the highest
scoring party or tied parties. Therefore "remove everything whose removal helps"
and "always keep at least 15" are not simultaneously guaranteed. Select for high
performance within the desired range, remove clearly harmful entries first, and
report a conflict if the evidence still calls for pruning below 15. Do not call
that state complete, pad the pool, or tune thresholds after seeing results.
Likewise, a point-estimate increase is not sufficient evidence for exclusion.

Strong here means **selected for this bot's ordinary product pool**. A party can
fail this standard because the bot cannot pilot it. Exclusion does not establish
that the party is inherently bad or replaceably dominated. The stricter existing
three-part dominated rule (trusted close replacement, confirmed inferiority,
competent execution) still governs that separate verdict.

Ordinary average strength and Nash exploitability are different objectives.
Check excluded specialists against the final pool/mixture. If a strategically
necessary specialist fails the ordinary standard, report the conflict and revise
the declared selection policy explicitly; do not silently force it into strong
or silently discard the counterexample.

## Starting artifacts and evidence limits

| Artifact | Current local contents | Treatment in this rebuild |
|---|---|---|
| `data/team-inventory-v1/inventory.json` | 133 records, 115 eligible, 103 measured representatives after 12 loadout duplicates | Reuse provenance and legal sets; audit representative assumptions where training affects play |
| `data/team-inventory-v1/classification.json` | 79 strong, 21 weak, 2 pending, 1 dominated | Historical v1 labels; all 103 are candidates for blind remeasurement |
| `data/team-pool-v1/team-pool.json` | 79 parties, equal mass per variant cluster, split within cluster | Baseline own pool, not the desired final selection |
| `data/belief-pool-v2/belief-pool.json` | 102 parties: 79 strong + 21 weak + 2 pending | Provisional prior baseline; validate before freezing decisive measurements |
| `data/meta-nash-v2/solution.json` | Nine weighted parties from a 23-party game, blind:3000 at c = 1.0 | Historical baseline; weights need recomputation |
| `data/meta-pool-v0/meta-pool.json` | Built-in human-selectable 32 | Replace as the default play-screen catalog after selection |

Classification v1 used open:300 discovery (195,168 games) and open:1000
confirmation (68,768). The prior tests and Nash used blind with **c = 1.0**, at
1000 and up to 3000 iterations respectively, without pondering. These large
sample counts do not repair the mismatch with the blind product profile.
Existing measurements and their frozen protocol remain historical evidence;
write new protocol/results separately instead of changing what they claim to
have measured.

Specific unresolved cases:

- `sample-16` was called strong at open:1000 (0.545). Its source ポニョ beat it
  by +0.070 [0.032, 0.113]. The old dominated rule withheld exclusion because
  source trust and level allocation did not qualify. **The new ordinary-pool
  pruning does not require a dominated verdict**; measure both under blind.
- `sample-18` and `rental-cban-12` were pending; no inherited admission.
- The sole old dominated verdict, カビバン2020 → カビバン2022, needs blind c = 0.4
  confirmation before it justifies exclusion from the revised prior. The
  c = 1.0 blind difference was +0.034, below the old 0.05 materiality threshold.
- セミスター was labelled weak but scored 0.589 against the candidate Nash
  mixture, CI [0.486, 0.693]; extend evidence before deciding. Several off-prior
  chimeras beat that mixture around 0.60–0.63. They remain challenge cases.
- Prior machine-team factor 0.25 is an a-priori choice, **not calibrated**:
  only 23 human preview-collision cases were available. See
  `data/team-inventory-v1/allocation.json`. Do not advertise this as a fitted
  human usage model.

## Execution sequence for the next AI

### 1. Freeze the product contract and experiment manifest

Read the working tree, this plan, and applicable skills before editing. Use the
browser/UI skills for visible changes and metric-calibration before using the
revised score to select teams. Do not rerun the old large matrix first.

- One live blind search profile, c = 0.4; retain **27,000 iterations** as the
  product budget. The owner changed mode and c, not this budget.
- Browser pondering remains enabled; ladder and fixed-work experiments have no
  pondering. Record this remaining difference, rather than claiming identical
  behavior solely because c and base iterations agree. Unifying state adapters
  or adding ladder pondering is outside this decision.
- Audit browser game, ladder, solver, evaluator, fork continuations, native
  measurement harnesses and wasm bridges for implicit c = 1.0 defaults. Product
  strength evidence must explicitly serialize information mode, c, iterations,
  prior fingerprint, bot revision, preview behavior, cap treatment and seeds.
- `team_eval` already accepts `--agent blind:ITERS:0.4`; supply it explicitly on
  both sides (audit `--agent-col` inheritance). Prevent resumed jobs from mixing
  old c = 1.0 cells with new cells.
- Keep historical replay/open-state decoding where needed for faithful old
  records. It must not remain a live product open-mode toggle or silently change
  historical information. Research true-state internals are not a product mode.

### 2. Validate and freeze the opponent prior

Own-party pruning must **not** prune the opponent model to the same 15–30.
Humans can bring weak or off-pool parties. Keep strong, weak and pending eligible
parties in the prior; remove dominated parties only with adequate new evidence.
If the old dominated verdict is not reconfirmed, restore that candidate to the
new prior before the decisive selection measurements.

Reuse the held-out setup/report tools, but run c = 0.4 with disjoint seeds. Compare
the provisional broad prior, the previous v1 prior and a strong-only diagnostic;
include strong human teams, weak teams, held-out page teams and off-prior cases.
Check weighted fallback and the allocation sensitivity. A small screening budget
can select experiments; confirm consequential choices at 27,000 iterations.
Freeze support, weights and file hash before selecting the own pool. Later prior
changes invalidate dependent results; do not quietly regenerate a different
fallback by reordering entries according to the new strong labels.

### 3. Preregister classification/selection protocol v2

Create a new versioned protocol and measurement directory; retain
`data/team-inventory-v1/PROTOCOL.md` unchanged. Reuse the inventory and tools where
possible; `tools/team-label.py` currently hardcodes 0.45 and open-era rules, so it
cannot decide the new labels unmodified.

Before opening new selection scores, freeze:

- A fixed human-source panel, cluster-balanced to avoid near copies gaining
  weight by count, with explicit important matchup strata. Use the existing
  human-source inventory to construct it. Retain an independent challenge panel
  (held-out page teams, off-prior chimeras and excluded candidates). Neither panel
  shrinks with S. Do not substitute the final Nash mixture as the sole evaluator.
- A common scoring panel for all candidates: do not remove a different subset of
  opponents from each candidate's denominator. Any mirror/lineage policy must
  leave scores comparable and be fixed in advance.
- Ordinary draws: retain equal mass per variant cluster, equally split among
  selected members, as the working rule. Use this same rule in measurement,
  browser Random and ladder Random. Human manual selection shares membership
  and sets; it is not itself a probability distribution.
- A higher absolute admission floor. **Proposed working value: lower 95% bound
  above 0.50 against the frozen trusted-human panel**, replacing v1's 0.45 rule.
  This number is a planning choice, not a separately owner-specified threshold.
  Freeze it before confirmation; if fewer than 15 qualify, report the shortfall
  rather than lowering it after observing results. This floor alone is not the
  final strong label: harmful members must also be pruned.
- Paired battle/search seeds, side swaps, independent discovery and confirmation
  seeds; uncertainty resampling at the shared seed block, preserving pairing.
  Address multiple candidate exclusions with simultaneous intervals or a fixed,
  independently confirmed candidate set. Ordinary per-team 95% intervals after
  adaptive selection are insufficient to claim all pruning decisions are sound.
- A finite sample/extension schedule and a precision target for individual
  deficits and pool-level deletion effects. Translate between them using the
  actual draw weights; do not impose a pool-level margin so large that diluted
  weak-party effects can never be detected. State unresolved outcomes explicitly.
- Ties and 500-turn caps separately, both scored 0.5; inspect cap-sensitive
  rankings and actual selection/play for combo parties.

### 4. Screen cheaply, then select and confirm at product budget

Calibrate the revised metric with concrete competent, broken and confound
controls under blind c = 0.4. Start with sample-16/source and a few documented
combo cases; establish the pilot's resolution before broad measurement.

Screen all 103 eligible representatives at blind:3000:0.4 against the fixed panel.
Use this to prioritize measurements, not to qualify final strong parties. Budget
sensitivity is already observed: c = 1.0's Nash anchor changed between 300 and
3000. Bring plausible entrants, uncertain boundary cases and old high-budget
specialists into the **blind:27000:0.4** confirmation shortlist. Audit a sample
of apparent low-budget rejects for ranking reversals before discarding them.

Choose a discovery candidate S in the 15–30 range using the fixed draw rule and
panel objective. Trim demonstrably harmful entries, recomputing weights after
each deletion; rank candidate subsets with uncertainty instead of simply taking
the old top 30. Confirm the proposed selection and relevant deletion/swap
comparisons on fresh seeds. If confirmation changes S, use reserved fresh blocks
or the preregistered sequential rule; do not reuse the same hold-out indefinitely.

Compare the final candidate with the 79-party local pool and the old 32-party
pool under **identical blind c = 0.4 conditions and the same fixed panel**. Report
V, uncertainty, per-party scores, deletion effects, stratum scores, caps, and
reasons for every inclusion/exclusion. Uncertain entrants stay pending; ordinary
exclusion alone does not remove a party from the prior or reference inventory.

Add a declared browser-ponder sensitivity check using a reproducible think-time
schedule or bounded extra-work scenarios. It is a sensitivity experiment, not a
measurement of all possible human think times. If selected membership reverses
materially, resolve that before claiming it fits browser play as well as ladder.

### 5. Recompute and challenge Nash

On the frozen prior, solve the selected strong game at c = 0.4, with support-
relevant confirmation at 27,000 iterations. Challenge it with excluded parties,
held-out designs and the known off-prior exploiters on fresh data. Refine
uncertain exploiters and revisit selection when evidence warrants it. Report
scope and uncertainty: this is a finite candidate game, not the entire format.

Do not keep the nine existing weights because they are already wired. Do not
claim absence of an exploiter merely because a wide interval overlaps 0.5.
The evaluator's default opponent mixture follows the accepted new Nash artifact;
its displayed iterations must distinguish exploratory runs from product-budget
measurements. All evaluation budgets use c = 0.4.

### 6. Generate products and integrate the shared strong catalog

Generate own pool, prior, Nash and reference memberships from explicit versioned
inputs. Keep their different probability fields separate; make prior generation
stable against ordinary-label changes after its freeze.

- `/` and `?blind`: identical blind game and c = 0.4. Remove open-sheet UI and
  opponent-set pinning from normal play; verify hidden sets remain hidden.
- Human built-in catalog, human Random and ordinary bot Random: identical strong
  IDs and exact sets; both Random paths use the ordinary draw weights. Ladder
  `pool:random` and `pool:ID` consume that same artifact. Rematches redraw the
  blind bot party as usual.
- Custom-party import remains available; user-supplied opponent pools are explicit
  overrides, not the default strong catalog and not replacement belief priors.
  No old persisted pool may silently defeat the new default membership.
- `?nash` draws its subset by Nash weights, with the same blind profile and prior.
- Solver/evaluator use the same profile and prior where applicable; historical
  fork/replay data retains its recorded semantics and is clearly distinguished
  from current-product validation.
- Invalidate obsolete index-keyed preview tables. Update legality, catalog,
  hidden-information, weighted-draw, worker/native configuration and integration
  checks. Follow the relevant browser skills for visible verification.

### 7. Close documentation and handoff only on evidence

Make README describe the current product and point here for rebuild status.
Remove contradictory current claims (v1 shipping weights, 32-party prior,
open-sheet default, premature DONE). Artifact READMEs should describe their actual
files: the current Nash v2 README still calls the 21-party do1 game final although
`solution.json` points at do2 with 23 parties.

Completion requires:

- Blind-only live product, c = 0.4 across consumers, tested information boundary.
- One measured strong catalog for human built-in choices and ordinary bot draws;
  target 15–30, with unresolved size/strength conflicts explicitly blocking a
  completion claim rather than hidden by quota-filling.
- Frozen broad prior, product-budget selection evidence, independent pruning and
  exploit challenges, and documented pondering limits.
- New Nash weights tied to the same prior/profile and supported measurements.
- Reproducible manifest, raw-result availability, supported labels and passing
  relevant tests. Historical web e2e was reported 44/45, with an evaluator-copy
  failure; rerun after changes instead of carrying forward a pass claim.
- Clear local/validated/published status. Commit local milestones; do not publish
  without explicit authorization. The next AI starts at step 1, not deployment.

## Progress

Branch `blind-rebuild` (local, not pushed). Each item names its evidence.

### Step 1 — product contract (done)

- Live play is blind: `/` and its alias `?blind` are one door; no opponent
  picker, no open-sheet surface, no baked pair tables on any door; worker,
  ladder client (`tools/ps-client.js`, blind-only, `--mode`/`--no-tables`
  retired) and evaluator use the one blind profile (c = 0.4, 27,000). The
  `open` profile stays only to replay records made under it (forks, kifu).
- Wasm `BlindSearcher`/`ProtocolSearcher` default c is the blind profile's
  (was an implicit 1.0); `fromFork` defaults to its record's profile; the
  true-state research `Searcher` keeps 1.0 and is not reachable from a
  product door.
- `team_eval` refuses a spec without `:C` and stamps every record with
  `cond` (bot build fingerprint, both priors' fingerprints, turn cap, seed
  base, preview/ponder); resume refuses any other condition.
- Remaining difference, recorded rather than removed: the browser ponders
  (up to 10× the budget while the human thinks); the ladder and all
  fixed-work measurements do not.
- Checks: web e2e 42/42 and the worker-profile check pass against stand-in
  files; wasm node smoke/determinism/fork/kifu/solver pass. Rerun on the
  real artifacts before completion.

### Step 2 — opponent prior (screen done, 27k confirmation running)

`data/belief-pool-v3` = v2's 102 teams + カビバン 2020 restored (its
dominated verdict was never reconfirmed under c = 0.4), same allocation rule,
generated without reading selection labels. Held-out test, row X = 4 teams
with the arm's prior, column = blind with v3, blind:3000:0.4, seed base
20261101, 8 seeds (`tools/belief-pool-v3.py`, report in
`data/belief-pool-v3/measurements/`):

| Opponent group | v3 | v3 − v1 | v3 − strong-only | v3 − machine ×1 | v3 − unweighted |
|---|---|---|---|---|---|
| weak/pending human (13) | 0.697 | +0.054 [+0.025, +0.085] | +0.142 [+0.093, +0.184] | 0 (identical) | 0 (identical) |
| off-prior (20) | 0.558 | +0.002 [−0.018, +0.025] | −0.011 [−0.043, +0.019] | −0.013 [−0.029, +0.002] | +0.008 [−0.027, +0.041] |
| strong, unique preview (8) | 0.565 | +0.069 [+0.031, +0.110] | 0 (identical) | 0 (identical) | 0 (identical) |
| strong, colliding (1) | 0.688 | +0.047 [−0.031, +0.141] | −0.031 [−0.203, +0.125] | 0 | 0 |

The weak component costs nothing against strong teams and gains against
weak ones; machine factor and weighted fallback are unresolvable here (only
off-prior games differ). Confirmation at blind:27000:0.4 (seed base
20261102, 2 seeds): v3 and v1 on all cells, strong-only where it differs.

