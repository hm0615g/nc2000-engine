# Selection protocol v2 (blind, c = 0.4)

Frozen 2026-09-30 after the opponent prior was frozen (plan step 2, evidence
in `data/belief-pool-v3/measurements/`) and before any stage A, B or C
selection score was opened. Parameters are the constants of
`tools/team-select.py` at the freezing commit (`SHORTLIST_TOP` 40,
`AUDIT_N` 6, `DISC_MIN` 0.52, `DISC_CAP` 36, `FLOOR` 0.50, `SIZE_MIN`/`SIZE_MAX`
15/30, two looks at z = 2.241, `BLOCKS_POOL` 240, `BLOCKS_PANEL` 32). Plan:
`docs/TEAM-POOL-REBUILD-PLAN.md` steps 3–5. `data/team-inventory-v1/PROTOCOL.md`
(v1, open-era) stays unchanged as the record of what v1 measured. Changes after
freezing are made only as a new version of this file, with the reason.

## Conditions

| Item | Value |
|---|---|
| Bot | build fingerprint `fnv1a64:c31d5635eb296d9a:m17e-solver-build-v3` (engine + bot sources, `crates/bot/build.rs`); every record carries it in `cond.bot` |
| Rules | engine regulation `gen2nintendocup2000noohkostadium2strict` as the engine implements it |
| Information | **blind vs blind** (`BlindAgent` both sides): the opponent's six are seen at preview, sets are inferred |
| Opponent prior | both sides `data/belief-pool-v3/belief-pool.json`, frozen in plan step 2 (`cond.belief_row` / `cond.belief_col` = `fnv1a64:ffa73c4245fe3774`) |
| Search | UCB state-keyed search, `RmConfig` defaults with rule UCB and hp_buckets 16 (the wasm/ladder configuration), **c = 0.4 on both sides**; screening **3000** iterations, product budget **27000** |
| Pondering | none (fixed iterations); browser pondering is covered by the sensitivity check below |
| Preview | live preview search, no baked tables |
| Turn cap | 500 turns → 0.5; ties → 0.5; caps and ties counted and reported separately |
| Pairing | `crates/bot/examples/team_eval.rs`: game k of every cell shares the battle seed and both agent seeds (seed base and k only); each k is played with each team on p1 and on p2. A pair is played once per k in one orientation (`tools/team-select.py` writes unordered pairs), so no game is duplicated |

A directory passed to `tools/team-select.py` must hold one condition; the
harness refuses to resume under a different one.

## Candidates, panel, challenge panel

- **Candidates**: the 103 eligible measured representatives of
  `data/team-inventory-v1` (`panel.json` `candidates`). Loadout duplicates
  inherit their representative's outcome.
- **Panel** (`panel.json`, frozen): every eligible measured **human-source**
  record — 魔人島 serious entries, Historia Cup 7.5 top 8, Smogon hub samples
  (including the reconstructed Bright Powder originals) and the community
  rental transcriptions: 78 teams in 58 variant clusters. "Trusted-human
  panel" in the plan means this panel; machine-made META-NASH teams are
  candidates, never panel members. Each cluster carries weight 1/58, split
  equally among its members.
- **Score** `s(t) = Σ_p w_p μ(t, p)` over the whole panel, identical for
  every candidate. A candidate that is itself a panel member takes
  `μ(t, t) = 0.5`, the exact value by symmetry. Nothing else is removed:
  near variants and lineage relatives are played.
- **Challenge panel** (never part of `s`): the 4 held-out page teams, the
  16 off-prior chimeras, and every candidate not selected.
- **Strata**: v1's definitions (`trap`, `boom`, `spikes`, `recovery`,
  `electric55`, `snorlax55`); all six carry ≥10% of panel weight, so all are
  important. Per-stratum scores are reported for every measured candidate and
  for the pool; they are not an admission rule.

## Draw rule

`q_S(t)`: each variant cluster among the selected members is drawn with equal
probability, split equally among its selected members. The same rule is used
for `V(S) = Σ q_S(t) s(t)`, browser Random (human and bot) and ladder
`pool:random`.

## Estimator and uncertainty

