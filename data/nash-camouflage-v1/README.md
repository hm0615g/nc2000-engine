# Search for a disclosed NC2000 party pool

Each player knows the opposing six-party distribution, full sets and weights, observes its usual private player channel, and chooses its own lead and three. Production WASM uses 729k live preview and the adaptive 243k battle cap. Seat partners use the same realized rosters, team search seeds and battle seed. The primary score counts a tie or 500-turn cap as half. Research output is kept separately from the deployed team products.

## Result

The [full candidate](conclusion-candidate.json) has seven types across three public rosters. Sample07 receives **45.46%** (70% `sample-07-beq-g2-c4`, 30% original), APEX4 receives **27.27%** (40/30/15/15), and the standalone Blissey–Miltank roster `red-v10-s1105-r2-s0155` receives **27.27%**. The fit gave Deme zero weight. In the measured shell game, Sample07 scored 68.75% against APEX4, APEX4 scored 81.25% against the Blissey roster, and the Blissey roster scored 68.75% against Sample07. These are 16-game estimates for each matchup. [Shell fit](fit-summary.json)

| Disclosed deviation | Initial screen | Fresh wins against the candidate |
| --- | ---: | ---: |
| APEX Snorlax with Self-Destruct | 6/6 | 9/32 |
| `sample-14` | 2/2 | 14/32 |
| `sample-17-orig` | 2/2 | 17/32 |

Of 67 screened deviations, 19 initially scored above half. The leading held-out score was 53.13%, with a simultaneous conservative 95% interval of 14.45–91.80%. This interval covers random seat-pair scores for the three frozen opponents. The other deviations received screening data only. Arbitrary same-roster variants, continuous reweightings and new rosters remain open. We retain the mix as a research candidate. [Independent confirmation](confirmation-summary.json)

One APEX4 type was absent from the 16 held-out draws. A supplementary test fixed the opponent to `sample-17-orig` and ran two fresh seat-pairs for each of the seven candidate types. Both agents continued to see the actual published distributions. The challenger scored **52.61%** at the candidate weights (per-comparison 95% interval 7.96–97.26%). Its record was 4/4 against `s0155` and 1/4 against each Sample07 type. Each conditional score has only two independent blocks. Both tests left a small observed advantage for this challenger. [Full-support repeat](stratified-summary.json)

The seven phases used 948 games, with zero ties and zero turn caps. Eight additional Colab CPU replays duplicated local games. All phases used the same production WASM and data hashes. The final audit checks every row against its schedule, every player's live preview and compatible candidates, the seat swaps, and seed separation between phases. Run `python3 data/nash-camouflage-v1/report.py` from the project root to regenerate the final distribution and totals. [Measurements](results.json) · [Run hashes](manifest.json)

Against Sample07, Deme's five-type mix scored 50.0% with the full distribution disclosed and 37.5% when its realized type was identified. Against Deme, Sample07 scored 50.0% and 55.0% under those two conditions. Against Sample07, APEX4 scored 36.88% and 22.50%. In the final candidate's separate 32-game paired comparison against the APEX Snorlax challenger, it won 23 times under the full distribution and 20 times with the realized type disclosed. Opponent selections changed in 14 games and seven outcomes changed. The estimated gain was 9.38 percentage points, with a conservative 95% interval of −58.53 to +77.28 points. [Initial controls](screen-summary.json) · [Final controls](confirmation-summary.json)

The [s0155 follow-up](s0155/README.md) tested three one-move variants in 158 further games. The leading parent–Counter Blissey mixture lost its screening advantage on new seeds, and both supported-roster guard scores fell. The seven-type research candidate above remains the reference for subsequent searches.

A [Sample17 entry test](sample17/README.md) allocated half the s0155 weight (13.64% overall) to Sample17. The 176-game comparison favored the proposal. The 96-game independent check gave Sample17 and Sample07 higher scores against it. The seven-type pool remains the research reference.

## Initial paired screen

