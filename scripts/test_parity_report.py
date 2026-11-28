import copy
import unittest

from parity_report import progress


class ProgressTests(unittest.TestCase):
    def setUp(self):
        self.plan = {
            "baseline_complete": False,
            "domains": [{"id": "A", "parity_verified": False, "implementation_complete": False}],
            "scenarios": [],
        }

    def test_incomplete_inventory_has_no_percentage(self):
        self.assertIsNone(progress(self.plan))
        self.plan["baseline_complete"] = True
        with self.assertRaises(AssertionError):
            progress(self.plan)

    def test_only_evidence_backed_scenarios_count(self):
        self.plan["baseline_complete"] = True
        self.plan["scenarios"] = [
            {"id": "A1", "domain": "A", "implementation_complete": True,
             "parity_verified": True, "current_acceptance_artifacts": ["synthetic-evidence"]},
            {"id": "A2", "domain": "A", "implementation_complete": True, "parity_verified": False},
        ]
        self.assertEqual(progress(self.plan), {"total": 2, "implemented": 2, "verified": 1})
        broken = copy.deepcopy(self.plan)
        broken["scenarios"][0]["current_acceptance_artifacts"] = []
        with self.assertRaises(AssertionError):
            progress(broken)
        self.plan["domains"].append({"id": "B", "parity_verified": False, "implementation_complete": False})
        with self.assertRaises(AssertionError):
            progress(self.plan)


if __name__ == "__main__":
    unittest.main()
