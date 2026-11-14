# Preview prior compatibility

The compiled Bayesian preview policy depends on the conditional distribution
of full sets within **both** players' public shells (species, levels, gender,
and item presence). A new unrelated shell does not invalidate existing keys.
The opponent's response depends on the own-shell distribution too.

`web/src/preview-prior.mjs` is shared by the build pipeline and browser worker.
It groups equal effective sets, sums their weights, and reduces exact decimal
weight ratios within each shell before hashing. Team IDs, descriptions, JSON
formatting, team/roster order, equivalent even IVs, and proportional rescaling
do not invalidate policies. Move slot order remains part of the set identity,
as in the native bake. Nicknames, abilities, and natures are not engine inputs
to this Gen 2 preview game. Dex and source changes retain their existing strict
checks; semantic prior compatibility does not authorize source migration.

The artifacts retain their original `priorSha256` and `inputs` provenance.
`priorDependencies` records the bake's shell fingerprints. Metadata can only
be attached by `tools/preview-prior-metadata.mjs` while all original bake inputs
still match. Fresh activation installs this metadata automatically. The raw
bake manifest remains a record of the original, pre-metadata bake files.

At runtime each key requires matching fingerprints for both shells. Affected
keys use live preview search until recomputed; unaffected keys keep their
original quantized choices. The build reports the number of affected keys,
rather than retiring the whole artifact. A legacy artifact without semantic
evidence retains the strict whole-file prior check. Changed catalog, dex,
coefficient, and source dependencies are not exempted by this change.

## Full bakes and partial invalidation behavior

When `data/belief-pool-v3/belief-pool.json` changes only unrelated shells or JSON
formatting, unaffected keys keep their precomputed preview policies. If a public
shell's conditional full-set distribution changes before a full re-bake,
`web/vite.config.ts` filters out the incompatible policies from the worker
bundle while preserving the historical artifact on disk, and runtime lookups
reject incompatible custom priors and fall back to live search using
`data/live-search.json`.

Rebuilding or expanding the opponent prior does not alter own-team catalog sets
or Nash draw weights. For the current compiled preview artifact status and
re-bake workflow, see [`data/preview-training-v3/README.md`](../data/preview-training-v3/README.md)
and [`data/belief-pool-v3/README.md`](../data/belief-pool-v3/README.md).
