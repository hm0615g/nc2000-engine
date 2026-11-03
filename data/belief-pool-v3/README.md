# belief-pool-v3 — the known-party opponent prior

The deployed prior now has 411 entries: known real-world parties including
variants, plus generated sets only when present in the current playable
catalog. The catalog and Nash draw mixtures are unchanged. `bake-prior.json`
preserves the previous 221-entry input for preview provenance, historical
regressions, and the catalog's fixed training choices.

`python3 tools/belief-pool-v3.py build` invokes
`tools/build-known-prior.mjs`. It retains admitted baseline weights, adds
new real-world types with weight 1, and collapses duplicate source aliases
without adding mass. It removes 25 off-catalog generated entries and adds
215 distinct types. Sources, exclusions for invalid-format parties, and
hashes are recorded in `known-expansion.json`; excluded records are not
silently converted into different legal parties. The native verifier checks
all admitted sets and exact support for every catalog party.

810 existing preview keys remain usable; the 180 keys depending on changed
`sample-07`, `sample-12`, or `sample-09` shells use live preview search.

## Historical 221-entry prior

`bake-prior.json` records what every searcher previously assumed about the party it faces:
the web play door, `?nash`, `?solver`, `?evaluate`, kifu continuations and
the ladder client. It is never a list of parties the bot plays.

Built by `python3 tools/belief-pool-v3.py build` from explicit inputs only:
the membership and order of the frozen `data/belief-pool-v2` file plus the
records in `RESTORED`, with sets from `data/team-inventory-v1`. Selection
labels (`data/team-selection-v2`) are never read, so relabelling the own pool
cannot move this file. Fingerprint (the one `team_eval` stamps into
`cond.belief_*`): `fnv1a64:7d23346d1218e317` after the owner-requested
[training-v3 repair](../training-v3/README.md). Historical measurements retain
the previous `fnv1a64:957b746a94b97708` fingerprint.

## Support

Every eligible measured team across `data/team-inventory-v1` (103 records),
`data/team-selection-v2/evolved-teams.json` (8 records), and
`data/partybox-v0/teams.json` (101 unique measured records), plus 9
`<id>-lowatk` Female-optimized twins for catalog teams whose `low-attack-v2`
optimization changes a Pokémon's gender (`221` records total). Sets now pass
the shared training-v3 normalization. Independent native verification confirms
exact full-set/stat support for all 33 current catalog parties. The
[pre-repair audit](../prior-training-audit-v1/README.md) records the historical
maximum-training and low-Attack support defects that motivated the repair.

## Allocation

Unchanged rule from v2: `weight = 1 / (members of its variant cluster in
this prior)`, ×0.25 for machine-generated teams. Weights matter only where
previews collide and in the fallback's weighted mode (`crates/bot/src/
belief.rs`). The machine factor is an a-priori choice, not a fitted human
usage model: only 23 human preview-collision cases exist in the local
spectator corpus (`data/team-inventory-v1/allocation.json`).

## Evidence (plan step 2, blind c = 0.4)

Screen at blind:3000:0.4: `measurements/prior-c04-b3000.json`; product-budget confirmation: `measurements/prior-c04-b27000.json`. Summary in `docs/TEAM-POOL-REBUILD-PLAN.md` §Progress.
These are historical measurements of the previous spreads, not remeasurements
of training-v3.
