# team-pool-v2 — the catalog

`team-pool.json` is generated (`node tools/build-team-products-v2.js`) from
`data/team-selection-v2/selection.json`: the 24 parties labelled `selected`
under `data/team-selection-v2/PROTOCOL.md`, ranked by their confirmation
score at blind:27000:0.4.

## Who reads it

| Consumer | Use |
|---|---|
| web play door `/` (and `?blind`) | the built-in parties a human picks from, the human's Random, and the bot's party every battle and rematch |
| web `?solver` | the team pickers |
| ladder client `tools/ps-client.js` | `--team pool:random` / `pool:ID` |

Same ids and exact sets on every path. It is **not** the bot's model of its
opponent (`data/belief-pool-v3`) and not the `?nash` draw
(`data/meta-nash-v3`). No baked pair table is keyed to it.

## Draw rule

`drawWeight`: every variant cluster among the selected parties is drawn with
equal probability, split equally among its selected members (24 parties in 16
clusters) — near copies of one design do not multiply its share. Both Random
buttons and the ladder's `pool:random` use these weights; the selection's
pool value `V` was measured under the same rule.

`score` on each entry is the confirmation panel score (fresh seeds, stage C).
