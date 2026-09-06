#!/usr/bin/env python3
import copy
import importlib.util
import hashlib
import math
import json
import tempfile
import unittest
from pathlib import Path

spec = importlib.util.spec_from_file_location("evaluate_learning", Path(__file__).with_name("evaluate-learning.py"))
evaluation = importlib.util.module_from_spec(spec)
spec.loader.exec_module(evaluation)

spec = importlib.util.spec_from_file_location("fit_pick_prior", Path(__file__).with_name("fit-pick-prior.py"))
pick_prior = importlib.util.module_from_spec(spec)
spec.loader.exec_module(pick_prior)

spec = importlib.util.spec_from_file_location("confirm_learning", Path(__file__).with_name("confirm-learning.py"))
confirmation = importlib.util.module_from_spec(spec)
spec.loader.exec_module(confirmation)


class ConfirmationTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        self.gate = self.root / "gate.json"
        self.worker = self.root / "worker"
        self.worker.write_text("frozen worker")
        self.blocks = []
        for index in range(2):
            block = self.root / str(index)
            block.mkdir()
            config = {"schema": "nc2000-learning-arena-v1", "seed": 101+index, "games": 4,
                      "pool": str(self.worker), "dex": str(self.worker),
                      "agents": [{"program": str(self.worker), "args": ["--iters", "30"], "artifacts": []} for _ in range(2)]}
            (block / "config.json").write_text(json.dumps(config))
            (block / "launcher.json").write_text(json.dumps({"arena": str(self.worker)}))
            self.blocks.append(block)

    def test_registration_freezes_cumulative_checkpoints_and_inputs(self):
        confirmation.freeze(self.gate, self.blocks)
        gate = confirmation.verify(self.gate)
        self.assertEqual([block["checkpoint_games"] for block in gate["blocks"]], [4, 8])
        self.assertEqual(gate["sample_cap_games"], 8)
        self.worker.write_text("different worker")
        with self.assertRaisesRegex(ValueError, "artifact changed"):
            confirmation.verify(self.gate)

    def test_started_runs_cannot_be_registered_after_viewing_results(self):
        (self.blocks[0] / "games.jsonl").touch()
        with self.assertRaisesRegex(ValueError, "before any games"):
            confirmation.freeze(self.gate, self.blocks)

    def test_registration_rejects_adaptive_candidate_changes(self):
        path = self.blocks[1] / "config.json"
        config = json.loads(path.read_text())
        config["agents"][0]["args"] += ["--c", "0.4"]
        path.write_text(json.dumps(config))
        with self.assertRaisesRegex(ValueError, "remain fixed"):
            confirmation.freeze(self.gate, self.blocks)

    def test_registration_rejects_repeated_schedules(self):
        with self.assertRaisesRegex(ValueError, "repeated seed"):
            confirmation.freeze(self.gate, self.blocks + self.blocks[:1])


class PairedEvaluationTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.path = Path(self.directory.name) / "games.jsonl"
        self.manifest = {"type": "manifest", "config": {"schema": "nc2000-learning-arena-v1", "games": 4}}
        self.rows = [{
            "type": "game", "game": i, "pair": i//2, "swap": i%2,
            "outcome": "p1", "score": 1.0 if i%2 == 0 else 0.0,
            "team_ids": [0, 1], "battle_seed": f"pair-{i//2}",
            "capped": False, "turns": 10,
            "decision_ns": [[100], [200]], "worker_ns": [[90], [190]],
        } for i in range(4)]

    def write(self, rows=None):
        with self.path.open("w") as file:
            for row in [self.manifest] + (self.rows if rows is None else rows):
                file.write(json.dumps(row) + "\n")

    def test_opposite_seat_wins_cancel_within_each_pair(self):
        self.write()
        result = evaluation.summarize(self.path)
        self.assertEqual(result["pair_scores"], [.5, .5])
        self.assertEqual(result["score"], .5)
        self.assertFalse(result["positive_strength_evidence"])

    def test_incomplete_pairs_do_not_become_extra_independent_games(self):
        self.write(self.rows[:3])
        with self.assertRaisesRegex(ValueError, "incomplete"):
            evaluation.summarize(self.path)
        result = evaluation.summarize(self.path, require_complete=False)
        self.assertEqual(result["complete_pairs"], 1)
        self.assertIsNone(result["normal95"])
        self.assertFalse(result["positive_strength_evidence"])

    def test_two_winning_pairs_are_not_significant_with_zero_sample_variance(self):
        for row in self.rows:
            row.update(outcome="p1" if row["swap"] == 0 else "p2", score=1.0)
        self.write()
        result = evaluation.summarize(self.path)
        self.assertEqual(result["normal95"], [1.0, 1.0])
        self.assertLess(result["hoeffding95"][0], .5)
        self.assertLess(result["empirical_bernstein95"][0], .5)
        self.assertFalse(result["positive_strength_evidence"])

    def test_capped_games_cannot_be_substituted_for_real_draws(self):
        self.rows[0].update(capped=True, outcome="tie", score=.5)
        self.write()
        with self.assertRaisesRegex(ValueError, "capped"):
            evaluation.summarize(self.path)

    def test_changed_teams_break_pairing(self):
        self.rows[1]["team_ids"] = [1, 0]
        self.write()
        with self.assertRaisesRegex(ValueError, "changed teams"):
            evaluation.summarize(self.path)

    def test_duplicate_completed_games_are_rejected(self):
        self.write(self.rows + [copy.deepcopy(self.rows[0])])
        with self.assertRaisesRegex(ValueError, "duplicate"):
            evaluation.summarize(self.path)

    def test_score_cannot_disagree_with_engine_outcome(self):
        self.rows[1]["score"] = 1.0
        self.write()
        with self.assertRaisesRegex(ValueError, "score"):
            evaluation.summarize(self.path)

    def prepare_shards(self):
        self.manifest["config"].update(seed=101, agents=[{"args": ["--iters", "30000"]} for _ in range(2)])
        self.manifest["hashes"] = {"arena": "arena", "pool": "pool", "dex": "dex",
                                   "agents": [{"program": "worker", "artifacts": []} for _ in range(2)]}
        self.write()
        first = self.path
        self.path = first.with_name("second.jsonl")
        self.manifest["config"]["seed"] = 103
        for row in self.rows:
            row["battle_seed"] = "second-" + row["battle_seed"]
        self.write()
        return first, self.path

    def test_shards_combine_pairs_without_recounting_game_outcomes_as_independent(self):
        paths = self.prepare_shards()
        result = evaluation.combine(paths)
        self.assertEqual(result["games"], 8)
        self.assertEqual(result["complete_pairs"], 4)
        self.assertEqual(result["score"], .5)
        self.assertFalse(result["positive_strength_evidence"])

    def test_combining_a_run_twice_is_rejected(self):
        first, _ = self.prepare_shards()
        with self.assertRaisesRegex(ValueError, "duplicate run seed"):
            evaluation.combine([first, first])

    def test_changed_candidate_cannot_enter_the_same_confirmation(self):
        paths = self.prepare_shards()
        self.manifest["config"]["agents"][0]["args"] += ["--c", "0.4"]
        self.write()
        with self.assertRaisesRegex(ValueError, "identities differ"):
            evaluation.combine(paths)


class PickPriorTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        root = Path(self.directory.name)
        self.path = root / "preview.jsonl"
        pool_path = root / "pool.json"
        sets = [{"species": name} for name in "abcdef"]
        pool_path.write_text(json.dumps({"teams": [{"id": "one", "sets": sets}, {"id": "two", "sets": sets}]}))
        self.manifest = {"type": "manifest", "config": {
            "schema": "nc2000-preview-collection-v1", "games": 4,
            "pool": str(pool_path), "agents": [{}, {}],
        }, "hashes": {"pool": "sha256:" + hashlib.sha256(pool_path.read_bytes()).hexdigest(), "agents": [{}, {}]}}
        lines = [f"|poke|p{side+1}|{name}, L50, M|item" for side in range(2) for name in "abcdef"]
        self.rows = []
        for game in range(4):
            frames = []
            for side in range(2):
                frames.append({"side": side, "agent": side ^ (game % 2), "turn": 0,
                    "frame": {"lines": lines, "request": {"teamPreview": True, "side": {
                        "pokemon": [{"details": f"{name}, L50, M"} for name in "abcdef"],
                    }}, "legal_actions": ["team 3, 1, 2"]},
                    "response": {"action": "team 3, 1, 2", "iterations": 30000, "legality_drift": 0, "projections": 0}})
            self.rows.append({"type": "preview", "game": game, "pair": game//2, "swap": game%2,
                              "team_ids": [0, game//2], "agent_seeds": [2*game, 2*game+1], "frames": frames})

    def write(self):
        self.path.write_text("\n".join(json.dumps(row) for row in [self.manifest] + self.rows) + "\n")

    def test_publicly_identical_enemy_teams_share_the_prior_and_lead_order_is_preserved(self):
        self.write()
        model = pick_prior.fit(self.path, .2)
        own = [row for row in model["rows"] if row["team"] == "one" and row["side"] == 0 and row["enemy_preview"]]
        self.assertEqual(len(own), 1)
        self.assertEqual(own[0]["choices"], [{"species": ["c", "a", "b"], "count": 4}])

    def test_incomplete_collection_is_rejected(self):
        self.rows.pop()
        self.write()
        with self.assertRaisesRegex(ValueError, "incomplete"):
            pick_prior.fit(self.path, .2)

    def test_duplicate_query_seed_is_rejected(self):
        self.rows[1]["agent_seeds"] = list(reversed(self.rows[0]["agent_seeds"]))
        self.write()
        with self.assertRaisesRegex(ValueError, "duplicate query seed"):
            pick_prior.fit(self.path, .2)


class BettingIntervalTests(unittest.TestCase):
    def test_mixture_has_unit_expectation_and_controls_null_rejection_exactly(self):
        trials = 12
        for mean in [.2, .5, .8]:
            expectation = rejected = 0.0
            for wins in range(trials+1):
                probability = math.comb(trials, wins)*mean**wins*(1-mean)**(trials-wins)
                wealth = evaluation.log_betting_evalue({0.0: trials-wins, 1.0: wins}, mean)
                opposite = evaluation.log_betting_evalue({1.0: trials-wins, 0.0: wins}, 1-mean)
                expectation += probability*math.exp(wealth)
                rejected += probability*int(max(wealth, opposite) >= math.log(40))
            self.assertAlmostEqual(expectation, 1.0, places=12)
            self.assertLessEqual(rejected, .05)

    def test_null_expectation_with_tied_pairs_is_also_one(self):
        trials = 12
        expectation = 0.0
        for wins in range(trials+1):
            for losses in range(trials-wins+1):
                ties = trials-wins-losses
                probability = math.comb(trials, wins)*math.comb(trials-wins, losses)*.2**(wins+losses)*.6**ties
                wealth = evaluation.log_betting_evalue({0.0: losses, .5: ties, 1.0: wins}, .5)
                expectation += probability*math.exp(wealth)
        self.assertAlmostEqual(expectation, 1.0, places=12)

    def test_repeated_checkpoints_control_the_probability_of_ever_rejecting(self):
        for mean in [.2, .5, .8]:
            surviving = {0: 1.0}
            rejected = 0.0
            for trial in range(1, 41):
                next_surviving = {}
                for previous_wins, mass in surviving.items():
                    for outcome, probability in [(0, 1-mean), (1, mean)]:
                        wins = previous_wins+outcome
                        counts = {0.0: trial-wins, 1.0: wins}
                        opposite = {1.0: trial-wins, 0.0: wins}
                        crossed = max(evaluation.log_betting_evalue(counts, mean),
                                      evaluation.log_betting_evalue(opposite, 1-mean)) >= math.log(40)
                        if crossed:
                            rejected += mass*probability
                        else:
                            next_surviving[wins] = next_surviving.get(wins, 0.0)+mass*probability
                surviving = next_surviving
            self.assertLessEqual(rejected, .05)

    def test_intervals_include_fractional_draw_scores_and_invert_each_tail(self):
        samples = [0.0, .25, .5, .5, .75, 1.0]*8
        lower, upper = evaluation.betting_interval(samples)
        self.assertLess(lower, .5)
        self.assertGreater(upper, .5)
        self.assertAlmostEqual(lower, 1-upper, places=12)
        self.assertLess(evaluation.betting_interval([1.0, 1.0])[0], .5)
        self.assertGreater(evaluation.betting_interval([1.0]*32)[0], .5)


if __name__ == "__main__":
    unittest.main()
