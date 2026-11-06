#!/usr/bin/env python3
"""Falsification controls for the historical measurement audit."""
import copy
import importlib.util
import json
from pathlib import Path
import random
import unittest

spec = importlib.util.spec_from_file_location(
    "measurement_validation", Path(__file__).with_name("measurement-validation.py"))
audit = importlib.util.module_from_spec(spec)
spec.loader.exec_module(audit)


class MeasurementAuditTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.report = json.loads(audit.git_blob(audit.REPORT_PATH))
        cls.parties = json.loads(audit.git_blob(audit.PARTY_PATH))

    def test_all_historical_assignments_and_scores(self):
        result = audit.accounting(self.report, self.parties)
        self.assertEqual((result["red_wins"], result["blue_wins"], result["draws"]), (42, 22, 0))
        self.assertEqual(result["records_at_or_above_cap"], [])
        self.assertEqual(result["paired_score_histogram"], {1.0: 14, .5: 14, 0.0: 4})

    def test_attack_only_needs_own_seed_and_seat(self):
        # Literal held-out referee truth, not arguments to the attack.
        self.assertEqual(audit.recover_secrets(11792330719408480657, 1), {
            "battle_seed": "64677,10395,40644,54272",
            "blue_seed_u64": 14175318832926202200,
            "blue_effective_seed_u32": 14175318832926202200 & 0xFFFFFFFF,
        })
        self.assertEqual(audit.boundary(self.report)["recovered_games"], 64)

    def test_independently_assigned_seeds_do_not_pass_the_attack(self):
        # Synthetic diagnostic only; this generator is not the proposed
        # confirmation seed source. All three values are drawn separately.
        altered = copy.deepcopy(self.report)
        rng = random.Random(20261103)
        for game in altered["duel_tiers"][0]["game_records"]:
            game["battle_seed"] = audit.battle_seed(rng.getrandbits(64))
            game["blue_seed"] = rng.getrandbits(64)
            game["red_seed"] = rng.getrandbits(64)
        self.assertEqual(audit.boundary(altered)["recovered_games"], 0)

    def test_missing_record_is_not_silently_dropped(self):
        altered = copy.deepcopy(self.report)
        altered["duel_tiers"][0]["game_records"].pop()
        with self.assertRaisesRegex(ValueError, "incomplete history"):
            audit.accounting(altered, self.parties)

    def test_corrupted_score_is_detected(self):
        altered = copy.deepcopy(self.report)
        altered["duel_tiers"][0]["game_records"][0]["blue_score"] = 1.0
        with self.assertRaisesRegex(ValueError, "wrong block scores"):
            audit.accounting(altered, self.parties)

    def test_corrupted_side_or_seed_is_detected(self):
        for field, value in (("blue_is_p1", False), ("blue_seed", 0), ("index", 1)):
            with self.subTest(field=field):
                altered = copy.deepcopy(self.report)
                altered["duel_tiers"][0]["game_records"][0][field] = value
                with self.assertRaisesRegex(ValueError, f"wrong {field}"):
                    audit.accounting(altered, self.parties)

    def test_corrupted_aggregate_is_detected(self):
        altered = copy.deepcopy(self.report)
        altered["duel_tiers"][0]["red_wr"] = .75
        with self.assertRaisesRegex(ValueError, "wrong aggregate red_wr"):
            audit.accounting(altered, self.parties)

    def test_record_reordering_is_not_silently_paired(self):
        altered = copy.deepcopy(self.report)
        games = altered["duel_tiers"][0]["game_records"]
        games[0], games[2] = games[2], games[0]
        with self.assertRaisesRegex(ValueError, "wrong index"):
            audit.accounting(altered, self.parties)


if __name__ == "__main__":
    unittest.main()