D5 comprises the original Deme and four archived descendants, sampled uniformly. All four descendants failed current legality because Raikou cannot learn Crunch. Each new `-legal` ID replaces that single slot with the parent's Sleep Talk or, if Sleep Talk is already present, Thunder. S4 comprises the four existing Sample07 types with equal weights. APEX4 starts at the deployed proportions. All 13 sets passed production validation, and the full opponent preview frames within each family were byte-identical on a common seed.

D5 × S4 covers all 20 type pairs. Each pair plays in both seats under three disclosures: both pools public, only the realized Deme type public, and only the realized Sample07 type public. The published opponent distribution for the focal bot stays the same. APEX4 × S4 has the same matched singleton control for APEX4; APEX4 × D5 supplies the third two-family payoff. Payoffs use both published type probabilities. The camouflage contrast is the mixed score minus the weighted singleton score on the same seat blocks. Statistical intervals treat the two seats as a single independent unit.

The three-family payoff table proposes initial outer weights. Every proposed replacement, change of conditional weights and assembled cross-family pool must be played using its actual published distribution. The catalog is the source for outside challengers. The experiment uses the TPU-free production simulator throughout.

## Response search and stopping rule

Within each familiar shell, compare the existing disclosure, its best-looking singleton, a 70/30 mixture of the two highest-scoring types, and a 50/50 mixture of the leading type with a new legal move change. Also test equal APEX4 proportions and the previous five-type candidate. The type ranking uses conditional performance against the two other shells weighted by the initial shell fit, with an equal-opponent fallback for a pure own-shell fit. Newly generated variants have their own IDs. The coverage changes target Tyranitar's Thunderbolt into Water types, Surf and Explosion together on Cloyster, and Self-Destruct on Snorlax. The primary screen gives each known-shell proposal four seat-pairs and each other catalog party one.

For each shell, nominate the best alternative and play it and the base profile on six fresh, common blocks against the actual initial mixture. Keep the alternative only if it scores higher. Recheck the three leading outside singletons on those common blocks and admit the leader if it scores above half. Hashes break screening ties. Fit one conditional profile per distinct visible roster and mix their outer weights by regret matching on the seat-symmetrized shell game. Direct tests of the assembled mixture cover each of the 13 original singleton types, removal of each individual support member with renormalization, and adding 20% of each absent new mutant. These inexpensive probes receive one seat-pair each unless an identical distribution already has a scheduled test. The screen also covers within-family weights, cross-roster weights and outside teams.

Freeze the final shortlist before testing it: the leading within-pool proposal, the leading outside party and the best remaining screened proposal each receive 16 fresh independent seat-pairs. They share seeds across challengers. Report the largest observed score, and simultaneous bounds for that tested shortlist. If a challenger exceeds 50% in screening and reaches 65% on the fresh pairs, try one further explicit redistribution towards it under the resources available. The 65% threshold chooses an engineering follow-up; it is not a statistical equilibrium certificate.

## Replaying the experiment

Each `*-games.jsonl` can serve directly as the task file for the production runner. The archived rows include actual sets by ID, seeds, picks, final outcomes and action hashes. Set `NASH_POOLS` to the corresponding pool definition. Use a new output directory for each replay. For example, from the project root with the Node WASM target built:

```sh
NASH_POOLS=data/nash-camouflage-v1/screen-pools.json node web/tests/nash-refresh.mjs data/nash-camouflage-v1/screen-games.jsonl target/camouflage-screen-replay 8
```

The response, refinement and shell-fit stages all use `response-pools.json`. The deviation screen uses `final-pools.json`. Independent confirmation and its full-support repeat use `confirmation-pools.json`. Any redistribution and its final check use `repair-pools.json`. The `mixtures` and `ids` arrays follow seat order. The first logical player sits in the seat named by `rowSeat`. A full-screen replay includes all matched singleton controls.

The eight [Colab replays](colab-parity.json) matched local CPU execution on both selections, every action hash, turn count and outcome. They are duplicate checks and contribute no additional statistical observations.
