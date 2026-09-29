# Team pool, opponent prior, and Nash rebuild

Status: **implemented 2026-09-29 (local commits, not pushed or deployed)** —
steps 1–5 done; results and open owner decisions in [§Results](#results-2026-09-29).
Owner-requested handoff, 2026-09-29. Repository inspected at `23168bb`.

This file is the authoritative plan for this rebuild. Update it in place as
work proceeds; keep measurements in versioned data artifacts and link them here.

## Objective and agreed policy

Import candidates from [魔人島 / 俺のパーティ軍団](https://majinjima.ma-jide.com/party/index.htm)
into the existing candidate universe, then select teams, rebuild the opponent
prior, and recompute Nash using the current bot. The owner identifies this source
as trusted by players. Source trust is an admission signal, not an automatic
strength verdict.

Final presentation has four labels:

| Label | Ordinary bot team draw | Opponent prior | Retained as reference |
|---|---|---|---|
| nash | Yes; also weighted draw in Nash mode | Yes | Yes |
| strong / 強い | Yes | Yes | Yes |
| weak / 弱い | No | Yes | Yes |
| dominated / 劣後 | No | No | Yes |

Nash is a subset of strong, not a higher scalar strength grade. Model the base
classification separately from Nash support and weights, even if the user sees
one of four labels. A zero Nash weight does not make a team weak or dominated.
If a supposedly weak candidate proves necessary to the equilibrium, revisit its
classification rather than enforcing the earlier label.

Weak means below the bot-use standard but not replaceably dominated: it may have
distinct matchups, sets, or win conditions, or be difficult for this bot to pilot.
Prior inclusion does not mean equal weighting. Neither self-play win rate nor
Nash weight estimates human usage frequency.

Keep an explicit pending review state during the work. It is not a fifth final
quality label. Do not turn uncertainty into a dominated verdict. Format-invalid
or out-of-format source material is excluded by a separate eligibility decision,
not assigned the weak label and admitted to the prior.

## Operational definition of dominated / 劣後

All three owner-agreed conditions must hold:

1. A similar team exists in a highly trusted source. Name that replacement and
   its exact source version. Similarity includes level allocation, roles, win
   conditions, and matchup coverage; species overlap alone is insufficient.
2. The candidate has a clearly lower win rate than that replacement against a
   common opponent panel under common conditions. Require uncertainty-aware
   confirmation. Retain or review candidates with meaningful unique advantages
   against important opponents even when their overall average is lower.
3. The inferiority is not explained by the bot's inability to execute a gimmick
   or combination. Inspect actual selection and play. If execution competence
   cannot be established, withhold the dominated verdict.

This is replaceable inferiority in a declared opponent environment, not a proof
of game-theoretic dominance against every possible team. Every verdict must
record the replacement, opponent panel, conditions, uncertainty, and execution
evidence. A poor average result without a trusted close replacement is not enough.

Do not substitute a species viability ranking, item popularity, source category,
or the bot's dislike of a move for these three conditions.

## Verified starting point

- `data/meta-pool-v0/meta-pool.json` currently has 32 teams: five HC7.5 teams and
  27 sample teams. Random selection for ordinary/open and blind play is uniform
  (`web/src/pool-pick.ts`). The source README retains historical counts; inspect
  the JSON when determining current contents.
- `data/meta-nash-v1/pool-artifact.json` draws sample-07/08/10 with stored weights
  0.575/0.222/0.201, normalized by the loader. This is a separate own-team artifact.
- `data/belief-pool-v1/belief-pool.json` contains 87 teams. Nash uses it for
  opponent belief. Ordinary blind currently uses the active own-team pool for
  belief too (`web/src/app.tsx`). That coupling must be addressed for the new
  strong-only draw and strong-plus-weak prior policy.
- `web/src/data.ts` has a historical minimum of 32 for the belief pool loader.
  Audit cardinality assumptions, IDs, ordering, and consumers during integration.
- `crates/bot/src/belief.rs` also embeds community rental sets for fallback.
  Audit this and other imputation sources: removing a dominated team from one
  JSON must not silently leave the same team active through another source.
  Shared individual sets can remain where independently justified; do not ban
  every move or species that appeared on a dominated team.

The player's concrete complaint identifies **sample-16**:

| Pokémon | Level | Item | Moves |
|---|---:|---|---|
| Golem | 50 | Soft Sand | Earthquake / Fire Blast / Explosion / Roar |
| Gengar | 55 | Miracle Berry | Shadow Ball / Thunderbolt / Fire Punch / Ice Punch |
| Poliwrath | 50 | Black Belt | Hydro Pump / Submission / Return / Belly Drum |
| Exeggutor | 50 | Gold Berry | Psychic / Hidden Power Bug / Explosion / Leech Seed |
| Porygon2 | 55 | PRZ Cure Berry | Return / Ice Beam / Curse / Recover |
| Snorlax | 50 | Leftovers | Double-Edge / Earthquake / Self-Destruct / Curse |

Both reported details (level-50 Poliwrath and Soft Sand Golem) occur in this one
team, including in the saved upstream transcription
`data/meta-pool-v0/raw/samples-27.txt`. It is in the ordinary pool and belief pool,
not the shipped Nash mixture. Prioritize its review, but the report does not
itself satisfy the three conditions for dominated.

Other concrete provenance checks:

- Compared with the linked original Mario Party 13, sample-07 changes Electrode's
  Light Screen to Thunder Wave, Marowak's Bonemerang to Earthquake, Snorlax's Belly
  Drum to Curse, and Cloyster's PRZ Cure Berry to Gold Berry. Compare exact versions;
  these differences do not establish that either version is inferior.
- Original Bright Powder substitutions are recorded for sample-09/11/14/15/16/17/22/25
  in `tools/build-meta-pool.js`. The current format permits Bright Powder. Audit
  original versions instead of silently treating edited translations as originals.

## Source intake

The index distinguishes serious, experimental, minor-species, and theme teams.
On 2026-09-29, its HTML contained 30 entries classified by the author as
「かなりガチなパーティ」 (`#ffaaaa`), of which 28 were not marked OHKO-allowed.
These are entries, not necessarily distinct complete teams. Some links select
different versions on the same page. Recheck the live index before intake.
The HTML declares EUC-JP; preserve Japanese text correctly.

Start with that source-defined subset. Read individual descriptions and versions;
do not bulk-import the entire site or treat published battle logs as an unbiased
win-rate sample. Preserve the original six-member composition, levels, moves,
items, DVs, gender, training, and happiness where supplied. Record assumptions
for missing values. Do not mix a current table with older log variants. Source
table/prose contradictions require explicit resolution or pending status.

These eight pages were identified as useful first comparisons, not approved
automatic inclusions or a fixed quota:

| Source | Reason to inspect |
|---|---|
| [マリオパーティ13](https://majinjima.ma-jide.com/party/2013/mario.htm) | Direct comparison with sample-07; offensive multi-ace team |
| [サンダー昆布シリーズ](https://majinjima.ma-jide.com/party/2014/thdrcomb.htm) | Spikes and phazing; use the specified completed version |
| [秀才カビ’23](https://majinjima.ma-jide.com/party/2023/vakabi.htm) | Snorlax/Miltank; documented third-party tournament use |
| [速攻カビガラポリ](https://majinjima.ma-jide.com/party/2024/kabiliz.htm) | Belly Drum, paralysis, and screens; fixed-team tournament result |
| [ソード＆シールド](https://majinjima.ma-jide.com/party/2020/rraikou.htm) | Reflect Raikou and Marowak |
| [受けケンタ](https://majinjima.ma-jide.com/party/2022/ukekent.htm) | Defensive support for Substitute Tauros |
| [新生ハピムウマvol.1](https://majinjima.ma-jide.com/party/2025/hapim.htm) | Protect/Perish Misdreavus and team-level combinations |
| [おぢいちゃんII’20](https://majinjima.ma-jide.com/party/2012/goinkyo.htm#2020) | Alakazam/Heracross; Encore and Perish interactions |

Further candidates include ポリガラクラゲ and 藝術道場. There is no agreed final
team-count target. An earlier 8–12-team suggestion was exploratory, not a quota.
Validate against the repository's actual operational rules, including the sleep
rule described in README; do not infer battle semantics from the format name.

## Work sequence and acceptance evidence

### 1. Inventory and reproducible import

Inspect the working tree and current implementation before changing anything.
Read applicable skills. Reuse existing import/validation machinery where it fits.
Create a versioned candidate inventory containing old teams and new source teams.
Store stable IDs, normalized sets, source URL/version, retrieval date or content
fingerprint, source family, explicit deviations, and validation findings.
Deduplicate exact sets while retaining all provenance; link near variants without
merging away meaningful differences. Separate source eligibility from quality.

Deliverable: import can be reproduced, each admitted team is legal and traceable,
and every deviation from its source is visible. Keep raw retrievals minimal and
avoid copying full explanatory articles or player logs into publication artifacts.

### 2. Calibrate selection, then classify

Inspect a few concrete games and the source's intended selection/win conditions
before building a large ranking system. Establish comparable close-team pairs,
starting with sample-16 if a justified replacement is found and sample-07 versus
the Mario original. Follow metric-calibration before using a metric to steer
pruning. Do not run a full expensive matrix before the scoring and pilots work.

Freeze an evaluation protocol before confirmation results are inspected:

- A common opponent panel with declared weights and important matchup strata;
  include existing teams, new trusted teams, and held-out human-source teams.
- Current bot revision, rules, information mode, provisional prior, search budget,
  pondering setting, preview behavior, and turn-cap treatment.
- Paired battle seeds and side swaps, separate discovery and confirmation seeds,
  uncertainty method, material-difference thresholds, and stopping rules.
- A strong/weak admission standard and evidence required for each of the three
  dominated conditions. Thresholds are not yet agreed; make them explicit before
  applying them. Similarity and source reputation alone cannot set this grade.

Use equal information and compute for symmetric product-strength comparisons.
Do not treat a full-information adversary result as an ordinary blind duel.
Separate draws and turn caps. Inspect execution failures in selection and key
combinations; a bot-limited team may be weak for deployment without being dominated.
Use explicit exposure to combinations where ordinary self-play fails to field them.

Record base labels, replacement IDs where applicable, and linked evidence.
Nash-support membership is deliberately deferred. Stage-2 classifications are
provisional because rebuilding the prior can change play and matchup results.

### 3. Rebuild and freeze opponent prior

Include strong and weak eligible candidates; exclude dominated candidates.
Keep dominated records in the reference inventory only. Prevent duplicated source
families or many near variants from gaining weight merely by entry count.
Audit the actual belief representation: a team candidate pool, team weights, and
a per-species move prior are different things. Identify which mechanisms the
product consumes before choosing an implementation; do not assume JSON weights
are honored by current belief code.

Calibrate probability allocation against the intended human opponent environment,
not self-play strength or Nash weights. Compare defensible allocations while
holding own-team draws and opponent tests fixed. Explicitly test strong human
teams so the weak component does not impose an unnoticed strength penalty. A
strong-only arm may be a diagnostic comparator, not an unannounced reversal of
the agreed strong-plus-weak prior policy. Report any conflict with that policy.

Deliverable: a versioned, validated prior with documented support, allocation,
consumers, and held-out results. Freeze it before the decisive matchup matrix.

### 4. Reconfirm classifications and solve Nash

With that prior and the current bot fixed, recheck classification-sensitive
comparisons and measure the strong candidate matrix. Reuse the existing Nash
solver/harness where possible, but inspect its information conditions and hardcoded
paths. Historical matrices are background evidence, not current measurements.
Keep old Nash as a baseline, not a privileged support set.

Solve the finite candidate game, refine uncertain support-related matchups, and
check sensitivity to sampling noise and product-relevant search budgets. Store
weights separately from base labels. Do not force a three-team solution or call
the result an equilibrium of the entire game.

Challenge the mixture with weak, dominated, and held-out candidates using fresh
confirmation data. If an excluded candidate exposes a gap, revisit classification
and replacement evidence. If prior support/allocation changes, version it and
recompute affected results instead of mixing measurements across priors.

### 5. Generate products and integrate

Generate from one classified inventory:

- Ordinary own-team pool: strong, including Nash members.
- Opponent prior: strong plus weak, with its calibrated allocation.
- Nash artifact: support and weights over strong teams.
- Reference inventory: all source records, eligibility decisions, classifications,
  and explicit dominated-to-replacement links.

Keep own-team draws independent from opponent belief in both default and custom
pool flows. Audit web, worker/wasm, native/ladder, evaluator, and fallback consumers.
Invalidate old index-based preview tables when IDs/order/sets change; do not reuse
answers keyed to a previous pool. Preserve stable identification and distinguish
ordinary draw probabilities, prior probabilities, and Nash probabilities.

Ordinary draw weighting is not yet agreed; choose and document it separately.
Do not silently change human-selectable team availability or add classification
UI merely because these internal artifacts exist. Follow browser skills if UI
work becomes necessary. Public push/deployment is a separate publication action;
this handoff request does not instruct publication.

## Existing entry points

- Prior study and its information-condition caveats: [EXP-prior-exploit.md](EXP-prior-exploit.md).
- Previous mixture, matrix, and gates: [META-NASH-V1.md](META-NASH-V1.md),
  `crates/bot/examples/meta_nash.rs`, `prior_exploit.rs`, and `prior_br.rs`.
- Current pool source/conversion: `tools/build-meta-pool.js`,
  `data/meta-pool-v0/README.md`, `tools/parse-community-rentals.js`.
- Source eligibility lessons: `data/community-rentals-v0/ASSESSMENT.md`.
- Belief implementation: `crates/bot/src/belief.rs`, `prior.rs`, and `preview.rs`.
- Explicit combination exposure: `crates/bot/examples/combo_gauntlet.rs`.
- Product loading/draws: `web/src/data.ts`, `app.tsx`, `team-pool.ts`,
  `pool-pick.ts`, and `nash-mix.ts`.
- Existing checks to inspect: `crates/conformance/tests/meta_pool.rs`,
  `web/tests/pool-swap.spec.ts`, and `web/tests/nash.spec.ts`.

## Completion criteria

Every admitted candidate has traceable exact sets and a supported classification;
every dominated verdict names a trusted substitute and satisfies all three
conditions. Unresolved cases are reported rather than hidden by forced grading.
Generated artifact memberships match the agreed table, and source/family duplicates
do not accidentally change distributions. The fixed-prior Nash result survives
independent challenges within declared uncertainty and scope. Product consumers
load the intended artifacts, stale preview data cannot be used, and relevant
validation/integration checks pass. The final report states the four groups,
Nash weights, prior allocation, replacement decisions, and remaining limitations.

## Results (2026-09-29)

All numbers are win shares for the team named first; intervals are 95%
seed-index bootstraps. Raw per-game logs stay in `tmp/team-eval/`
(gitignored, local to the machine that ran them); committed measurements
hold per-cell totals and every result, and each run is reproducible game for
game from its recorded agent spec and seed base
(`crates/bot/examples/team_eval.rs` is deterministic per game).

**Measurement conditions differ from the shipped bot.** Budgets were 300 /
1000 / 3000 iterations with no pondering; the product plays 30,000 (open,
c = 1.0) or 27,000 (blind, c = 0.4), and the web ponders
(`data/search-profiles.json`, `web/src/bot-worker.ts`). Open-mode runs used
the product's c = 1.0; **every blind run (prior tests, Nash) used c = 1.0,
not the blind profile's 0.4**. The harness now takes `blind:ITERS:C`;
re-measuring under `blind:ITERS:0.4` is the first open item below.

### 1. Inventory and import

- `data/team-inventory-v1/` ([README](../data/team-inventory-v1/README.md)):
  133 records, 115 eligible, 103 measured after collapsing 12 loadout
  duplicates. 魔人島: the index held 30 serious entries (28 non-OHKO), which
  gave 31 records (ガラプラス publishes two teams) plus 4 page tables no
  serious entry designates (reference only). Each page's hidden
  rental-utility field was decoded (exact dex numbers, DVs, stat exp, move
  and item numbers) and cross-checked against the visible table; two
  disagreements were resolved explicitly (`tools/import-majinjima.js`
  `RESOLUTIONS`).
- Ineligible: 2 OHKO-rule entries; 3 entries whose Donphan uses Encore
  (only obtainable from a Japan-only Phanpy event, which the format
  rejects); in the shipped prior v1, 6 rental entries that are
  format-illegal or built for another rule (OHKO-allowed, Little Cup,
  Metronome-only, Item Clause).
- Bright Powder originals of the 8 edited Smogon samples were
  reconstructed; two of them (09, 22) are independently confirmed by
  rental transcriptions.
- **sample-16's source**: it is a translation of Party Box #20 「ポニョ」
  by stoic (rental-cban-13 transcribes the original) with five set
  changes, including the two a player reported (Poliwrath 55→50, Golem
  Quick Claw→Soft Sand). Recorded as `lineage` in the inventory.

### 2. Classification ([PROTOCOL](../data/team-inventory-v1/PROTOCOL.md), frozen before any score was opened)

- Metric calibration (open:300, 82-team human panel): ceiling sample-07
  0.661, boom-deletion 0.501, floor 0.064, confound (order reversed) 0.637;
  noise 0.023 → `VALID`; Berserk Gene deletion is below resolution.
- Discovery: 195,168 games (open:300, 32 per pair). Confirmation: 68,768
  games (open:1000) for the boundary band and every dominated comparison.
- **Labels: 79 strong, 21 weak, 2 pending (sample-18, rental-cban-12),
  1 dominated.**
- **Dominated**: 魔人島 カビバン (2020) → replaced by カビバン'22 (same
  author, Snorlax Zap Cannon→Curse): +0.063 at open:1000 (CI excludes 0),
  no unique advantage (incl. the nine Miltank teams the author aimed Zap
  Cannon at), execution established from logs
  (`condition3-log-reads.json`). Rechecked blind (blind:1000, c = 1.0):
  +0.034 [+0.007, +0.056] — same direction, below the materiality bar
  there.
- **Withheld** (conditions 1–2 met, execution not established): Mario13 ←
  sample-07 (+0.073; Belly Drum games win 0.36 vs 0.64 without), 王パCH流
  アレンジ ← sample-11-orig (+0.074; Sandstorm set on entry regardless of
  matchup), HC7.5 3rd ← sample-08 (+0.051; Swagger+Substitute misplayed).
- **sample-16 (owner decision)**: strong (0.545 at open:1000). Its original
  ポニョ beats it by +0.070 [+0.032, +0.113], but the original is published
  only by Party Box / the rental DB (outside the protocol's highly trusted
  set) and allocates levels differently. If Party Box counts as highly
  trusted and a changed level allocation is acceptable, sample-16 can be
  re-tested as dominated.
- Execution: the bot rarely fields trap/support ghosts (Misdreavus in the
  ハピムウマ family 5–8% of games); forcing the Blissey+Misdreavus+ace core
  did not lift the weaker family members
  (`measurements/forced-exposure-hapimuuma.json`).

### 3. Opponent prior ([belief-pool-v2](../data/belief-pool-v2/README.md), frozen)

- Mechanism change (`crates/bot/src/belief.rs`): a pool whose teams carry
  `weight` samples preview-consistent candidates by weight and falls back to
  the weighted-mode loadout per species; unweighted pools are bit-identical
  to before.
- Allocation: cluster-normalized, machine-made teams ×0.25. Fallback rule
  chosen on the human spectator corpus (revealed-move recall 0.546/0.543 vs
  0.507/0.505 for file order on the two halves) and checked in battle
  (+0.016 [−0.018, +0.048], `measurements/fallback-rule.json`).
- Held-out test at blind:1000 (`measurements/prior-heldout.json`): vs a
  strong-only prior, +0.100 against weak teams and +0.038 against
  off-prior teams; against strong teams bit-identical (no strong team's
  preview collides with a non-strong member), so the weak component costs
  nothing there. At or above the shipped v1 prior in every group.

### 4. Nash ([meta-nash-v2](../data/meta-nash-v2/README.md))

- Condition: blind vs blind on the frozen prior (the Nash door's
  information structure, at c = 1.0 — see the conditions note above).
  79-team game at 300 (98,592 games) → 18
  support-relevant teams at 1000 and 3000 (64 per pair) → two
  double-oracle rounds at 3000 (23 teams).
- The equilibrium moves with budget: 300 anchors on sample-07; at 3000
  魔人島 サンダー昆布 beats sample-07 0.78 and becomes a core member.
  Shipped (3000, 23 teams): rental-cban-8 0.208, sample-14 0.204,
  サンダー昆布 0.169, ソード＆シールド 0.154, 8番道路ジム 0.127,
  sample-14-orig 0.044, sample-12 0.041, sample-07 0.028,
  HC7.5 tsuru 0.025. The old shipped mixture (07/08/10) is exploitable by
  0.109 in this game.
- Challenge (`challenge-blind3000.json`): the 100 teams outside the game.
  No eligible candidate's interval clears 0.5 at the achieved sample size
  (4–160 games per support team); the highest are rental-cban-20 0.595
  [0.467, 0.719], セミスター (labelled weak) 0.589 [0.486, 0.693],
  ポリガラクラゲ 0.566. Off-prior chimeras 13, 07 and 16 (built from halves
  of human teams) do clear it (0.60–0.63) — residual exploit surface
  outside the candidate universe.
- Scope: the equilibrium of a finite 23-team game at 3000 iterations. The
  top of this metagame is flat and the weights are not stable across
  budgets or double-oracle rounds; this is not claimed to be an
  equilibrium of the whole game, and 30k-iteration behaviour is not
  measured directly.

### 5. Products and consumers

| File | Consumers |
|---|---|
| `data/team-pool-v1/team-pool.json` (79, cluster-balanced draw) | web `/` "Random" bot pick, plain `?blind`, ladder `--team pool:random` |
| `data/belief-pool-v2/belief-pool.json` (102) | every blind searcher: `?blind` (also with a user pool file), `?nash`, `?solver`, `?evaluate`, ladder blind mode |
| `data/meta-nash-v2/pool-artifact.json` (9) | `?nash` draw, `?evaluate` default opponents |
| `data/team-inventory-v1/reference.json` | reference only |

- Start-screen lists are unchanged (bundled 32). Draws from the new pool
  carry no pool index, so no baked pair table is ever read against them.
- Checks: `cargo test -p conformance --test team_products` (memberships,
  sets, weights, dominated links, legality and play-out); web e2e 44/45 —
  the failure (`evaluate.spec.ts` "exposes no technical input by default")
  predates this work: the 2026-09-27 test forbids 反復 while the test build
  prints 「実際の探索量は300反復です」.
- Not published: push/deploy is a separate owner action.

### Open items

State: implemented on local `master`, not pushed (push = Pages deploy =
owner approval). Owner decisions:

1. sample-16 vs its original ポニョ (trust in Party Box; level allocation).
2. Whether the start-screen lists should gain the new teams or drop
   weak/dominated ones.
3. Publication.
4. Product-spec divergence the owner raised on 2026-09-30: the ladder bot
   and the web bot share the search core and `data/search-profiles.json`
   but play differently — web `/` is open-sheet (30,000, c = 1.0), web
   `?blind`/`?nash` and the ladder are blind (27,000, c = 0.4); only the web
   ponders; the ladder rebuilds state from the Showdown protocol
   (`WasmProtocolSearcher`, `tools/ps-client.js`) while the web mirrors the
   engine battle (`WasmBlindSearcher`, `web/src/bot-worker.ts`). Whether to
   converge them, and how many URL doors to keep (`/`, `?blind`, `?nash`,
   `?solver`, `?evaluate`, `?fork`), is undecided.

Measurement work, in priority order (all commands resume and are
deterministic; inputs are in `data/team-inventory-v1/eval/`):

1. **Re-measure the blind results under the shipped profile**
   (`--agent blind:ITERS:0.4`): the prior held-out test
   (`python3 tools/prior-heldout-setup.py`, arms as in
   `measurements/prior-heldout.json`) and the Nash game (round robin of
   `data/meta-nash-v2/solution-blind3000-do2.json` `ids` at 3000, then
   `tools/nash-solve.py`, `tools/nash-challenge.py`). If the support or the
   prior verdicts move, regenerate with `node tools/build-team-products.js`.
2. A product-budget spot check of the Nash support (e.g. 30,000 iterations,
   32 games per pair ≈ 1–1.5 h at 11 threads on this Mac).
3. セミスター (weak) scores 0.589 against the Nash mixture on 72 games —
   extend before any relabel; the two pending teams (sample-18,
   rental-cban-12) stay out of the ordinary draw until resolved.
