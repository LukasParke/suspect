"""A small bounded queue. Credentials exist only in manager/worker memory."""
from __future__ import annotations

import json
import os
import threading
import time
import uuid
from collections import OrderedDict, deque
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any

from catalog import CONSUMERS, LANGUAGES, OPERATIONS
from runtime import failed_result

MAX_CONCURRENT = 3
HISTORY_LIMIT = 64
SESSION_LIMIT = 256
MAX_RECEIPT_BYTES = 24_576
TERMINAL = frozenset(("completed", "failed", "cancelled"))


class RequestError(Exception):
    def __init__(self, status: int, code: str, message: str) -> None:
        self.status, self.code, self.message = status, code, message
        super().__init__(message)


def valid_token(token: Any) -> bool:
    return isinstance(token, str) and 8 <= len(token) <= 4096 and all(33 <= ord(char) <= 126 for char in token)


@dataclass
class Job:
    id: str
    language: str
    operation: str
    sequence: int
    credential_version: int
    created_at: float = field(default_factory=time.time)
    state: str = "queued"
    started_at: float | None = None
    finished_at: float | None = None
    started_clock: float | None = None
    elapsed_ms: int = 0
    cancel: threading.Event = field(default_factory=threading.Event, repr=False)
    result: dict | None = None
    receipt_saved: bool | None = None

    def view(self, position: int | None = None) -> dict:
        elapsed = self.elapsed_ms
        if self.state == "running" and self.started_clock is not None:
            elapsed = round((time.monotonic() - self.started_clock) * 1000)
        return {
            "id": self.id, "language": self.language, "operation": self.operation,
            "state": self.state, "createdAt": round(self.created_at * 1000),
            "startedAt": round(self.started_at * 1000) if self.started_at else None,
            "finishedAt": round(self.finished_at * 1000) if self.finished_at else None,
            "elapsedMs": elapsed, "queuePosition": position,
            "cancelRequested": self.cancel.is_set(), "result": self.result,
            "receiptSaved": self.receipt_saved,
        }


