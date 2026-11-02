# Training v3: normalized parties and opponent priors

Implemented locally on 2026-11-03 after the owner's request to normalize the
playable pool, align the prior and adversarially check every distributed spread.
No publication or deployment is part of this change.

## Policy

`tools/normalize-training.js` is the common generation policy. It preserves
species, moves, items, level, required public gender, shiny status and Hidden
Power type. Within those constraints it maximizes HP and non-Attack stats,
then Hidden Power power. It maximizes Attack when a move uses it and otherwise
minimizes Attack. All non-Attack EV fields are 255; Attack EV is 255 or 0.

Counter, Seismic Toss, Super Fang and Bide do not require Attack. Transform
copies the target's Attack; the user's original Attack can be minimized.
Metronome, Mimic and Mirror Move retain maximum Attack because they can access
physical moves. Return uses happiness 255 and Frustration uses 0. Conflicting
Return/Frustration requirements are rejected rather than silently choosing one.

The catalog retains the existing gender choice and camouflage grouping.
APEX4's special Cloyster stays male at Attack DV 9 / EV 0; Explosion Cloyster
stays male at DV 15 / EV 255. All four public signatures are identical.
Special Hidden Power Ice users now use DV 11 / EV 0 at power 70. `pb-100`
Rapid Spin Starmie now has maximum Attack. `pb-13` Porygon2's Frustration
happiness is corrected from 255 to 0.

Prior membership, ordering, weights and source archives are preserved.
The 221-entry prior now has exact full-set/stat support for all 33 catalog
parties, including the same-gender low-Attack variants. The earlier Rain Dance
witness now determinizes Zapdos HP 215 and Cloyster Speed 121 correctly.

Normalized fallback sources are generated into `supplemental.json` and
`rentals.json`; `Belief` embeds these derivatives. The 40 supplemental teams
and the same 19 legal rental teams use this policy. The nine foreign-format or
illegal rental entries remain excluded by the existing runtime rules.

## Independent adversarial verification

`verification.json` records hashes of all seven checked product files:
33 catalog teams, 221 prior entries, the 4/17/5 current and historical draw
mixtures, 40 supplemental teams and 19 eligible rentals: **2,034 set occurrences**.

The Rust checker does not call the JavaScript normalizer. It constructs real
engine stats, validates team legality, and enumerates all **65,536 DV tuples**
for each of **446 distinct constraints/stat classes**. An adversary can change
all DVs and training simultaneously while preserving gender, shiny status,
Hidden Power type and Unown form. It looks for improved protected stats or
Hidden Power power without reducing another protected stat, and improved
Attack in its required direction. It also checks exact EV policy and
Return/Frustration happiness. **No improving spread or policy violation remains.**

This is optimality under the owner's maximum-stat/minimum-unused-Attack policy
and the stated constraints. It is not a proof that maximal HP/Speed or a gender
choice maximizes game win rate against every possible opponent.

The same checker rejects 465 pre-repair set occurrences from commit `14cf3590`;
`negative-control.json` retains those counterexamples. Catalog support is checked
through actual `Belief::determinize`, not just matching team IDs or public
rosters. A legal played Rain Dance turn also verifies the original failure path.

## Reproduction and regression checks

```sh
node tools/build-training-products.js
cargo run --quiet -p nc2000-bot --example verify_training -- data/training-v3/verification.json
node --test tools/normalize-training.test.js tools/optimize-low-attack.test.js
cargo test -p nc2000-bot --test blind
```

Rebuilding must leave product bytes unchanged. The JS tests check the native
certificate's hashes against current inputs, so stale verification fails.
The Pages build runs regeneration, the exhaustive native check, certificate
comparison and the 21 JS regression tests. The nine blind integration tests
also pass locally.
The negative control uses the old prior and catalog extracted with `git show
14cf3590:<path>` into `tmp/training-v3-controls/{prior,catalog}.json`, then:

```sh
cargo run --quiet -p nc2000-bot --example verify_training -- data/training-v3/negative-control.json tmp/training-v3-controls/prior.json tmp/training-v3-controls/catalog.json
```

That command is expected to exit 1. Rejected entries are evidence that the
adversary detects the known old defects; their expectations were not weakened.

`cargo test -p nc2000-bot --lib belief::` has eight pre-existing move/fallback
test failures. The same eight failures reproduce in an isolated checkout of
`14cf3590` before this repair; the remaining 22 pass. They are not stat-check
failures, and this change does not rewrite those expectations.

## Preview and measurement compatibility

Old preview computations are explicitly marked `retired-training-v3`; their
computation hashes, policies and payoffs remain historical. Runtime lookup
refuses them and uses live selection search. Production builds verify the
retirement receipt against the new prior/catalog hashes. Full grid verification
covers all 1,089 catalog pairs / 990 unique keys and checks live worker use.
Freshly baked artifacts can restore caching after recomputation.

Historical selection measurements and Nash weights retain their original
fingerprints and are labelled not remeasured. Their numbers are not certified
for the new training/prior. Source hashes are not relabelled to make old
computations appear current. WASM and the normal production web build are
rebuilt and checked locally.
