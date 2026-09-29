# Team pool, opponent prior, and Nash rebuild

Status: approved design, implementation not started. Owner-requested handoff,
2026-09-29. Repository inspected at `23168bb`.

This file is the authoritative plan for this rebuild. Update it in place as
work proceeds; keep measurements in versioned data artifacts and link them here.
The current session only researched sources and agreed on the policy. It did not
import teams, classify candidates, run new matchups, or change shipped pools.

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