The unit of pairing is the shared seed block: panel opponent p, seed k, both
sides. `ŝ(t)` is the stratified mean over panel opponents with the fixed
weights. Its variance is `Σ_p w_p² σ̂² / n_p`, where σ̂² is the within-opponent
game variance pooled over that candidate's cells (degrees of freedom reported)
and n_p the games against p. Paired differences use per-game differences over
the shared (opponent, k, side). Intervals are normal 95%. A percentile
bootstrap over seed blocks needs ≥ 8 blocks per cell, which the product-budget
stages cannot afford (1–4 blocks per cell); this is the reason for the
analytic form. Deletion effects `Δ_remove(t, S) = V(S∖t) − V(S)` are linear in
the scores with the actual draw weights recomputed after removal; their
variance treats candidates' scores as independent, which is conservative for
differences.

## Metric calibration (skill `metric-calibration`), stage A

Arms against the full panel: ceiling `sample-07`, floor `ctl-floor-s07`
(every move replaced by the least useful legal ones), deletions
`ctl-del-s07-boom` (self-KO moves removed) and `ctl-del-s07-gene` (Berserk Gene
removed), confound `ctl-conf-s07-order` (display order reversed). The five arms
play **8 seed blocks** (k = 0..7) at stage A, so the verdict speaks to the
metric rather than to the 2-block sample candidates get there. noise = max
(95% half-width of ceiling and confound, |confound − ceiling|); break = min
(ceiling − floor, ceiling − boom deletion). `VALID` if break ≥ 2 × noise,
`WEAK` if break > noise, `INVALID_STUCK_FLOOR` if the boom deletion is within
the ceiling's half-width, otherwise `INVALID_NOISY`. Calibration is read before
any candidate score. If it is not `VALID`, stage A orders the stage B queue but
excludes nobody (every candidate goes to stage B). The resolution of a 2-block
stage A score (2 × its 95% half-width) is reported; smaller differences are not
read as orderings. The four controls also play stage B (k = 0) as a
supplementary product-budget check, reported and not gating.

Pilot read before broad use: the paired stage A difference sample-16 − ポニョ
(rental-cban-13) with its interval, against that resolution.

## Stage A — screening, blind:3000:0.4

- Seeds: base **20261111**, k = 0, 1 (4 games per pair).
- Cells: every pair of panel members; every machine candidate and every
  calibration control against every panel member (`cells-a`, 5265 cells);
  the calibration arms' cells extend to k = 7.
