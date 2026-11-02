# ICO investigation: findings and limits

The session's implementation, experiment artifacts tracked in Git, and proposed
HP fix were reverted at the user's request. This note is the retained result.
Historical evidence remains accessible in reverted commit `2d2c3d05`.

- **Definition:** ICO (Information-Constrained Oracle) uses only rules-permitted
  observations and deductions, with unlimited computation. It is not omniscient.
- **No win-rate gap was established.** A finite two-decision surrogate covered
  30 public rosters and 976 hidden worlds. Its median initial-action gap against
  81k was 3.827–3.841 points on a 100-point heuristic scale. Chance sampling,
  restricted selections and unresolved continuation prevent conversion to win
  percentage points or a certified full-game ICO gap. Terminal match evaluation
  was stopped before completion; partial pilot results support no conclusion.
- **Reproduced observation:** the shipped WASM search, with constructor seed
  17011, chose different actions in two pairs with identical recorded own
  information and player-visible histories: `switch 2` / `move icepunch`, and
  `move rockslide` / `switch 2`. This establishes dependence in those fixtures;
  its effect on the randomized policy distribution or win rate was not measured.
- **HP hypothesis:** same-candidate, same-RNG determinization of those fixtures
  differed in opponent HP: 215 versus 206, with sampled maximum HP 215. The
  source retains/clamps the original integer HP when replacing maximum HP.
  A proposed percentage-based reconstruction made the native 81k actions,
  visits and means coincide. However, the separate HP-only causal intervention
  was not completed, and the replacement belief model was not validated.
- **Why non-maximal HP appeared:** the existing opponent prior includes the
  machine-generated `lineage-fine-s07`, whose level-55 Zapdos has HP EV 192;
  `mjj-2013-mario` has HP EV 255. Both have maximal HP IV, yielding maximum HP
  206 and 215 respectively. This is not a nine-HP percentage-rounding error.
  The exporter included matching public-roster prior entries without first
  validating their relevance to ordinary maximum-trained opposition. No
  strategic justification for HP EV 192 was established in this session.
- **Scope remains open:** whole-team legality, the authoritative observation
  contract, maximum-training controls, other hidden-state paths, and preview
  artifact compatibility were not fully verified. Retained hidden clocks were
  observed in source, but no additional behavioral counterexample was completed.
- **Methodological correction:** population validation and controlled causal
  tests must precede production edits. Changing production behavior and existing
  test expectations to fit a hypothesis does not independently validate it.
  Boundary stress tests and representative win-rate tests answer different
  questions. The earlier definitive attribution to HP was withdrawn.

No production WASM replacement or deployment occurred. The task's Colab session
was terminated; no GCP Batch jobs were submitted. Diagnostic scratch files under
`tmp/ico-v1`, `tmp/ico-v2` and `tmp/ico-leak` are not validated product artifacts.
