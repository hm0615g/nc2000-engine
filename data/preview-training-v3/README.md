# Compiled preview after training v3

Recomputed on 2026-11-03 for the corrected catalog and opponent prior. This
replaces the tables retired by the original training repair; previous
computation hashes were not relabeled.

The 33 catalog parties have 30 distinct public rosters, producing **990 keys**.
For each public roster the baker uses the actual runtime `Belief` candidates
and weights: 49 prior entries, or 47 distinct full-set types, across the
covered rosters. It cross-checks membership with `Belief::alive()`.
Own keys retain the real trained stats, happiness and ordered moves. Opponent
keys contain public roster facts and never the opponent's hidden loadout.

The old generator overwrote DVs/EVs and omitted non-catalog prior variants.
It also assembled bench Pokémon from templates where entry effects had already
fired. The replacement initializes each selected lineup through the native
engine before converting it to compact rollout state. A regression test checks
that bench Berserk Gene Tauros keeps its item and has no entry boost/confusion,
while lead Tauros correctly activates it.

## Boundary repair update

The [boundary repair](../blind-boundary-repair-v1/README.md) selectively recomputed
182 Nash-involving matrices and reused 946 other matrices after checking their
source/data dependencies and candidate identities. This rebuilt 149 affected
policy keys; all 841 unrelated keys were preserved. Recomputed choices happened
to be identical: this rollout bake does not call the repaired live search.
`partialRebake` records the previous artifact hash and reuse proof. The new
artifact still covers all 990 keys and carries the current engine identity.

## Computation and limits

The bake evaluates 1,128 unordered type-pair matrices. Actions cover each legal
lead and unordered pair of bench Pokémon. Covered sets cannot call moves that
depend on bench ordering; the baker rejects Beat Up and move-copy/call sets.
Each matchup uses two rollout seeds, or three when either public roster is
APEX, in both side orientations, with a 250-turn limit. The prior-conditioned
Bayesian solver runs 600 sweeps with existing finite-sample shrinkage and
subteam pooling. Ordinary parties retain the previous diversity mixture;
APEX uses the solved policy. Policies are quantized to 64 deterministic slots.

These are finite-rollout approximate policies with selective reuse as recorded above. They are not
a full-game equilibrium certificate or a new measurement of party-draw Nash
weights. The historical draw weights and measurement labels remain unchanged.

`basis.json` records all source/data hashes and computation settings.
`bake-manifest.json` identifies the complete artifact by SHA-256. Runtime lookup
requires the correct engine identity, prior, own full set, public opponent
roster and search coefficient. Build validation rejects stale source inputs.
Regenerating training products preserves these fresh tables byte for byte.

## Verification

The original full-bake `production-verification.json` exercised the bundled
production worker. Updated worker identity and timings after the boundary
repair are in [the repair verification](../blind-boundary-repair-v1/production-verification.json).
Both checks cover:
all **1,089 ordered catalog pairs, both sides (2,178 cases)** return `src=table`
and zero search iterations, with no search progress messages. Local lookup
latency is median **1.1 ms**, p95 **1.5 ms**; battle initialization plus lookup
and worker messaging is p95 **25.2 ms** on the verification host.
The test gates lookup p95 below 20 ms and initialization plus lookup p95 below
100 ms, including in the Pages build before deployment.

A hidden item/move substitution preserves the choice. Changes to own happiness,
prior bytes, public gender or public item presence correctly reject the cache
and perform the requested one search iteration. Direct WASM coverage also
checks every one of the 64 quantized slots for every catalog pair against the
engine's legal choices.

## Reproduction

Run from the repository root, then `web` as indicated:

```sh
cargo test -p nc2000-bot --example meta_eco catalog_preview::tests
cargo run --release -p nc2000-bot --example meta_eco -- bake-catalog-preview --threads 12 --out-dir tmp/prior-preview-v1
python3 tools/activate-preview-bake.py tmp/prior-preview-v1
node tools/build-training-products.js
cd web
NC2000_BASE=/nc2000-engine/ npm run build
NC2000_BASE=/nc2000-engine/ npm run test:preview-production
NC2000_CHECK_OUT=../data/preview-training-v3 node tests/compiled-preview-coverage.mjs
```

Build current `pkg-web` and `pkg-node` WASM first with `crates/wasm/build.sh` and `crates/wasm/build.sh nodejs` when engine/bot
sources change. Matrix cache entries are scoped by the full computation basis;
an interrupted bake resumes matching work. A partial `--pair-limit` run never
activates incomplete policies. For deployment verification, set
`NC2000_PREVIEW_CHECK_URL=https://puniu3.github.io/nc2000-engine/` and
`NC2000_PREVIEW_CHECK_OUT` to a scratch report path when running the production
test. It discovers the deployed worker asset and verifies served catalog/prior
hashes, so it also checks the actual CI-built WASM rather than a local copy.
