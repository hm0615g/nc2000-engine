# team-inventory-v1 — every team the rebuild considered

The single classified inventory behind the shipped team files
(`docs/TEAM-POOL-REBUILD-PLAN.md`). Facts come from sources and are rebuilt
by scripts; judgments (labels) live in `classification.json` and cite
evidence; the shipped files are generated from both.

```sh
node tools/import-majinjima.js [--fetch]   # sources/majinjima.json (cache: tmp/majinjima-cache/, gitignored)
node tools/build-team-inventory.js         # inventory.json
node tools/team-eval-inputs.js             # eval/ (teams, id lists, controls, off-prior opponents)
node tools/build-team-products.js          # team-pool-v1, belief-pool-v2, meta-nash-v2, reference.json
cargo test -p conformance --test team_products
```

## Files

| File | What |
|---|---|
| `sources/majinjima.json` | 魔人島「俺のパーティ軍団」 serious-team intake: each index entry's table rows, its decoded hidden rental-utility field, mapped + validated sets, every deviation/assumption, page fingerprints. No article prose or battle logs. |
| `inventory.json` | 133 records: the bundled pool (32), Bright Powder originals the Smogon thread edited out (8, reconstructed from the thread's notes), the shipped prior's other members (55: META-NASH machine teams and rentals), the rest of the rental DB (9), and 35 魔人島 records (31 entry teams + 4 page variants no serious entry designates). |
| `PROTOCOL.md` | The frozen classification protocol (conditions, panel, thresholds, dominated rules). |
| `eval/` | Harness inputs: `teams.json` (candidates, held-out page teams, calibration controls, off-prior chimeras), id lists. |
| `measurements/` | Aggregated evidence (per-team scores, calibration, confirmation, execution audit). Raw per-game JSONL stays in `tmp/team-eval/` (gitignored). |
| `classification.json` | Base labels with evidence, replacement links for dominated verdicts, and the prior allocation parameters. |
| `reference.json` | Generated: every record with eligibility, label, replacement, and its memberships in the three shipped files. |

## Inventory fields

- `id` — stable. 魔人島 ids are `mjj-<year>-<page>[-<anchor>|-a|-b]`;
  unlisted page tables are `mjj-<year>-<page>-t<N>`.
- `family` / `origin` — source family (`majinjima`, `hc75-top8`,
  `smogon-hub-samples`, `community-rentals`, `meta-nash-*`) and whether a
  human or the META-NASH search made the team.
- `source` — URL, exact version (page heading / thread sample / rental
  archetype), retrieval date and content fingerprint where fetched here.
- `eligibility` — `eligible` or `ineligible` with reasons. Separate from
  quality: format-illegal (OHKO moves, Phanpy's Japan-only event Encore,
  Item Clause, Little Cup levels), built for another rule
  (OHKO-allowed / Little Cup / Metronome-only), incomplete, or outside the
  intake subset (reference-only page variants).
- `deviations` — every departure from the published source: unpublished
  training assumed maximal, happiness assumed for Return/Frustration,
  spelling normalizations, PP Ups the engine cannot express, explicit
  table/encoded-field resolutions (`tools/import-majinjima.js`
  `RESOLUTIONS`).
- `setSignature` / `exactDuplicates` — byte-level set identity (none among
  eligible records).
- `loadoutDuplicates` / `measuredAs` — same six item+move loadouts, only
  DVs/training/gender differ (12 records, mostly rental transcriptions whose
  training the DB never published). One member is measured; the others
  inherit its label.
- `nearVariants` / `variantCluster` — ≥5 shared species and ≥3 identical
  loadouts; clusters are connected components over eligible records and
  are the unit that keeps near copies from buying weight by count.
- `previewSignature` / `previewCollisions` — what the blind belief matches
  on (species, level, item presence); collisions are where the prior's
  weights matter.
- `lineage` — verified source-to-source derivations the near-variant rule
  misses (sample-16 ← Party Box #20 「ポニョ」; sample-07 ← 魔人島 Mario
  Party 13).
