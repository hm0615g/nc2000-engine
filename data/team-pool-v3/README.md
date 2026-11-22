# `team-pool-v3`

Generated deterministically by [`tools/build-team-products-v3.js`](../../tools/build-team-products-v3.js) from committed repo sources (`data/team-pool-v2/team-pool.json`, `data/belief-pool-v3/belief-pool.json`, `data/kodoku-r3/majinjima-assassins.json`, `data/team-inventory-v1/inventory.json`) plus the strict-LIVE-verified flagship shell `s0053-lefties`.

## Summary

- **Catalog**: [`data/team-pool-v3/team-pool.json`](./team-pool.json) (`format: "nc2000-team-pool-v3"`)
- **Flagship Mixture**: [`data/team-pool-v3/flagship-mixture.json`](./flagship-mixture.json) (`50% s0053-lefties-hpi` + `50% s0053-lefties-dp`)
- **Selected teams**: `33` across **`32` pairwise-distinct public Turn-0 shells** (`1` flagship 2-variant camouflage cluster `vc-s0053-lefties` + `31` distinct single-shell Tier-1 archetypes)
- **Distinct Level-55 Aces**: `23` (`Zapdos`, `Suicune`, `Marowak`, `Snorlax`, `Raikou`, `Porygon2`, `Charizard`, `Tauros`, `Tyranitar`, `Miltank`, `Machamp`, `Gengar`, `Victreebel`, `Nidoking`, `Crobat`, `Venusaur`, `Heracross`, `Alakazam`, `Kingdra`, `Dragonite`, `Entei`, `Poliwrath`, `Rhydon`)
- **Draw Weight Sum**: `1.000000` exact (`1/32 = 0.03125` per shell; flagship `s0053-lefties` split `0.015625 + 0.015625`)
- **Legality & Training**: `33/33` WASM `Validator` PASS, strict Japanese Nintendo Cup 2000 cartridge legality (zero NYPC event moves, zero incompatible Gen 2 egg-move combos, zero duplicate items), normalized via `training-v3` (`changedTeams: 4`, `changedSets: 4`, `changedGenders: 3`)

## Flagship Strongest Core (`vc-s0053-lefties`, Rank 1–2, `nash: true, evolved: true`)

Built from the `s0053` shell (`Electrode50 / Suicune55 / Cloyster50 / Marowak55 / Snorlax50 / Zapdos55`) and promoted via **7,008 strictly uncached NO-CACHE LIVE (`243k/729k`, `preview.source == 'live'`, `cache_hits == 0`) games** across item ablation (`2,520g`), structural variant sweeps (`2,688g`), and independent holdout certification (`1,800g`):

- **Shared 4 slots**:
  - `Electrode L50` @ `Miracle Berry` — `Thunderbolt / Explosion / Light Screen / Thunder Wave`
  - `Suicune L55` @ `Mint Berry` — `Surf / Ice Beam / Rest / Sleep Talk`
  - `Cloyster L50 M` @ `Gold Berry` — `Surf / Ice Beam / Spikes / Explosion`
  - `Snorlax L50 M` @ `Pink Bow` — `Body Slam / Earthquake / Self-Destruct / Curse`
- **2-Axis 50/50 Camouflage** (byte-identical Turn-0 preview signature):
  - **`s0053-lefties-hpi` (50%)**: `Marowak L55 M @ Thick Club [EQ / Rock Slide / Fire Blast / Swords Dance]` + `Zapdos L55 @ Leftovers [Thunderbolt / Hidden Power Ice / Rest / Sleep Talk]`
  - **`s0053-lefties-dp` (50%)**: `Marowak L55 M @ Thick Club [EQ / Rock Slide / Hidden Power Bug / Swords Dance]` + `Zapdos L55 @ Leftovers [Thunderbolt / Drill Peck / Rest / Sleep Talk]`
- **Strict NO-CACHE LIVE Verification Metrics (`1,800g` holdout + `480g` prior LIVE)**:
  - **Broad-8 Top Meta Shells**: **`64.45%`** unweighted defender WR (**`8/8`** certified `UCB95_t <= 55%`: `mjj-2014-thdrcomb 72.9%`, `hc75-3rd-bean 67.7%`, `s13:champ-talk 66.7%`, `deme-r2-05-s0043 65.6%`, `pb-63 63.5%`, `APEX4 62.5%`, `rental-cban-11 60.4%`, `mjj-type-QA-moudoku 56.2%`)
  - **Top-18 Challenger Holdout**: **`54.63%`** unweighted defender WR (**`11/18`** certified `UCB95_t <= 55%`), outperforming `v2Cz:hpi-whpb` (`Bright Powder`) by `+1.92pp` across 14 common LIVE challengers (`53.04%` vs `51.12%`) and `51.0%` head-to-head (`96g`)
  - **Worst-Case Exploiters (No Total Collapse)**: Only 2 of 19 LIVE opponents sit below 40% pooled defender WR — `pb-84` (`35.1% ± 3.7%`, `n=168`) and `mjj-2014-offantom` (`36.3% ± 4.1%`, `n=168`, lifted `+8.0pp` from `Bright Powder`'s `28.3%`); all remaining `17/19` opponents are `>= 42.3%`
  - **Zero Genetic Hitchhikers**: All 6 slots active across 1,800 holdout games (`s1: 19.3%`, `s2: 9.8%`, `s3: 80.7%`, `s4: 19.3%`, `s5: 100.0%`, `s6: 70.9%`)

## Tier-1 Roster Composition (`32` Distinct Shells)

1. **Flagship Strongest Core (`1` shell / `2` variants, `nash: true, evolved: true`)**: `s0053-lefties-hpi`, `s0053-lefties-dp`
2. **Strict-LIVE Verified Top Breakers & Meta Heavyweights (`12` shells, human)**: `pb-84`, `mjj-2014-offantom`, `pb-122`, `rental-cban-8`, `pb-6`, `pb-131`, `pb-87`, `mjj-type-QA-moudoku`, `sample-14`, `pb-63`, `pb-66`, `mjj-2014-thdrcomb`
3. **Evolved Diverse Archetypes (`4` shells, `evolved: true`)**: `apex-c04-bug-3atkstar-boomgen` (Kodoku v14 Shell A representative), `deme-r1-00-s0030`, `red-v5-r1-s0097`, `red-v10-s1105-r2-s0155`
4. **Classic & Diverse Human Tier-1 Archetypes (`15` shells, human)**: `sample-07`, `sample-08`, `sample-12`, `sample-17-orig`, `mjj-2015-ayagen`, `sample-11`, `sample-06`, `mjj-2020-rraikou`, `mjj-2010-garap-a`, `sample-02`, `sample-09`, `sample-22-orig`, `hc75-top8-tsuru`, `rental-cban-13`, `pb-89`
