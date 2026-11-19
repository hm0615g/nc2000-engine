# Search for a disclosed NC2000 party pool

Each player knows the opposing six-party distribution, full sets and weights, observes its usual private player channel, and chooses its own lead and three. Production WASM uses 729k live preview and the adaptive 243k battle cap. Seat partners use the same realized rosters, team search seeds and battle seed. The primary score counts a tie or 500-turn cap as half. Research output is kept separately from the deployed team products.

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

The response, refinement and shell-fit stages all use `response-pools.json`. The deviation screen uses `final-pools.json`. Independent confirmation uses `confirmation-pools.json`. Any redistribution and its final check use `repair-pools.json`. The `mixtures` and `ids` arrays follow seat order. The first logical player sits in the seat named by `rowSeat`. A full-screen replay includes all matched singleton controls.

The eight [Colab replays](colab-parity.json) matched local CPU execution on both selections, every action hash, turn count and outcome. They are duplicate checks and contribute no additional statistical observations.
