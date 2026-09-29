# team-pool-v1 — the bot's ordinary own-team pool

`team-pool.json` is generated (`node tools/build-team-products.js`) from
`data/team-inventory-v1`: every measured team whose base label is
**strong** under `data/team-inventory-v1/PROTOCOL.md`. Nash members are a
subset of it; weak, pending and dominated teams never appear here.

## Who reads it

| Consumer | Use |
|---|---|
| web open door `/` | the bot's team when its pick is "Random" (`web/src/own-pool.ts`, `app.tsx` `drawOpponent`) |
| web `?blind` (no pool file loaded) | the bot's team every battle and every rematch |
| ladder client `tools/ps-client.js` | `--team pool:random` / `pool:ID` |

It is **not** the list a human picks from on the start screen (that stays
the bundled `meta-pool-v0`, unchanged) and **not** the bot's model of its
opponent (`data/belief-pool-v2`). Draws carry no pool index, so no baked
pair table (keyed to the bundled list) is ever read against them.

## Draw rule

`drawWeight`: every variant cluster among the strong teams is drawn with
equal probability, split equally among its strong members — near copies of
one design do not multiply its share. Weights sum to 1; readers
renormalize.

Chosen here because the plan left ordinary-draw weighting open; the
alternative (uniform per team) differs only where two strong teams share a
cluster (listed in `data/team-inventory-v1/reference.json`).
