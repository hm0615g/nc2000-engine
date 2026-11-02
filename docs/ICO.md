# Information-Constrained Oracle (ICO)

ICO means **Information-Constrained Oracle**: an ideal player with unlimited
computation but only rules-permitted information and deductions from that
information. It is not a fully observed battle solver. Its executable finite
approximation is `tpu/src/gen2_tpu/ico.py`; battle games are exported by
`crates/bot/examples/ico_export.rs`.

The measured 81k census, independent-chance checks and production information-
invariance counterexamples are recorded in [`data/ico-v1`](../data/ico-v1/README.md).

## Objective and information contract

Fix our party draw to the shipped APEX4 mixture. The long-term reference is
`max_pi min_tau E[score]`, where score is win=1, draw=0.5, loss=0, `pi` is our
legal selection/battle strategy and `tau` includes the opponent's team,
selection and battle strategy. The opponent may know our distribution and
algorithm, but not the realized private draw. Neither player sees an
unrevealed opposing action or future random result.

At every decision, a strategy is indexed by:

- its owner's known team;
- the complete player-visible protocol and own request history;
- the owner's past attempted actions, including publicly cancelled actions.

The exporter uses `PlayerChannel`, including private/public split filtering.
It never keys policies by a battle state, world ID, search seed, chance sample
index, opposing private set or unannounced opposing action. Both simultaneous
choices use observations captured **before** either choice is applied.
Information sets are structurally interned; identical keys must have identical
legal actions and own preceding sequences. Exported labels are SHA-256 hashes;
duplicate exported labels are rejected rather than merged.

This prevents strategy fusion at future decisions as well as at the root.
Latent states may be used to compute transitions and rewards; they cannot
select different policies within one information set. Opponent-private setup
is one decision shared across all our unobserved APEX4 variants.

## Finite solver

The representation is the sequence-form payoff operator `x^T A y`, with one
empty sequence and one sequence for each information-set/action pair. Realization
plans obey perfect-recall flow constraints. CFR+ updates counterfactual regrets;
the output is the linearly weighted, own-reach-weighted average behavioral
strategy. All legal actions in the exported continuation tree are retained.

JAX executes the fixed-shape iteration on CPU or TPU. Degree-bucketed sparse
gathers/reductions avoid heavily contended scatter additions on TPU. Independent
NumPy float64 best responses audit each checkpoint:

```
lower = min_tau u(pi_current, tau)
upper = max_pi u(pi, tau_current)
remaining_saddle_gap = upper - lower
```

These are numerical brackets **for the declared finite payoff game**, not
statistical confidence intervals or formal interval-arithmetic certificates.
A small saddle gap only resolves the game given its chance samples, private
type support, selection support and frontier values.

Small-game tests cover matching pennies, an asymmetric matrix, the exact Kuhn
poker value, private-bit non-revelation/revelation, perfect recall, independent
best responses, baseline reconstruction, invalid keys and unresolved frontiers.
Native tests cover hidden sets/seeds, hidden bench selections, action-mask
consistency, recall and observation-preserving exact chance expansion.

## First production comparison

`tools/ico-audit.py` exports a first-battle-decision census. Its defaults are an
explicitly restricted model, not the full ICO objective:

- All four shipped APEX4 variants retain their shipped draw weights.
- Opponent cases cover the distinct public rosters of the built-in catalog.
  Every prior loadout matching a case's visible roster is included, with its
  roster reordered to avoid identifying types by incidental file order.
- The opponent chooses its private loadout and selection adversarially,
  before our private variant/pick is revealed.
- The most frequent `--picks` selections in the compiled preview are retained.
  Our retained probabilities are renormalized. Opponent selections use the
  reference catalog party's retained support, with adversarial probabilities.
  This is neither all legal selections nor the full shipped preview mixture.
- Initial preview uses the documented fixture seed `1,2,3,4`; the census is
  conditional on those initial states. It is not a posterior reconstruction of
  arbitrary midgame histories. Subsequent chance is sampled independently per
  node, with common random numbers across counterfactual actions/worlds.
- Horizon counts decision transitions, including forced switches, not necessarily
  complete turns. Terminal rewards are exact outcomes. Unresolved frontiers use
  `eval_leaf` as an estimate and separately retain the uncertainty interval [0,1].
- `--exact-chance` exhaustively enumerates raw random draw classes with their
  engine probabilities and float64 masses. It preserves full observation logs
  and refuses work-cap exhaustion, with no sampled fallback. It does not make
  a truncated frontier or restricted root population exact for the full game.

The baseline is the shipped live blind search algorithm at **81,000 iterations,
c=0.4, hp_buckets=16, legacy action selection**, without the optional timed
endgame solver. It reproduces the browser's two `observe` seed draws (compiled
preview, then the first battle decision). Chance sampling seeds and bot search
seeds are separate. `tools/ico-wasm-parity.cjs` checks native actions, visits and
means against the actual WASM implementation and refuses a stale WASM build.

The baseline first choice is recomputed in every hidden world. If two worlds
with the same observable history produce different native choices, that is
recorded as an information-boundary counterexample, not averaged away. The
baseline comparison game treats those actual native choices as exogenous
transitions, retaining own action observations for future decisions. ICO then
optimizes the continuation for both comparisons.

Consequently `estimated_root_loss_interval` measures the difference between
ICO root play and actual native root play **with optimized continuation in the
same finite surrogate game**. It is not the win-rate gain from replacing the
whole bot. A negative difference is permitted when the native root is not
information invariant. Full-game frontier uncertainty is reported separately;
`full_game_certified` remains false. Never sum local decision losses and call
the result a match win-rate improvement.

## Reproduction

```sh
cargo test -p nc2000-bot --example ico_export
uv run --project tpu pytest tpu/tests/test_ico.py -q
cargo build --release -p nc2000-bot --example ico_export
python3 tools/ico-audit.py --out tmp/ico-run --horizon 2 --samples 4 --threads 4
uv run --project tpu python -m gen2_tpu.ico tmp/ico-run/sample-07.game.json \
  --out tmp/ico-run/sample-07.result.json --iterations 4096
node tools/ico-wasm-parity.cjs tmp/ico-run/sample-07.game.json
```

For Colab, upload only selected game files and the ICO module/tests/worker.
`tools/ico-colab-worker.py` requires a TPU, records runtime and source hashes,
checkpoints each game atomically and refuses incompatible resumes. Keep the
Python TPU owner in one process; tests can run with `pytest.main` in that process.
Collect and verify results before stopping the task's session. Never stop an
unrelated Colab session.

`tools/ico-holdout.py` freezes a trained root policy on an independently sampled
game and optimizes only continuation. `tools/ico-report.py` verifies collected
input/runtime hashes before aggregating the census and sensitivity checks.
Saved WASM witnesses can be replayed directly with
`node tools/ico-wasm-parity.cjs data/ico-v1/wasm-hidden-witness.json`.

Increasing CFR iterations addresses only the saddle gap. Approaching the full
ICO requires independently expanding hidden-type and selection support,
integrating initial hidden chance, extending history/depth, replacing frontier
estimates with validated continuation bounds, and searching for unseen adversarial
teams. Learned leaf values alone do not supply a convergence certificate.

Theory: [CFR](https://papers.nips.cc/paper_files/paper/2007/hash/08d98638c6fcd194a4b1e6992063e944-Abstract.html),
[information-set search and strategy fusion](https://mlanctot.info/files/papers/aaai14-w4-iioos.pdf),
[public-belief-state solving / ReBeL](https://arxiv.org/abs/2007.13544).
