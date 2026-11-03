# Measurement validation: first failing gate

The historical record accounting passes. The seed information boundary fails.
Per `docs/MEASUREMENT-VALIDATION-PROTOCOL.md`, prospective D/K matches were
**not started**. This is a falsification result, not a completed validation
of the remaining gates, and not an implementation of the replacement runner.

## Independent accounting

The Python auditor restores the original report at `872d8ed8`, independently
regenerates all party draws and physical seat/seed assignments, and checks
every record and aggregate. It finds:

- 64 games / 32 mirrored blocks: Red 42 wins, Blue 22 wins, no ties.
- Block scores: 14 at 1, 14 at 0.5, 4 at 0; mean 0.65625.
- Paired standard error 0.0612269542; the archived descriptive normal interval
  is correctly calculated. This is not a new confirmation claim.
- Maximum recorded turn 102; no recorded game reaches the 250-turn cap.
  That cap does not explain these recorded results.
- 16 exact-same-type games: Red 9 wins, Blue 7 wins. The other 48 games use
  independently drawn types from the same pool, rather than identical types.
- Five recorded solver interventions.

These checks validate consistency of the saved records, not the actual
terminal winners: the historical run did not retain all action logs. No
independent engine rescore of those 64 historical games is possible from the
saved score labels alone. No full historical replay was run in this phase.

## Minimal information-boundary counterexample

The scheduling code calls one `SplitMix64` stream consecutively to obtain:

1. the game's complete initial PRNG seed;
2. the physical seat 0 agent seed;
3. the physical seat 1 agent seed.

The mirror swaps algorithm roles, preserving these physical assignments.
Red receives its full 64-bit seed. SplitMix64's output transform is invertible:
right-xor shifts can be undone, and both odd multipliers have inverses modulo
2^64. Its state increment is public. Consequently, **Red's own seed and own
seat alone recover both the game seed and Blue's seed**.

The attack function `recover_secrets(own_seed, own_side)` takes exactly those
two inputs. It does not read the run master seed, block number, opponent
seed, real teams, game object, or future choices. Referee truth is consulted
only afterwards, to check the prediction.

For historical game 0:

| Item | Value |
|---|---|
| Allowed attack input: Red seed | `11792330719408480657` |
| Allowed attack input: Red physical side | `1` (seat 2) |
| Recovered game seed | `64677,10395,40644,54272` |
| Recovered Blue seed | `14175318832926202200` |

Both recovered values match the saved record. Taking the low 32 bits also
recovers the effective seed supplied to the product ProtocolAgent.
The same attack succeeds on **all 64 games**, across both physical sides.
The current `omniscient_audit` scheduler still contains this construction;
the finding is not restricted to the archived binary.

This falsifies the protocol's seed separation requirement (A4/A5/A10/A22).
Putting these seed values in separate worker processes would not repair the
problem. It is an information channel through the worker's own seed.

### What this does not establish

The historical T2 path does not contain this inverse attack. This finding
does **not** demonstrate that T2 used recovered secrets, that any of the 42
wins was caused by the seed coupling, or that a correctly isolated rerun
would reach parity. Nor does it contradict the arithmetic 42–22 count.
The new recovery probe is an auditor's attack on the measurement interface;
its success must not be attributed retroactively to historical T2 behavior.

No policy-selection, Nash-mixing, or party-weight explanation follows from
this result. The narrow conclusion is that the proposed blind-information
guarantee does not hold for the old/current scheduler. Historical win-rate
causality remains unresolved.

## Controls and reproducibility

Eight automated tests pass, including corrupted individual score, aggregate,
seat/seed/index, missing-record and reordered-record rejection. The attack
also runs against a synthetic fixture with separately drawn agent and game
seeds: it recovers 0/64. This negative control is a diagnostic, not a proof
that the fixture's PRNG or a future runner is cryptographically secure.

```sh
python3 tools/test-measurement-validation.py -v
python3 tools/measurement-validation.py --out data/measurement-validation-v1
```

The audit deliberately exits **2** after writing its report, meaning the
confirmation gate failed. Tool errors and invalid accounting raise errors
instead. The full per-game recovery witnesses and historical/current input
hashes are in `audit.json`; the unmodified historical report is retained as
`historical-report.json`.

## Remaining work and stopping decision

Do not continue to a large win-rate run using these seed assignments. The
next runner needs independently generated role/game/type randomness (or
reviewed cryptographic domain separation with a secret master key), while
preserving the product's own 32-bit PRNG behavior. Keep referee manifests
inaccessible to workers. Re-run the recovery probe as a negative control,
then implement and pass the other information-boundary, product-identity,
action-mapping, terminal-scoring and clock-control gates. A changed scheduler
requires a new confirmation manifest and unused seeds.

The replacement worker/referee, T2-W, fresh K preview, 16 A/A blocks, actual
worker parity suite and 2,048 prospective confirmation games remain unrun.
No gate is marked passed merely because this first failure was found.
