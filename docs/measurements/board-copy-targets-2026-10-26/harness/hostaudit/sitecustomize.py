"""Host-side timing for ai-orchestrator's recipes, loaded through `PYTHONPATH`.

Inert unless `SPIKE_HOST_AUDIT` names a file. Then it appends one JSON line when the
interpreter starts and one when it exits, one per subprocess (a store `onetaskgraph` with its
command line, any other program by its name, started and reaped, with wall-clock times) and
one per call of each host step named in `STEPS` (its own wall time, nested calls included). It is the shape
of ai-orchestrator's `tests/github_board.py` `STORE_CALL_AUDIT`, widened to durations, and
it lives outside that checkout, so no tracked file there changes.
"""

from __future__ import annotations

import atexit
import functools
import importlib.abc
import importlib.machinery
import json
import os
import subprocess
import sys
import threading
import time

LOG = os.environ.get("SPIKE_HOST_AUDIT")

STEPS = {
    "orchestrator.plan_review": (
        "bar_fingerprint",
        "plan_bar_fingerprint",
        "checklist_fingerprint",
        "review_key",
        "plan_key",
        "unreviewed",
        "plan_unreviewed",
        "plan_answers",
        "owned_by_node",
    ),
    "orchestrator.plan_store": (
        "plan_listing",
        "read_projects",
        "read_tasks",
        "read_plan",
        "project_record",
    ),
    "orchestrator.task_body": (
        "document_bodies",
        "measure",
        "serve",
        "document_refusals",
    ),
    "orchestrator.plan_copy": ("_measure_documents", "_copy_options", "_report"),
    "orchestrator.design_chain": ("plan_repository", "chosen"),
    "orchestrator.spike_plan": ("current_visual_report",),
    "orchestrator.design_approval": (
        "approve",
        "one_document",
        "resolved_loader",
        "resolved_digest",
        "body_digest",
        "validate_content",
        "asset_digests",
        "visual_evidence",
        "planned_rows",
        "ungraphed",
        "approval_key",
    ),
}

_lock = threading.Lock()


def _emit(record: dict) -> None:
    if not LOG:
        return
    with _lock, open(LOG, "a") as out:
        out.write(json.dumps(record, sort_keys=True) + "\n")


def _timed(module: str, name: str, function):
    @functools.wraps(function)
    def wrapper(*args, **kwargs):
        start = time.time()
        try:
            return function(*args, **kwargs)
        finally:
            _emit(
                {
                    "kind": "step",
                    "name": f"{module.rsplit('.', 1)[-1]}.{name}",
                    "start": start,
                    "end": time.time(),
                    "pid": os.getpid(),
                }
            )

    return wrapper


def _record_store(argv, start, code) -> None:
    program, words = argv
    record = {
        "kind": "store" if program == "onetaskgraph" else "subprocess",
        "start": start,
        "end": time.time(),
        "returncode": code,
        "pid": os.getpid(),
    }
    if program == "onetaskgraph":
        record["argv"] = words
    else:
        record["program"] = program
    _emit(record)


def _patch_asyncio(module) -> None:
    """The plan store's SDK starts the CLI with `asyncio.create_subprocess_exec`."""
    original = module.create_subprocess_exec

    async def create_subprocess_exec(program, *args, **kwargs):
        start = time.time()
        process = await original(program, *args, **kwargs)
        argv = _store_argv([program, *args])
        if argv is None:
            return process
        done = []
        wait = process.wait

        async def timed_wait():
            code = await wait()
            if not done:
                done.append(True)
                _record_store(argv, start, code)
            return code

        process.wait = timed_wait
        return process

    module.create_subprocess_exec = create_subprocess_exec


class _Finder(importlib.abc.MetaPathFinder):
    def find_spec(self, fullname, path, target=None):
        if fullname == "asyncio" and not getattr(self, "_asyncio", False):
            self._asyncio = True
            spec = importlib.machinery.PathFinder.find_spec(fullname, path)
            if spec is None or spec.loader is None:
                return spec
            original = spec.loader.exec_module

            def exec_asyncio(module):
                original(module)
                _patch_asyncio(module)

            spec.loader.exec_module = exec_asyncio
            return spec
        if fullname not in STEPS:
            return None
        spec = importlib.machinery.PathFinder.find_spec(fullname, path)
        if spec is None or spec.loader is None:
            return spec
        original = spec.loader.exec_module

        def exec_module(module):
            original(module)
            for name in STEPS[fullname]:
                function = module.__dict__.get(name)
                if callable(function):
                    module.__dict__[name] = _timed(fullname, name, function)

        spec.loader.exec_module = exec_module
        return spec


def _store_argv(args) -> tuple[str, list[str]] | None:
    """Every subprocess, as its program's name and (for the store alone) its arguments."""
    argv = args if isinstance(args, (list, tuple)) else [args]
    if not argv:
        return None
    return os.path.basename(str(argv[0])), [str(a) for a in argv[1:]]


if LOG:
    _emit(
        {
            "kind": "interpreter",
            "name": "start",
            "start": time.time(),
            "pid": os.getpid(),
        }
    )
    atexit.register(
        lambda: _emit(
            {
                "kind": "interpreter",
                "name": "exit",
                "start": time.time(),
                "pid": os.getpid(),
            }
        )
    )
    sys.meta_path.insert(0, _Finder())
    _init = subprocess.Popen.__init__
    _wait = subprocess.Popen.wait

    def _popen_init(self, args, *rest, **kwargs):
        self._spike_store = _store_argv(args)
        self._spike_start = time.time()
        _init(self, args, *rest, **kwargs)

    def _popen_wait(self, *args, **kwargs):
        code = _wait(self, *args, **kwargs)
        argv = getattr(self, "_spike_store", None)
        if argv is not None and not getattr(self, "_spike_done", False):
            self._spike_done = True
            _record_store(argv, self._spike_start, code)
        return code

    subprocess.Popen.__init__ = _popen_init
    subprocess.Popen.wait = _popen_wait
