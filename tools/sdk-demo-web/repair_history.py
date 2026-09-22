"""Retain genuine prior results during this explicitly same-key transition."""
from __future__ import annotations

import json
from pathlib import Path
import re

from catalog import CONSUMERS, REPO
from jobs import Job, MAX_RECEIPT_BYTES, TERMINAL
from runtime import sha

PREVIOUS = REPO / "target/sdk-demo-web-20260911-04"
PREVIOUS_PREFLIGHT_SHA = "1384eb6639d9425af67b3404877038b4ca5ee798ceb95e25507ac1ee8f3d9b31"
NAME = re.compile(r"job-(\d{3})-([0-9a-f]{32})\.json\Z")


def restore_previous_results(manager, previous: Path = PREVIOUS) -> dict:
    if sha(previous / "preflight.json") != PREVIOUS_PREFLIGHT_SHA:
        raise ValueError("Previous native preparation changed")
    old = json.loads((previous / "preflight.json").read_text())
    current = manager.runtime.provenance()
    # The ten unaffected SDKs must still use exactly their original execution pins.
    for language in CONSUMERS:
        if old["runtimes"][language] != current["runtimes"][language]:
            raise ValueError("The original native cohort changed")
    latest = {}
    submitted = 0
    for path in sorted((previous / "jobs").glob("job-*.json")):
        match = NAME.fullmatch(path.name)
        if match is None or path.stat().st_size > MAX_RECEIPT_BYTES:
            continue
        sequence = int(match[1])
        submitted = max(submitted, sequence)
        data = json.loads(path.read_text())
        if data.get("mode") != "live" or data.get("tokenRecorded") is not False:
            continue
        view = data.get("job", {})
        language = view.get("language")
        if language not in CONSUMERS or view.get("operation") != "key" or view.get("id") != match[2] or view.get("state") not in TERMINAL:
            continue
        latest[language] = (sequence, path, view)
    restored = []
    with manager._condition:
        if manager._submitted or manager._jobs:
            raise ValueError("Results can only be restored before the new server accepts jobs")
        for language, (sequence, path, view) in latest.items():
            state, result = view["state"], view.get("result")
            if not isinstance(result, dict):
                continue
            if state == "completed":
                # A replaced language must earn a NEW successful confirmation.
                if language in ("dart", "kotlin"):
                    continue
                if result.get("ok") is not True or result.get("httpStatus") != 200 or result.get("exitCode") != 0 or result.get("reason") is not None or any(result.get("outputTruncated", {}).values()):
                    continue
                confirmation = manager.runtime.runner.validate_response(json.dumps(result.get("confirmation")), "key")
                if confirmation.get("ok") is not True:
                    continue
                result = {**result, "kind": "confirmed-previous-session"}
            elif result.get("ok") is not False:
                continue
            result = {**result, "previousReceipt": {"path": str(path.relative_to(REPO)), "sha256": sha(path)}}
            job = Job(view["id"], language, "key", sequence, manager._credential_version,
                      created_at=view["createdAt"] / 1000, state=state)
            job.started_at = view["startedAt"] / 1000 if view.get("startedAt") else None
            job.finished_at = view["finishedAt"] / 1000 if view.get("finishedAt") else None
            job.elapsed_ms = view.get("elapsedMs", 0)
            job.result = result
            job.receipt_saved = True
            if state == "cancelled":
                job.cancel.set()
            manager._jobs[job.id] = job
            manager._latest[language] = job.id
            restored.append({"language": language, "state": state, "receiptSha256": result["previousReceipt"]["sha256"]})
        manager._submitted = submitted
    return {"previousRoot": str(previous.relative_to(REPO)), "sameKeyContinuityConfirmedByUser": True,
            "restored": restored, "newNativeExecutions": 0, "replacedLanguageSuccessesRestored": 0}
