#!/usr/bin/env python3
import copy
import importlib.util
import json
import tempfile
import unittest
from pathlib import Path

spec = importlib.util.spec_from_file_location("evaluate_learning", Path(__file__).with_name("evaluate-learning.py"))
evaluation = importlib.util.module_from_spec(spec)
spec.loader.exec_module(evaluation)


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


if __name__ == "__main__":
    unittest.main()
