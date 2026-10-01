# team-selection-v2 — which parties the bot plays (blind, c = 0.4)

The selection behind the catalog `data/team-pool-v2`: the built-in parties a
human can pick and the parties the bot draws. Plan:
`docs/TEAM-POOL-REBUILD-PLAN.md`. Rules: `PROTOCOL.md` (frozen before any
selection score; amendment v2.1 for the pondering check).

## Files

| File | What |
|---|---|
| `PROTOCOL.md` | Conditions, panel, stages, thresholds, seed bases |
| `panel.json` | The fixed scoring panel: 78 human-source teams in 58 variant clusters, weights, strata; candidates and challenge panel |
| `selection.json` | Base label for each of the 103 candidates — `selected` / `pending` / `not-selected` — with the stage and reason, the prior fingerprint, V and deletion effects |
| `reference.json` | Generated (`tools/build-team-products-v2.js`): every inventory record with v1 label, selection label, catalog / prior / Nash membership and the three probability fields kept apart |
| `measurements/` | Stage A, B, C reports, execution audit, pondering sensitivity and its extension |

Raw games stay in `tmp/team-eval-v2/` (gitignored); every game is
reproducible from its record's `cond` (bot fingerprint, prior fingerprint,
turn cap, seed base) and `crates/bot/examples/team_eval.rs`.

## Outcome

- Score: weighted mean on the fixed panel at blind:27000:0.4 on both sides,
  prior `data/belief-pool-v3` (`fnv1a64:ffa73c4245fe3774`), no pondering.
- **24 selected**: confirmation (fresh seeds) lower bound above 0.50; no member
  clearly harmful under the simultaneous deletion test. **11 pending**:
  undecided after two looks. **68 not selected**: one failed confirmation,
  7 were beyond the 36-entrant cap, 60 scored below 0.52 at stage B
  (every candidate reached stage B because stage A's calibration was WEAK).
- Pool value on the same panel: 0.601 ± 0.006 for the 24, against 0.498 for
  the previous local 79-team pool and 0.515 for the old bundled 32.
- `not-selected` and `pending` are about this bot's ordinary pool, not
  `dominated`: no dominated verdict is issued here, and every candidate stays
  in the prior and the reference inventory. Several pending trap / Baton Pass
  teams are bot-limited (their key member is brought in 0–7% of games).
- Pondering sensitivity (candidate at 2× the budget): rank agreement 0.62 with
  the product-budget scores; three flags at the boundary did not stand on 64
  fresh blocks each.