- Shortlist for stage B = union of: the **top 40** candidates by stage A score;
  the **old high-budget specialists** (positive weight in
  `data/meta-nash-v2/solution-blind3000-do2.json` or `solution.json`, plus
  META-NASH v1's sample-07/08/10); the **owner-reported case** sample-16 with
  its source ポニョ (rental-cban-13) and its Bright Powder original, and the
  カビバン 2020/2022 pair; a **reversal audit** of 6 candidates drawn with seed
  20261112 from those not otherwise shortlisted.
- Stage A never admits a candidate.

## Stage B — product-budget discovery, blind:27000:0.4

- Seeds: base **20261121**, k = 0 (156 games per candidate).
- Every shortlisted candidate, and the four calibration controls, against
  every panel member.
- Reversal audit: if an audit candidate scores ≥ 0.55, the 5 highest stage A
  candidates below the shortlist are added to stage B once, and reported.
- Discovery selection `S_disc`: candidates with stage B score ≥ **0.52**, best
  first, at most **36**; then the harm trim below on stage B data.
- Execution audit for shortlisted combo teams (trap, Belly Drum, Perish,
  Baton Pass, Spikes + phazing): selection frequency and distinguishing-move
  use from the game records, with at least two log reads where a combo team
  lands near a decision boundary.

## Stage C — confirmation, blind:27000:0.4, fresh seeds

- Seeds: base **20261131**. Look 1: k = 0, 1 for every `S_disc` member. Look
  2: k = 2, 3 for members undecided at look 1.
- **Admission floor**: lower bound of `s(t)` > **0.50** against the panel, on
  stage C data only, with z = 2.241 at each look (α = 0.025 one-sided split
  over two looks). Upper bound < 0.50 at a look → not selected. Undecided after
  look 2 → `pending`.
- **Harm trim** among floor-passers: a member is clearly harmful when the lower
  simultaneous bound of `Δ_remove(t, S)` is above 0, Bonferroni over |S|
  (one-sided α = 0.025/|S|). Remove the most harmful (largest z), recompute
  draw weights, repeat until none qualifies. A point-estimate increase never
  removes a member. The test is on the deletion effect with the real draw
  weights, so it has the same power as a test of the member's own deficit
  against the rest; no pool-level margin is added.
- **Size**: target 15–30. Above 30: keep the 30 highest stage C scores and
  report members whose rank is uncertain at the cut. Below 15: report the
  shortfall; the floor is never lowered after results. One escape is
  preregistered: if fewer than 15 remain, stage B candidates with score ≥ 0.50
  outside `S_disc` may be confirmed on the reserved block **20261181** under
  the same two-look rule, once, followed by the harm trim on the union. No
  other hold-out reuse.
- Base labels written to `selection.json`: `selected` (the final S),
  `pending` (undecided), `not-selected` (with the reason: below the stage C
  floor, harmful, not an entrant at stage B, or not shortlisted at stage A).
  `not-selected` is not `dominated` and does not remove a team from the prior or
  the reference inventory. The v1 three-condition dominated rule still governs
  `dominated`; no dominated verdict is issued in this protocol without its
  replacement, confirmed inferiority and execution evidence.

## Pool comparisons

`V` of the final S (stage C scores), of the local 79-team `team-pool-v1`
(its `drawWeight`) and of the bundled 32 (`meta-pool-v0`, uniform, the old
Random draw), all on the same panel at blind:27000:0.4. Old-pool members with
stage B data use it (a fixed pool's average of discovery scores is unbiased).
Members without 27k data are covered by a sampled estimate: 240 seed blocks per
old pool, each drawing a member by its draw weight restricted to the
unmeasured members and an opponent by panel weight, seed base **20261141**.
Reported: V with SE, differences, per-party scores, deletion effects, stratum
scores, caps and ties, and the reason for every inclusion and exclusion.

## Pondering sensitivity

Bounded extra work: the candidate plays `blind:54000:0.4` (twice the budget,
standing in for ponder time during the human's move) against the panel at
`blind:27000:0.4`. Candidates: final S, pending, and the 5 best stage B
candidates outside S. Each plays the same 32 seed blocks drawn once from the
panel by weight (seed base **20261151**). A **material reversal** is a
selected member whose 95% upper bound is below 0.50 there, or a non-selected
candidate whose 95% lower bound is above 0.50. Any material reversal is
resolved before the pool is claimed to fit browser play; otherwise the check is
reported with its resolution (it cannot see small reversals).

## Nash (plan step 5)

- Game: the final S as strategies, blind vs blind on the frozen prior, c = 0.4.
- Discovery: blind:3000:0.4 round robin of S, 16 seed blocks per pair (stage A
  games reused for panel pairs, extended from k = 2), solved with RM+
  (`tools/nash-solve.py`) and its seed bootstrap.
- Support-relevant set R: weight ≥ 0.01 in the 3000 solution or in ≥ 10% of
  bootstrap solutions. R's round robin at blind:27000:0.4, 16 blocks per pair,
  seed base **20261161**; solve; every S member outside R plays the 27k mixture
  (16 blocks); a member scoring above 0.5 joins R and the game is re-solved
  (at most two rounds).
- Challenge on fresh data: every non-selected candidate, the held-out page
  teams and the off-prior chimeras against the final mixture at 3000 (seed
  base **20261171**, 16 blocks), then the 10 highest and every challenger whose
  95% lower bound exceeds 0.45 at 27k (seed base **20261172**, 32 blocks). An
  exploiter is not declared absent because a wide interval overlaps 0.5.
- The artifact stores weights separately from base labels; scope is the finite
  game over S at the stated budgets.

## Reserved seed bases

20261181 (stage C escape, above), 20261191 (unallocated).

## Amendment v2.1 (2026-10-01, after the pondering check flagged candidates)

The pondering check flagged three material reversals (`mjj-2010-garap-a`
upper bound below 0.50; `lineage-fine-s07` and `sample-22-orig` lower bounds
above 0.50 by 0.001). v2 required resolution but did not say how; this
amendment adds data only and changes no admission. Each flagged candidate
plays 64 fresh blocks on a new shared panel schedule (seed base **20261191**,
the reserved base) under the same 2× condition; the reversal **stands** only
if those fresh blocks alone meet the same definition. Catalog membership
stays decided by stage C (product budget, no pondering, the ladder's
condition). A standing reversal is reported as a browser-fit conflict: the
catalog is then not claimed to fit browser play as well as the ladder for
that party.
