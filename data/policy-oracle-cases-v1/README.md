# Historical T2 decisions and the mixing question

This is a diagnostic investigation of the corrected 81k bot's historical
22–42 result against T2, recorded at `872d8ed8`. It does **not** establish
that the selected positions require mixing under optimal blind play.

Predicting a move and punishing it is not itself evidence that randomization
would help. For a valid zero-sum payoff matrix, compare the best pure row's
worst-case value with the optimal mixed row's worst-case value. A positive
gap means mixing is necessary for that matrix. A pure saddle can remain
unexploitable even when its action is perfectly predictable. A bot choosing
a different, inferior pure action can still be exploitable.
Search seeds also induce implicit randomization in the live agent. Root
visit fractions are not its execution probabilities; this experiment does
not identify the complete seed-marginal policy at each information set.

## Reproduction and diagnostic selection

Historical pair indices 0, 3 and 12 were chosen before reading their move
traces: the first exact-type double-loss pair, one different-type double-loss
pair, and the next exact-type double-loss pair. All six were known losses;
their replay is not a new win-rate estimate or a prevalence sample.

The six scores and terminal turns reproduce the historical records:
games 0/1 end at turns 36/18, 6/7 at 10/29, and 24/25 at 11/33.
The original run did not retain move traces, so this does not establish
historical action-by-action identity. In game 6, the time-limited endgame
solver intervenes once on the final turn in the replay, versus zero in the
historical record. Outcome and terminal turn are unchanged. The other five
games reproduce the recorded zero solver interventions.

The historical prior is restored, including Blue's uniform APEX4 weights.
The production search implementation, team sets and compiled preview choices
are unchanged. Trace generation reads diagnostics without advancing search
or decision RNG. `analyst_truth_*` and sampled worlds are retrospective
diagnostics, never inputs to either player's decision.

## Concrete sequences

All game and turn numbers below are the zero-based game index and engine turn.

- **Games 24 and 25, turns 2–4:** Blue Starmie uses Ice Beam three times.
  Red Snorlax uses Curse twice, then Double-Edge and KOs Starmie. Both policy
  predictions select Ice Beam at turns 3 and 4 in both games. Game 24's Blue
  search allocates 78,359 of 81,000 visits to Ice Beam at turn 4; those visits
  are diagnostics, not an execution probability.
- **Game 25, turn 11:** Blue Snorlax has 114/292 HP and selects Rest
  (80,820 root visits). Both opponent-policy samples predict Rest. Red uses
  Curse while Blue recovers. This is **not** a demonstrated failure to mix:
  Rest dominates all other Blue actions in the measured continuation games.
- **Game 7, turns 18–23:** In the Heracross endgame, Blue uses Curse at
  turns 18/19 and Red attacks with Megahorn; Blue attacks at turn 20 and Red
  uses Curse; Blue uses Rest at turn 23 and Red uses Curse. Both model
  predictions match Blue on those turns. The measured matrices nevertheless
  mostly choose pure strategies, so this read-and-response sequence is not
  evidence by itself of a rock-paper-scissors position.

A single-decision intervention changes only Blue's turn-2 action in games
24/25 to `switch 2` (Snorlax), **after** Red chooses its action. Priors,
preview, earlier decisions, seeds and subsequent algorithms are preserved.
Game 24 changes from a turn-11 loss to a turn-28 win; game 25 remains a loss,
at turn 13 instead of 33. This is a seeded counterfactual, not a statistical
improvement, dominance, or mixing certificate. Changed engine random-number
consumption after the intervention can contribute to different outcomes.

## What the matrices actually measure

`policy_oracle_matrix` replays every earlier engine event exactly, then
reconstructs the production ProtocolAgent from its own frames. At each target
it asserts both the actual action and every root visit count against the
trace. Earlier searches can be omitted because installation RNG is separate;
the equality assertions check this at all seven examined roots.

The 81k search is frozen. For each joint root input, fresh determinizations
and chance samples evaluate that immutable tree; unvisited continuations use
the configured rollout and heuristic horizon. Two continuation extractions
are compared: most-visited and visit-sampled. Neither is repeated production
re-search, and neither supplies optimal-play continuation values.

Columns are fixed input policies common to 4,096 sampled opponent worlds;
all 131,072 evaluation draws per cell also check legality. Opponent private
type-contingent policies are not exhaustively represented. In the Heracross
cases, only Curse/Megahorn/Rest are common; the varying fourth attack is
excluded. Thus even an exact solution of this sampled matrix would not
certify a full blind-game saddle.

