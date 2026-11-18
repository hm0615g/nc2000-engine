# Validation with mutually disclosed team pools

The candidate scored **15–15 against Deme** and **14–6 against the deployed mixture**. Weighting the five candidate types by their actual sampling probabilities gives 48.3% and 71.5%, respectively. The conservative seat-pair 95% bounds are 13.1–83.6% against Deme and 28.3–100% against the incumbent. These samples support retaining the candidate for evaluation; they cannot establish equilibrium or a winning margin against either opponent. [Scores and individual seed pairs](summary.json) · [Game records](games.jsonl)

Deme picked Tyranitar / Snorlax / Suicune against the Sample07 shell in all 24 games and Tyranitar / Skarmory / Snorlax against the red shell in all six. Tyranitar led both selections. The candidate's Sample07 types went 14–10 in total against Deme and its red type went 1–5. Individual type scores are especially sensitive to opponent choice. For example, `sample-07-beq-g1-c4` went 6–0 against Deme and 0–4 against the incumbent draws.

The secondary comparison drew three `apex-c04-bug-3atkstar-boomgen`, four `red-v10-s1105-r2-s0142`, and three `apex-c08-surgical-142` in its ten incumbent seed blocks. No `red-v10-s1105-r1-s0148` happened to be drawn, although both search and the draw included it at 15%. All 50 local games reached a battle outcome. The test makes no changes to the deployed mixture.

## Information available to each player

Both players know every eligible opposing six, its full sets and its exact draw probability. Each player privately receives its realized team. The normal Team Preview reveals species and other public roster information. Each bot independently chooses three Pokémon and its lead with 729,000 live preview iterations. Its opponent receives only its own player channel and learns the picks through play. Battle decisions use the adaptive 243,000-iteration cap.

The candidate uses the five exact weights and sets in [candidate.json](../candidate.json). Deme uses the single catalog team `deme-r1-00-s0030`. The incumbent uses the four deployed types at 40%, 30%, 15% and 15%. Each searcher takes its opponent's complete mixture before preview. The four Sample07 candidate types have the same public roster and equal conditional probability. The candidate's red type and Deme are each identifiable from the public roster and the disclosed pool.

## Sampling and verification

- Each block uses a fresh battle seed and one private search seed per team. The same rosters and seeds play twice, with the seats swapped. Against Deme each of the five candidate types gets three blocks. Against the incumbent each gets two blocks, with one independently drawn incumbent per block. The estimates weight the five candidate strata by their published draw probabilities. Wins and losses are raw game counts.
- The harness checks on every decision that the played opposing type remains among the consistent hypotheses. It records the candidates visible to each bot at preview. The verifier compares those IDs to the matching public rosters in the disclosed mixture and checks each seat swap and seed. A separate frame check gave byte-identical Deme preview frames on both seats across all four Sample07 types for a common seed.
- The uncertainty bounds apply Hoeffding's inequality to the independent seed-pair scores, weighting a pair in stratum i by its candidate weight divided by its number of pairs. These conservative 95% intervals are pointwise for each opponent. All sampled games use new seeds. The earlier fixed-prior games are a separate experiment.

## Replay

With the Node WASM target and Web dependencies installed, run from the repository root.

```sh
node web/tests/nash-refresh.mjs data/nash-refresh-v1/disclosed/games.jsonl target/disclosed-replay 10
python3 data/nash-refresh-v1/disclosed/summarize.py
```
