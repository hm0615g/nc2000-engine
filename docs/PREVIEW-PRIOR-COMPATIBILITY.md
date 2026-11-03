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

## 2026-11-03 migration and expansion finding

This change only installs dependency metadata and the new validation rule.
It does not expand the shipped prior, remeasure payoffs, or change any choices.
All 990 current keys remain compatible.

The requested future membership rule is: include real-world parties and their
variants; include evolution-generated parties only when in the current party
catalog. The expansion is **not** guaranteed to preserve all existing keys.
A concrete counterexample is `mjj-gym-Tokiwa` in
`data/kodoku-r3/majinjima-assassins.json`, after the existing training-v3
normalization. It shares `sample-09`'s public shell but differs from the current
`pb-18` and `sample-09-lowatk` variants. For example, its Smeargle holds Bright
Powder and its Nidoking has Dynamic Punch. Adding it with any positive weight
introduces a new type. This addition alone affects 62 keys (30 own-side keys
plus 33 opponent-side keys minus their overlap), preserving 928.

This is a demonstrated lower bound for the full expansion, not a complete
membership/weight audit. Some source records need validation/normalization
repairs before they can be admitted. The new regression test reproduces the
Tokiwa case without modifying the shipped prior.

Validation: semantic regression tests; production Web build; complete worker
lookup on both sides of all catalog pairs, including changed own/opponent
shell rejection, unrelated-shell reuse, and formatting-only prior changes.
