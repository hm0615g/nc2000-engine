# Frozen-prior and playable-party training audit (2026-11-03)

Historical snapshot at commit `14cf3590`, before the owner-requested
[training-v3 repair](../training-v3/README.md). The commands below reproduce
this snapshot from that checkout; current products use the repaired policy.

Follow-up to `docs/ICO-SESSION-FINDINGS-2026-11-03.md`. This audit changes no
production prior, playable sets, engine, WASM, or preview artifact. It establishes
candidate support and stat discrepancies, not an action-quality or win-rate gap.

## Reproduction

Run from the repository root:

```sh
cargo run --quiet -p nc2000-bot --example audit_prior_training -- data/prior-training-audit-v1/report.json
node tools/audit-party-training.js data/prior-training-audit-v1/party-report.json
node --test tools/optimize-low-attack.test.js
node tools/build-team-products-v2.js --out-root tmp/prior-training-rebuild
```

The native report uses the actual engine stat constructor, validator, `Observer`,
`Belief` and determinizer. All 221 original prior teams and their EV-255 controls
pass format validation. Controls retain DVs, gender, moves, items and level;
therefore these comparisons do not misclassify Hidden Power or low Attack DVs
as deficient training. Most reveal probes are synthetic single-mon observations.
The Rain Dance witness additionally executes a legal engine turn.

The independent JavaScript report inspects all 198 catalog sets, Gen 2 move
categories, Hidden Power constraints, minimum Attack DVs, public signatures
and the active four-team Nash artifact. The 11 existing optimizer tests pass.
Rebuilding the three team products into scratch produces byte-identical files.
Both reports record input SHA-256 hashes.

## 1. The frozen prior excludes ordinary maximum-training variants

`data/belief-pool-v3/belief-pool.json` has 221 entries: 187 human, 25 machine,
and 9 tagged evolved. **17 entries / 24 set occurrences have a stat below the
EV-255 control outside Attack.** Fifteen entries are machine-generated;
two are human source spreads. Counts by stat, including Attack, are:

| Stat | Set occurrences below the same-DV EV-255 control |
| --- | ---: |
| HP | 8 |
| Attack | 64 |
| Defense | 4 |
| Special Attack | 7 |
| Special Defense | 7 |
| Speed | 8 |

Attack counts include deliberate confusion-damage minimization and are not a
bug count. Likewise, a published non-maximal spread is not inherently an error.
The support defect is that **16 of those 17 entries have no preview-consistent
candidate reaching their EV-255 control on at least one deficient non-Attack
stat**. Even an opponent with identical moves/items/DVs and maximum training
cannot be represented. These are record counts, not distinct public-roster
counts; some entries share a preview.

Examples from native stat construction:

| Prior entry / Pokémon | Prior stat | Same-DV maximum-training control |
| --- | ---: | ---: |
| `lineage-fine-s07` Zapdos 55, HP | 206 | 215 |
| `lineage-fine-s07` Cloyster 50, Speed | 114 | 121 |
| `lineage-gb-uni-s13` Gengar 50, Defense | 80 | 111 |
| `ev-r1-pool-hc75-top8-tezan-{0,1}` Snorlax 50, HP | 243 | 266 |
| `lineage-fine-s13` Snorlax 55, HP | 266 | 292 |
| `ev-r1-rand-0-{0,1}` Nidoking 55, Speed | 115 | 149 |

The remaining entry, `lineage-fine-s07`, initially has five equally weighted
preview-consistent candidates. Four have Zapdos HP 215 and Cloyster Speed 121;
one has 206 and 114. **One revealed move can eliminate all four maximum-trained
candidates.** The played witness uses maximum-trained `lineage-fine-s07` sets
and the legal Electrode 50 / Cloyster 50 / Zapdos 55 selection (total 155).
After the opposing Electrode executes Rain Dance, only `lineage-fine-s07`
survives: the opponent's still-unseen Zapdos is determinized with HP 206 and
Cloyster with Speed 114, despite their actual 215 and 121. No damage or
HP-percentage reconstruction is involved. Tauros's Thrash, Marowak's Fury Attack,
and Snorlax's Attract also isolate that candidate in synthetic reveal probes.

The two human non-Attack spreads are `mjj-2021-q10con` Ninetales 55 (SpA/SpD
EV 236: stats 143/163 instead of 145/166) and `mjj-2011-garatank` Starmie 51
(HP EV 236: HP 167 instead of 169). These values already occur in the imported
source; the latter explicitly notes HP training level 59. Their strategic
motivation was not established here. Both lack maximum-training support at
their public rosters. Neither is in the current 33-team catalog.

## 2. Why the prior retains these spreads