class JobManager:
    def __init__(self, runtime: Any, receipt_root: Path, token: str = "", *,
                 concurrency: int = MAX_CONCURRENT, history_limit: int = HISTORY_LIMIT,
                 session_limit: int = SESSION_LIMIT) -> None:
        if not 1 <= concurrency <= MAX_CONCURRENT or history_limit < len(CONSUMERS):
            raise ValueError("Invalid local queue limits")
        self.runtime = runtime
        self.cards = runtime.cards()
        self._ready = {card["id"] for card in self.cards if card["ready"]}
        self._condition = threading.Condition(threading.RLock())
        self._jobs: OrderedDict[str, Job] = OrderedDict()
        self._latest: dict[str, str] = {}
        self._pending: deque[str] = deque()
        self._token = token if valid_token(token) else ""
        self._credential_version = 0
        self._closed = False
        self._submitted = 0
        self._history_limit = history_limit
        self._session_limit = session_limit
        self._receipt_root = receipt_root
        self._receipt_root.mkdir(parents=True, exist_ok=True, mode=0o700)
        self._receipt_error = False
        self.concurrency = concurrency
        self._workers = [threading.Thread(target=self._work, name=f"sdk-demo-job-{index}", daemon=True) for index in range(concurrency)]
        for worker in self._workers:
            worker.start()

    def _credential(self) -> dict:
        return {"ready": bool(self._token)}

    def set_token(self, token: Any) -> dict:
        if token != "" and not valid_token(token):
            raise RequestError(400, "invalid-key", "Enter an API key with 8–4096 visible characters and no whitespace.")
        with self._condition:
            if self._closed:
                raise RequestError(503, "shutting-down", "The local server is shutting down.")
            self._cancel_all()
            self._credential_version += 1
            self._token = token
            self._condition.notify_all()
            return self._credential()

    def start(self, language: Any, operation: Any) -> dict:
        if not isinstance(language, str) or language not in (*CONSUMERS, "all"):
            raise RequestError(400, "unsupported-language", "Choose one of the prepared SDK languages.")
        if not isinstance(operation, str) or operation not in OPERATIONS:
            raise RequestError(400, "unsupported-operation", "This page runs the read-only current-key operation.")
        selected = LANGUAGES if language == "all" else (language,)
        with self._condition:
            if self._closed:
                raise RequestError(503, "shutting-down", "The local server is shutting down.")
            if not self._token:
                raise RequestError(409, "key-required", "Add your OpenRouter API key before running a live request.")
            if any(item not in self._ready for item in selected):
                raise RequestError(409, "not-prepared", "A selected native SDK did not pass local preflight. See the setup guide.")
            new = [item for item in selected if item not in self._latest or self._jobs[self._latest[item]].state in TERMINAL]
            if self._submitted + len(new) > self._session_limit:
                raise RequestError(429, "session-limit", "This session reached its request limit. Restart ./demo-web.sh for a new session.")
            for item in new:
                self._submitted += 1
                job = Job(uuid.uuid4().hex, item, operation, self._submitted, self._credential_version)
                self._jobs[job.id] = job
                self._latest[item] = job.id
                self._pending.append(job.id)
            self._trim_history()
            self._condition.notify_all()
            return {"accepted": len(new), "jobIds": [self._latest[item] for item in selected]}

    def cancel(self, job_id: str | None = None) -> None:
        with self._condition:
            if job_id is None:
                self._cancel_all()
            elif job_id in self._jobs:
                self._cancel(self._jobs[job_id])
            else:
                raise RequestError(404, "job-not-found", "That job is no longer in this session's history.")
            self._condition.notify_all()

    def _cancel(self, job: Job) -> None:
        if job.state in TERMINAL:
            return
        job.cancel.set()
        if job.state == "queued":
            self._finish(job, failed_result("cancelled"))

    def _cancel_all(self) -> None:
        for job in self._jobs.values():
            self._cancel(job)
        self._pending.clear()

    def _trim_history(self) -> None:
        protected = set(self._latest.values())
        for job_id in list(self._jobs):
            if len(self._jobs) <= self._history_limit:
                break
            if job_id not in protected and self._jobs[job_id].state in TERMINAL:
                del self._jobs[job_id]
        self._pending = deque(job_id for job_id in self._pending if job_id in self._jobs and self._jobs[job_id].state == "queued")

    def _save_receipt(self, job: Job) -> bool:
        receipt = {"mode": self.runtime.mode, "tokenRecorded": False, "job": job.view()}
        encoded = (json.dumps(receipt, ensure_ascii=True, indent=2) + "\n").encode()
        if len(encoded) > MAX_RECEIPT_BYTES:
            return False
        path = self._receipt_root / f"job-{job.sequence:03}-{job.id}.json"
        try:
            fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
            with os.fdopen(fd, "wb") as stream:
                stream.write(encoded)
            return True
        except OSError:
            return False

    def _finish(self, job: Job, result: dict) -> None:
        if job.cancel.is_set():
            job.state = "cancelled"
            result = failed_result("cancelled")
        else:
            job.state = "completed" if result["ok"] else "failed"
        job.result = result
        job.finished_at = time.time()
        if job.started_clock is not None:
            job.elapsed_ms = round((time.monotonic() - job.started_clock) * 1000)
        job.receipt_saved = self._save_receipt(job)
        self._receipt_error |= not job.receipt_saved

    def _work(self) -> None:
        while True:
            with self._condition:
                self._condition.wait_for(lambda: self._pending or self._closed)
                if self._closed:
                    return
                job_id = self._pending.popleft()
                job = self._jobs.get(job_id)
                if job is None or job.state != "queued":
                    continue
                if job.credential_version != self._credential_version or not self._token:
                    self._cancel(job)
                    continue
                token = self._token
                job.state = "running"
                job.started_at, job.started_clock = time.time(), time.monotonic()
            try:
                result = self.runtime.execute(job.language, job.operation, token, job.cancel)
            except Exception:
                # Exceptions are intentionally not rendered/logged. Native scalar
                # diagnostics pass through the accepted sanitizer instead.
                result = failed_result("execution-failed")
            finally:
                token = ""
            with self._condition:
                self._finish(job, result)
                self._condition.notify_all()

    def snapshot(self) -> dict:
        with self._condition:
            queued = [job_id for job_id in self._pending if self._jobs.get(job_id) and self._jobs[job_id].state == "queued"]
            positions = {job_id: position + 1 for position, job_id in enumerate(queued)}
            latest = [self._jobs[self._latest[language]].view(positions.get(self._latest[language])) for language in CONSUMERS if language in self._latest]
            counts = {state: sum(job["state"] == state and job["language"] in LANGUAGES for job in latest) for state in ("queued", "running", "completed", "failed", "cancelled")}
            return {"jobs": latest, "counts": counts, "credential": self._credential(),
                    "submitted": self._submitted, "historyCount": len(self._jobs),
                    "receiptError": self._receipt_error, "closing": self._closed,
                    "serverTime": round(time.time() * 1000)}

    def get(self, job_id: str) -> dict:
        with self._condition:
            if job_id not in self._jobs:
                raise RequestError(404, "job-not-found", "That job is no longer in this session's history.")
            return self._jobs[job_id].view()

    def close(self) -> None:
        with self._condition:
            self._closed = True
            self._token = ""
            self._credential_version += 1
            self._cancel_all()
            self._condition.notify_all()
        for worker in self._workers:
            worker.join(timeout=27)
