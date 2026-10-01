# belief-pool-v2 — the opponent prior of the c = 1.0 rebuild (superseded)

**Historical, never deployed.** Superseded on 2026-10-01 by
`data/belief-pool-v3` (this membership plus カビバン 2020, validated under
the shipped blind profile). No consumer reads this file; the "Consumers"
section below describes the code as it was.

`belief-pool.json` was generated (`node tools/build-team-products.js`) from
`data/team-inventory-v1`. It is what every blind searcher assumes about the
team it faces. It is never a list of teams the bot plays.

## What the belief actually consumes

Audited in `crates/bot/src/belief.rs` before choosing the representation:

1. **Identification** — a candidate survives when its six (species, level,
   item presence) match the preview and its sets contain every revealed
   move / known item. Survivors are sampled **in proportion to `weight`**
   (this file is a *weighted prior*: any team carrying `weight` switches
   the belief to the weighted rules; unweighted pools keep the old uniform
   draw, bit-identically).
2. **Fallback** (no survivor — a custom team) — per species, the
   **weighted-mode loadout** (item + move set) among the prior's teams
   carrying that species, ties to the earliest in file order; then the
   embedded community rentals; then the learnset default.
3. The M18 per-species move prior (`data/belief-prior-v0.sample.json`) is a
   separate opt-in table; this file does not feed it.

## Support

Every eligible measured team whose base label is not `dominated`:
strong, weak and pending (`label` on each entry). Loadout duplicates are
represented once (the member whose training the source publishes). Order:
strong, pending, weak; by panel score within a label.

## Allocation

`weight = 1 / (members of its variant cluster in this prior)`, times
`machineFactor` (0.25) for META-NASH-generated teams — so near copies of
one design share one unit instead of multiplying it, and a human opponent
is assumed far less likely to bring a machine-made team. Weights matter
only where previews collide and in the fallback's weighted mode.

Calibration against the intended human environment (the local spectator
corpus, 570 human games, aggregate numbers only) and the battle checks are
in `data/team-inventory-v1/measurements/` (`fallback-rule.json`,
`prior-heldout.json`) and summarized in `docs/TEAM-POOL-REBUILD-PLAN.md`.
The battle checks ran blind:1000 at c = 1.0 without pondering, not the
shipped blind profile (27,000 iterations, c = 0.4, pondering).

## Consumers

web `?blind`, `?nash` and `?solver` (`web/src/data.ts` `fetchBeliefPool`,
handed to the worker as the searcher's pool JSON), and the ladder client
`tools/ps-client.js` in blind mode. A pool file a user loads under `?blind`
does not replace it.