- Historical `TeamGen` mutation (`d0842be1`) included `EvTweak`, drawing training
  from `{0,64,128,192,255}`, and arbitrary DV tweaks. The current generator
  already replaces those operations with deterministic training optimization;
  this does not rewrite historical frozen records.
- `tools/build-team-inventory.js` canonicalizes inherited v1 machine records.
  `canonicalize_team` preserves legal nonzero submaximal EVs; validation is a
  legality check, not a maximum-training policy.
- `tools/belief-pool-v3.py` copies those source sets, then allocates weights.
  The machine factor 0.25 changes mass but supplies no missing spread.
- `build_refs` constructs exactly those spreads. `consistent` filters on
  moves, items and Hidden Power type evidence. It never adds maximum-trained
  variants. `impute_mon` copies the candidate's stats and maximum HP.
- Fallback activates only when **zero** candidates survive. A wrong surviving
  spread therefore prevents fallback; increasing search iterations cannot
  supply its missing support. Fallback itself can also copy a source spread.
- `meta_eco::normalize_belief_pool` is a separate research path. It adds an
  optimized team only when gender or item presence changes, and can change
  other loadout fields too. It neither guarantees training coverage nor runs
  in the shipped WASM `Belief::new` path.

The existing preview/catalog gates establish recognition, fingerprints and
artifact consistency. They do not establish full spread support. No new
win-rate conclusion follows from these data checks.

## 3. Playable catalog: the main policy is implemented, with exceptions

All **33 teams / 198 Pokémon** have maximum non-Attack EVs. Their non-Attack DVs
are also maximal subject to Hidden Power constraints. All **43 sets eligible
under the existing no-Hidden-Power/no-caller low-Attack policy** have Attack
EV 0 and the minimum DV preserving HP and the required public gender.

The only multiple-variant catalog shell is APEX4. All four public signatures
(species, level, gender, item presence) agree. Its Cloyster variants are:

| Variant | Moves affecting the constraint | Gender | Attack DV / EV | Attack stat |
| --- | --- | --- | --- | ---: |
| `apex-c08-surgical-142` | no Physical move | M | 9 / 0 | 109 |
| `red-v10-s1105-r2-s0142` | no Physical move | M | 9 / 0 | 109 |
| `apex-c04-bug-3atkstar-boomgen` | Explosion | M | 15 / 255 | 146 |
| `red-v10-s1105-r1-s0148` | Explosion | M | 15 / 255 | 146 |

DV 9 is the minimum odd male-compatible DV for Cloyster, preserving maximum HP.
The active Nash artifact carries exactly the same four sets as the catalog.

The user's broader rule, "no Physical moves means minimum Attack", has these
additional exceptions beyond the current optimizer's narrower policy:

1. **11 special-only Hidden Power sets retain Attack EV 255.** The optimizer
   skips every Hidden Power user. Native construction and format validation
   confirm all 11 can use Attack DV 11 / EV 0 while preserving HP, gender,
   non-Attack stats, Hidden Power Ice type and power 70. Thirteen such special
   HP users exist; the other two already have DV 11 / EV 0. This is a conservative
   exclusion in the implementation, not a necessary Hidden Power constraint.
2. **`pb-100` Starmie has Rapid Spin but Attack DV 1 / EV 255.** A genderless
   maximum-trained physical attacker could have DV 15. The low DV already
   exists in the PartyBox source; the catalog optimizer leaves Physical users
   unchanged. This is an actual exception to the stated physical-maximum rule;
   its damage/confusion tradeoff has not been evaluated here.
3. `sample-11` Blissey also has DV 1 / EV 255 and a move classified Physical,
   Counter. Counter does not use Attack for damage. `pb-62` female Snorlax's
   DV 1 is already maximal for its fixed female gender and maximum HP: it is
   not an omitted male-maximum normalization.

## 4. The prior also misses catalog Attack minimization

All 33 catalog rosters avoid fallback, but **28 set occurrences have an actual
Attack stat absent from every preview-consistent prior candidate**. All missing
stat values in this catalog check are Attack; no other stat value is missing.

For APEX4, prior Cloyster Attack is 146 in every candidate, while the two
non-physical catalog variants actually have 109. Prior Starmie Attack is 126
in every candidate, while all four playable variants have 81. Thus preserving
gender alone makes preview recognition pass without making spread support
correct. This can affect modeled confusion damage; its action or win-rate
effect was not measured.

## Repair boundary

A repair needs an explicit prior training policy and coverage checks for both
maximum non-Attack stats and constrained minimum Attack, preserving legal
gender/Hidden Power/camouflage relationships. Dropping old machine entries alone
would leave the two human maximum-training holes and catalog Attack mismatch.
Applying the whole research dominance optimizer would also change species,
moves or items, beyond a training repair. Production edits and preview rebaking
are outside this investigation.
