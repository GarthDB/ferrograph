#!/usr/bin/env python3
"""Self-test of the accuracy harness on the built-in synthetic corpus (no network; ~5 s).

Needs python3, cargo and a ferrograph binary: $FERROGRAPH, else ../../target/{debug,release}/ferrograph.
Run: python3 scripts/accuracy/test_harness.py
"""

import json
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent


def find_binary():
    cands = [os.environ.get("FERROGRAPH")] + [str(HERE.parents[1] / "target" / p / "ferrograph") for p in ("debug", "release")]
    return next((c for c in cands if c and Path(c).is_file()), None)


@unittest.skipUnless(find_binary(), "no ferrograph binary (set $FERROGRAPH or `cargo build`)")
class SyntheticCorpus(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.tmp = tempfile.TemporaryDirectory()
        tmp = Path(cls.tmp.name)
        (tmp / "c.toml").write_text(f'[[corpus]]\nname = "synthetic"\npath = "{HERE / "synthetic"}"\n')
        subprocess.run(
            [sys.executable, str(HERE / "groundtruth.py"), "--config", str(tmp / "c.toml"), "--ferrograph", find_binary(),
             "--work", str(tmp / "w"), "--out-md", str(tmp / "r.md"), "--out-json", str(tmp / "r.json")],
            check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=300,
        )
        cls.res = json.loads((tmp / "r.json").read_text())["results"][0]
        cls.md = (tmp / "r.md").read_text()

    @classmethod
    def tearDownClass(cls):
        cls.tmp.cleanup()

    def test_ground_truth_is_exactly_the_known_dead_fns(self):
        got = sorted(r["name"] for r in self.res["R"].values())
        self.assertEqual(got, sorted(["dead_c", "dead_d", "dead_m", "run", "unused_helper", "pub_dead", "crate_dead", "lib_dead"]))

    def test_known_dead_chain_is_found(self):
        found = {self.res["R"][n]["name"] for n in self.res["modes"]["default"]["F"] if n in self.res["R"]}
        self.assertTrue({"dead_c", "dead_d", "dead_m", "unused_helper", "lib_dead"} <= found, found)

    def test_model_matches_cli(self):
        for mode, sc in self.res["selfcheck"].items():
            self.assertEqual(sc["diff"], 0, mode)

    def test_misses_are_explained(self):
        buckets = {m["name"]: m["bucket"] for m in self.res["modes"]["default"]["misses"]}
        self.assertEqual(buckets.get("pub_dead"), "pub root: pub")
        self.assertEqual(buckets.get("crate_dead"), "pub root: pub(crate)/pub(super)")

    def test_lib_and_bin_split(self):
        self.assertEqual(sorted(set(self.res["classes"].values())), ["bin", "lib"])

    def test_report_renders(self):
        self.assertIn("pooled", self.md)

    def test_pr_mutation_finds_orphaned_fn(self):
        tmp = Path(self.tmp.name)
        subprocess.run(
            [sys.executable, str(HERE / "mutation.py"), "--config", str(tmp / "c.toml"), "--corpus", "synthetic",
             "--ferrograph", find_binary(), "--work", str(tmp / "w"), "--n", "3",
             "--out-md", str(tmp / "m.md"), "--out-json", str(tmp / "m.json")],
            check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=300,
        )
        runs = json.loads((tmp / "m.json").read_text())["runs"]
        hit = [r for r in runs if r["name"] == "cleanup"]
        self.assertEqual(len(hit), 1)
        self.assertEqual(len(hit[0]["oracle"]), 1)
        self.assertEqual(hit[0]["tp"], hit[0]["oracle"])


if __name__ == "__main__":
    unittest.main()
