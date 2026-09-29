# belief-pool-v3 — the frozen opponent prior

`belief-pool.json` is what every searcher assumes about the party it faces:
the web play door, `?nash`, `?solver`, `?evaluate`, kifu continuations and
the ladder client. It is never a list of parties the bot plays.

Built by `python3 tools/belief-pool-v3.py build` from explicit inputs only:
the membership and order of the frozen `data/belief-pool-v2` file plus the
records in `RESTORED`, with sets from `data/team-inventory-v1`. Selection
labels (`data/team-selection-v2`) are never read, so relabelling the own pool
cannot move this file. Fingerprint (the one `team_eval` stamps into
`cond.belief_*`): `fnv1a64:ffa73c4245fe3774`.

## Support

Every eligible measured team of `data/team-inventory-v1` — 103 records:
v1's strong, weak and pending teams, and カビバン 2020
(`mjj-2020-kb-2020`), whose open-era dominated verdict (+0.063 at open:1000,
+0.034 at blind:1000 with c = 1.0, below the 0.05 bar) was never reconfirmed
under the blind c = 0.4 product. Restoring it is the plan's rule when the
verdict is not reconfirmed. `v1Label` on each entry is that historical
open-era label, not a selection label.

## Allocation

Unchanged rule from v2: `weight = 1 / (members of its variant cluster in
this prior)`, ×0.25 for machine-generated teams. Weights matter only where
previews collide and in the fallback's weighted mode (`crates/bot/src/
belief.rs`). The machine factor is an a-priori choice, not a fitted human
usage model: only 23 human preview-collision cases exist in the local
spectator corpus (`data/team-inventory-v1/allocation.json`).

## Evidence (plan step 2, blind c = 0.4)

Screen at blind:3000:0.4: `measurements/prior-c04-b3000.json`; product-budget confirmation: `measurements/prior-c04-b27000.json`. Summary in `docs/TEAM-POOL-REBUILD-PLAN.md` §Progress.
