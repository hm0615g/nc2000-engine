# Classification protocol v1 (frozen before confirmation)

Frozen 2026-09-29, after the pilot (`open:300`, 8 seeds, 10-team panel) and
while the discovery matrix was still running; no discovery or confirmation
score had been opened when these rules were written. Plan:
`docs/TEAM-POOL-REBUILD-PLAN.md` step 2. Changes after freezing are made
only as a new version of this file, with the reason.

## Conditions

| Item | Value |
|---|---|
| Bot | `crates/bot` at `e02f93a` (unchanged through `48a1042`); harness `crates/bot/examples/team_eval.rs` at `48a1042` |
| Rules | engine regulation `gen2nintendocup2000noohkostadium2strict`; Sleep/Freeze Clause as the engine implements them |
| Information | **open vs open** (`OpenAgent` both sides): both sheets public, picks and leads hidden until revealed — the default product mode. The opponent prior is not consulted in this mode, so stage-2 labels do not depend on the provisional prior. |
| Search | UCB state-keyed search, shipped rollout rules, discovery **300** iterations per decision, confirmation **1000**; equal budget both sides |
| Pondering | none (fixed iterations per decision) |
| Preview | live preview search; no baked pair tables |
| Turn cap | 500 turns → scored 0.5, counted and reported separately from ties |
| Pairing | game k of every cell shares the battle seed and both agent seeds (derived from seed base and k only); each k is played with the row team on p1 and on p2 |
| Discovery seeds | seed base `20260929`, k = 0..15 (32 games per unordered pair), full round robin of the 103 measured candidates + 4 held-out teams, plus the 4 calibration controls against all 107 |
| Confirmation seeds | seed base `20261001` (never used in discovery), k from 0 |

## Measured set

Eligible inventory records, one per loadout-duplicate group (`measuredAs`:
same six item+move loadouts, only DVs/training/gender differ; the member
whose source publishes training is measured and the others inherit its
label with that link as evidence).

## Panel and score

- **Panel** = eligible human-source measured teams + the 4 held-out page
  teams (legal human designs the intake does not admit). Machine-generated
  teams are candidates, never panel members.
- **Weights**: every variant cluster (inventory `variantCluster`) carries
  total weight 1, split equally among its panel members; held-out teams are
  their own clusters.
- **Panel score** of candidate X = weighted mean score of X against panel
  teams outside X's own variant cluster (so a candidate and its near
  variants face an identical panel).
- **Uncertainty**: cluster bootstrap over the seed index k (2000
  resamples; a resample keeps every cell's games for the drawn k values),
  percentile 95% intervals. Paired differences (X − Y) are bootstrapped the
  same way over games sharing (opponent, k, side).
- **Strata** (panel-team features, overlapping): `trap` (Mean Look/Spider
  Web with Perish Song or Baton Pass), `boom` (≥2 Explosion/Self-Destruct
  users), `spikes` (Spikes plus Roar/Whirlwind), `recovery` (≥3 members
  with Rest or a recovery move), `electric55` (a level-55 Zapdos, Raikou or
  Jolteon), `snorlax55`. A stratum is **important** when it carries ≥10% of
  panel weight.

## Metric calibration (skill `metric-calibration`)

Arms, all against the full panel at discovery seeds: ceiling `sample-07`
(the shipped Nash anchor), floor `ctl-floor-s07` (every move replaced by
the least useful legal ones), deletions `ctl-del-s07-boom` (the three
self-KO moves removed) and `ctl-del-s07-gene` (Berserk Gene removed),
confound `ctl-conf-s07-order` (display order reversed). With noise = the
larger of the ceiling's and confound's 95% half-width and |confound −
ceiling|, and break = the smaller of (ceiling − floor) and (ceiling −
boom deletion): the metric is usable for ranking if break ≥ 2 × noise
(`VALID`); the smallest single-element change it resolves is reported, and
differences below it are never read as orderings.

## Base labels

1. **strong** — confirmation panel score ≥ **0.45** (not materially below
   parity with trusted human teams; ±0.05 is the project's standing
   materiality bar). Candidates whose discovery 95% interval lies wholly
   above 0.45 are strong without confirmation; wholly below → weak without
   confirmation; any interval touching 0.45 goes to confirmation.
2. **weak** — below the strong bar and not dominated.
3. **dominated** — only when all three owner conditions hold:
   1. **replacement**: a team in the same variant cluster whose source is
      highly trusted (魔人島 serious entries, Historia Cup 7.5 top 8, Smogon
      hub samples; the rental DB and machine-generated teams do not
      qualify), with the same level allocation (the same species at 55,
      respectively 50–53), named with its exact source version;
   2. **inferiority**: replacement − candidate ≥ **0.05** on the panel at
      confirmation budget with the paired 95% interval excluding 0, and no
      important stratum where the candidate beats the replacement by ≥
      **0.10** with its interval excluding 0 (a unique advantage keeps the
      candidate out of `dominated`);
   3. **execution**: for every mon/move that distinguishes the candidate
      from the replacement, the confirmation games show the bot selecting
      that mon in ≥10% of games and using the distinguishing move when the
      mon is on the field; plus a read of at least two logs where the
      distinguishing element matters. If execution cannot be established
      the verdict is withheld.
4. **pending** — a confirmation interval still touching 0.45 after one
   extension (n doubled), or a dominated case whose evidence is
   incomplete. Pending teams join the opponent prior (they are not
   dominated) and stay out of the ordinary draw (they are not confirmed
   strong).

Nash membership is not a base label; it is decided in step 4 under the
product's Nash-door conditions (blind vs blind with the frozen prior) and
stored separately. A candidate needed by the equilibrium but labelled weak
is re-examined rather than forced.

## Stopping and budget

Discovery: 16 seeds, stopped early only if every candidate's interval is
already clear of the 0.45 band. Confirmation: 8 seeds (16 games) per
(candidate, panel team) cell at 1000 iterations for boundary candidates
and for every dominated comparison; one extension to 16 seeds if still
straddling.
