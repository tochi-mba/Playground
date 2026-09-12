#!/usr/bin/env python3
"""Convert the raw output of `adb shell am instrument -w -r` into JUnit XML plus a summary.

usage: instrument_to_junit.py <instrument.raw> <junit.xml> <summary.json>

Exit status is 0 only when the run completed and every test passed. Crashes, a runner that
never started, and tests that never finished all become synthetic <error> cases so nothing
is silently lost.
"""
from __future__ import annotations

import json
import re
import sys
import xml.etree.ElementTree as ET
from pathlib import Path

STATUS_PREFIX = "INSTRUMENTATION_STATUS: "
STATUS_CODE_PREFIX = "INSTRUMENTATION_STATUS_CODE: "
RESULT_PREFIX = "INSTRUMENTATION_RESULT: "
CODE_PREFIX = "INSTRUMENTATION_CODE: "
FAILED_PREFIX = "INSTRUMENTATION_FAILED: "
ABORTED_PREFIX = "INSTRUMENTATION_ABORTED: "

# AndroidJUnitRunner status codes. 1 = test started.
STATUS_BY_CODE = {0: "passed", -1: "error", -2: "failed", -3: "skipped", -4: "skipped"}
RESULT_OK = -1  # Activity.RESULT_OK; anything else means the instrumentation did not finish cleanly

_XML_BAD = re.compile("[^\t\n\r\x20-퟿-�]")


def _clean(text: str) -> str:
    return _XML_BAD.sub("", text or "")


def parse(text: str) -> dict:
    tests: list[dict] = []
    running: dict | None = None
    status: dict[str, str] = {}
    result: dict[str, str] = {}
    last: tuple[dict, str] | None = None
    final_code: int | None = None
    fatal: str | None = None
    preamble_error: str | None = None

    for line in text.splitlines():
        if line.startswith(STATUS_CODE_PREFIX):
            code = int(line[len(STATUS_CODE_PREFIX):].strip())
            cls, name = status.get("class", ""), status.get("test", "")
            if not name:
                # ActivityManager emits a code -1 block with Error=... when the runner cannot start.
                if status.get("Error"):
                    preamble_error = status["Error"]
            elif code == 1:
                running = {"classname": cls, "name": name}
            else:
                test = running if running and (running["classname"], running["name"]) == (cls, name) else {"classname": cls, "name": name}
                test["status"] = STATUS_BY_CODE.get(code, "error")
                test["message"] = "" if code in STATUS_BY_CODE else f"unknown status code {code}"
                test["stack"] = status.get("stack", "")
                tests.append(test)
                running = None
            status, last = {}, None
        elif line.startswith(STATUS_PREFIX):
            key, _, value = line[len(STATUS_PREFIX):].partition("=")
            status[key] = value
            last = (status, key)
        elif line.startswith(RESULT_PREFIX):
            key, _, value = line[len(RESULT_PREFIX):].partition("=")
            result[key] = value
            last = (result, key)
        elif line.startswith(CODE_PREFIX):
            final_code = int(line[len(CODE_PREFIX):].strip())
            last = None
        elif line.startswith(FAILED_PREFIX) or line.startswith(ABORTED_PREFIX):
            fatal = line
            last = None
        elif last is not None:
            holder, key = last
            holder[key] += "\n" + line

    if running is not None:
        detail = result.get("longMsg") or result.get("shortMsg") or "test started but never reported a result"
        running.update(status="error", message="test did not finish (process crashed?)", stack=detail)
        tests.append(running)

    completed = final_code == RESULT_OK and fatal is None and "shortMsg" not in result
    if not completed:
        detail = preamble_error or fatal or result.get("longMsg") or result.get("shortMsg") or result.get("stream", "").strip() or f"INSTRUMENTATION_CODE={final_code}"
        if fatal and fatal.startswith(FAILED_PREFIX):
            message = "instrumentation could not start (test APK not installed, or wrong runner/package)"
        else:
            message = "instrumentation did not complete (app process crashed or was killed)"
        tests.append({"classname": "harness.instrumentation", "name": "run", "status": "error", "message": message, "stack": detail})

    return {"tests": tests, "completed": completed, "stream": result.get("stream", "")}


def summarize(parsed: dict) -> dict:
    counts = {"passed": 0, "failed": 0, "errors": 0, "skipped": 0}
    for test in parsed["tests"]:
        key = {"passed": "passed", "failed": "failed", "error": "errors", "skipped": "skipped"}[test["status"]]
        counts[key] += 1
    total = len(parsed["tests"])
    ok = parsed["completed"] and counts["failed"] == 0 and counts["errors"] == 0 and total > 0
    return {"total": total, **counts, "completed": parsed["completed"], "ok": ok}


def to_junit(parsed: dict, summary: dict) -> ET.ElementTree:
    suites = ET.Element("testsuites")
    suite = ET.SubElement(
        suites,
        "testsuite",
        name="device",
        tests=str(summary["total"]),
        failures=str(summary["failed"]),
        errors=str(summary["errors"]),
        skipped=str(summary["skipped"]),
    )
    for test in parsed["tests"]:
        case = ET.SubElement(suite, "testcase", classname=_clean(test["classname"]), name=_clean(test["name"]))
        if test["status"] == "failed":
            node = ET.SubElement(case, "failure", message=_clean(test.get("message") or test["stack"].splitlines()[0] if test["stack"] else ""))
            node.text = _clean(test["stack"])
        elif test["status"] == "error":
            node = ET.SubElement(case, "error", message=_clean(test.get("message", "")))
            node.text = _clean(test["stack"])
        elif test["status"] == "skipped":
            ET.SubElement(case, "skipped")
    out = ET.SubElement(suite, "system-out")
    out.text = _clean(parsed["stream"])
    return ET.ElementTree(suites)


def main(argv: list[str]) -> int:
    if len(argv) != 4:
        print(__doc__, file=sys.stderr)
        return 2
    raw, junit_path, summary_path = (Path(p) for p in argv[1:])
    parsed = parse(raw.read_text(errors="replace"))
    summary = summarize(parsed)
    tree = to_junit(parsed, summary)
    ET.indent(tree)
    tree.write(junit_path, encoding="utf-8", xml_declaration=True)
    summary_path.write_text(json.dumps(summary, indent=2) + "\n")

    print(f"[junit] {summary['passed']} passed, {summary['failed']} failed, {summary['errors']} errors, {summary['skipped']} skipped -> {junit_path}")
    for test in parsed["tests"]:
        if test["status"] in ("failed", "error"):
            head = (test["stack"] or test.get("message", "")).strip().splitlines()
            print(f"[junit]   {test['status'].upper()} {test['classname']}#{test['name']}: {head[0] if head else ''}")
    return 0 if summary["ok"] else 1


if __name__ == "__main__":
    sys.exit(main(sys.argv))
