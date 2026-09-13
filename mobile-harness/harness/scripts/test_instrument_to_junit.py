#!/usr/bin/env python3
"""Unit tests for instrument_to_junit.py against captured `am instrument -r` output.

run: python3 harness/scripts/test_instrument_to_junit.py
"""
import json
import subprocess
import sys
import tempfile
import unittest
import xml.etree.ElementTree as ET
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import instrument_to_junit as conv  # noqa: E402

DATA = HERE / "testdata"


def run(raw: Path):
    parsed = conv.parse(raw.read_text())
    return parsed, conv.summarize(parsed)


class ParseTests(unittest.TestCase):
    def test_pass(self):
        parsed, summary = run(DATA / "pass.raw")
        self.assertTrue(summary["ok"])
        self.assertEqual(summary["total"], 1)
        self.assertEqual(parsed["tests"][0]["name"], "tappingIncrementUpdatesCount")
        self.assertEqual(parsed["tests"][0]["status"], "passed")

    def test_fail_keeps_multiline_stack_and_skipped(self):
        parsed, summary = run(DATA / "fail.raw")
        self.assertFalse(summary["ok"])
        self.assertTrue(summary["completed"])
        self.assertEqual((summary["passed"], summary["failed"], summary["skipped"], summary["errors"]), (1, 1, 1, 0))
        failed = next(t for t in parsed["tests"] if t["status"] == "failed")
        self.assertIn("CounterScreenTest.kt:22", failed["stack"])
        self.assertTrue(failed["stack"].startswith("java.lang.AssertionError"))

    def test_crash_mid_test_is_an_error(self):
        parsed, summary = run(DATA / "crash.raw")
        self.assertFalse(summary["ok"])
        self.assertFalse(summary["completed"])
        statuses = [(t["classname"], t["status"]) for t in parsed["tests"]]
        self.assertIn(("dev.tochi.mobileharness.CounterScreenTest", "error"), statuses)
        self.assertIn(("harness.instrumentation", "error"), statuses)
        self.assertIn("boom", parsed["tests"][0]["stack"])

    def test_missing_instrumentation_is_an_error(self):
        parsed, summary = run(DATA / "notfound.raw")
        self.assertFalse(summary["ok"])
        self.assertEqual(summary["errors"], 1)
        self.assertIn("could not start", parsed["tests"][0]["message"])

    def test_cli_writes_junit_and_summary_and_exit_code(self):
        with tempfile.TemporaryDirectory() as tmp:
            junit, summary = Path(tmp, "junit.xml"), Path(tmp, "summary.json")
            proc = subprocess.run([sys.executable, str(HERE / "instrument_to_junit.py"), str(DATA / "fail.raw"), str(junit), str(summary)], capture_output=True, text=True)
            self.assertEqual(proc.returncode, 1, proc.stderr)
            root = ET.parse(junit).getroot()
            suite = root.find("testsuite")
            self.assertEqual(suite.get("tests"), "3")
            self.assertEqual(suite.get("failures"), "1")
            self.assertEqual(len(root.findall(".//failure")), 1)
            self.assertEqual(json.loads(summary.read_text())["failed"], 1)

            proc = subprocess.run([sys.executable, str(HERE / "instrument_to_junit.py"), str(DATA / "pass.raw"), str(junit), str(summary)], capture_output=True, text=True)
            self.assertEqual(proc.returncode, 0, proc.stderr)


if __name__ == "__main__":
    unittest.main()