There are 336 cells across seven roots and two continuations, totaling
44,040,192 draws in the independent confirmation batch. Initial screening
used 4,096 draws per cell and different seeds. Root continuation cells are
mostly nonterminal: **the numbers below are search-evaluation units on
[0,1], not measured full-game win rates.**

| Root | Played | Best pure, most-visited tail | Pure value | Mixed value | Mix gain | Visit-sampled tail mix gain |
|---|---|---|---:|---:|---:|---:|
| Game 24 turn 2 | Ice Beam | Psychic | .42397 | .42578 | .00181 | 0 |
| Game 24 turn 3 | Ice Beam | Psychic | .36474 | .36474 | 0 | 0 |
| Game 24 turn 4 | Ice Beam | Ice Beam | .31432 | .31545 | .00113 | 0 |
| Game 7 turn 18 | Curse | Curse | .29569 | .29718 | .00149 | 0 |
| Game 7 turn 20 | Megahorn | Megahorn | .36581 | .36581 | 0 | 0 |
| Game 7 turn 23 | Rest | Megahorn | .28084 | .28084 | 0 | 0 |
| Game 25 turn 11 | Rest | Rest | .36930 | .36930 | 0 | 0 |

Cell standard errors are at most about .0014. The small positive gains are
not a demonstrated robust mixing requirement. A nominal simultaneous
Hoeffding half-width over all 336 bounded cell means is .00603 at 95%, under
the independent-draw model; it does not cover continuation/model error or
unrepresented opponent policies. The change of continuation itself removes
all positive point-estimated mixing gains.

For the most-visited continuation at game 25 turn 11, the relevant excerpt is:

| Blue action | Red Curse | Red Double-Edge |
|---|---:|---:|
| Curse | .30256 | .00386 |
| Double-Edge | .11554 | .00000 |
| Rest | .36930 | .41290 |
| Sleep Talk | .30726 | .02409 |

Rest is also best against the four remaining columns. This demonstrates why
being read on Rest cannot alone establish that mixing was required. It is a
strong pure-action result **within this continuation model**, not a claim
that the full game has been solved.

The current evidence supports investigating continuation valuation and
opponent-response modeling as well as execution randomization. It does not
justify attributing the 42–22 defeat to lack of mixing, nor concluding that
correcting party weights restores parity. The actual optimal blind-game
classification of the initial Starmie position remains unresolved.

## Commands and artifacts

Restore the original prior before reproducing:

```sh
mkdir -p tmp/historical-t2-audit
git show 872d8ed8:data/belief-pool-v3/belief-pool.json > tmp/historical-t2-audit/belief-pool.json
cargo build --release -p nc2000-bot --example omniscient_audit --example policy_oracle_matrix
target/release/examples/omniscient_audit --corrected-product --tiers T2_Rules_Bayesian_Oracle_BR --iters 81000 --pairs 32 --same-budget --k-oracle 2 --max-turns 250 --seed 2026110307 --threads 6 --endgame-states 0 --prior-json tmp/historical-t2-audit/belief-pool.json --trace-pairs 0,3,12 --trace-dir tmp/historical-t2-audit/reproduced-traces --out tmp/historical-t2-audit/reproduced-duels.json
target/release/examples/omniscient_audit --corrected-product --tiers T2_Rules_Bayesian_Oracle_BR --iters 81000 --pairs 32 --same-budget --k-oracle 2 --max-turns 250 --seed 2026110307 --threads 2 --endgame-states 0 --prior-json tmp/historical-t2-audit/belief-pool.json --trace-pairs 12 --blue-override-turn 2 --blue-override-action 'switch 2' --trace-dir tmp/historical-t2-audit/reproduced-fork --out tmp/historical-t2-audit/reproduced-fork.json
target/release/examples/policy_oracle_matrix data/policy-oracle-cases-v1/matrix-requests.json tmp/historical-t2-audit/reproduced-matrices.json
```

- `traces/`: six full decision traces, including search diagnostics and logs.
- `fork-switch-snorlax/`: the two intervention traces.
- `matrices.json`: all confirmation cells, uncertainties, policies and scope.
- `summary.json`: historical comparison, fork outcomes and matrix summaries.
- `artifact-sha256.json`: retained machine-artifact hashes.

The diagnostic pair filter suppresses aggregate confidence intervals: these
loss-selected games are not an independently sampled strength measurement.
