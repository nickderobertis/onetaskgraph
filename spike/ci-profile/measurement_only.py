#!/usr/bin/env python3
"""Apply the spike's measurement-only changes to a tree's `.github/workflows/ci.yml`.

    python3 spike/ci-profile/measurement_only.py <ci.yml> [--exclude=<projects>]

The same text edits apply to today's workflow and to the workflow of #3956 and its parent,
so both sides of an A/B carry byte-identical measurement changes:
- no cache write of any kind: every uv `save-cache` is false, setup-bun keeps no cache, every
  Rust `cache-save-if` is false, every `actions/cache/save` step is `if: false`, the Nx cache
  sidecar is read-only, and the Nx report is uploaded under a name no promotion reads;
- `lint` and `budgets` do not run (they measure nothing this spike reads);
- each test job's test step runs inside the process sampler, every libtest binary runs under
  the recording runner, and the records are uploaded as `spike-profile-<job>`;
- `--exclude` passes projects to `ci-job.sh --exclude=` (the tasks only the workflow edit
  itself selected), so the demonstration runs the selection its source edit makes.
It exits non-zero, naming the edit, when an expected site is missing.
"""

from __future__ import annotations

import re
import sys

TRIPLES = {"test-linux": "X86_64_UNKNOWN_LINUX_GNU", "test-macos": "AARCH64_APPLE_DARWIN",
           "test-windows": "X86_64_PC_WINDOWS_MSVC"}


def sub(pattern: str, replacement: str, text: str, what: str, minimum: int = 1, flags: int = 0) -> str:
    new, n = re.subn(pattern, replacement, text, flags=flags)
    if n < minimum:
        raise SystemExit(f"measurement_only: expected at least {minimum} site(s) for {what}, found {n}")
    return new


def recorder_steps(job: str) -> str:
    py = "python" if job == "test-windows" else "python3"
    venv_py = "$RUNNER_TEMP/spikevenv/Scripts/python.exe" if job == "test-windows" else "$RUNNER_TEMP/spikevenv/bin/python"
    return f"""      - name: "spike: install the process sampler"
        shell: bash
        run: |
          {py} -m venv "$RUNNER_TEMP/spikevenv"
          "{venv_py}" -m pip install --quiet psutil==7.1.3
          mkdir -p "$RUNNER_TEMP/spike"
          echo "SPIKE_PY={venv_py}" >> "$GITHUB_ENV"
          echo "SPIKE_TEST_RECORD=$RUNNER_TEMP/spike/tests.jsonl" >> "$GITHUB_ENV"
          echo "CARGO_TARGET_{TRIPLES[job]}_RUNNER={py} $GITHUB_WORKSPACE/spike/ci-profile/testrunner.py" >> "$GITHUB_ENV"
"""


def upload_step(job: str) -> str:
    return f"""      - name: "spike: upload the profile records"
        if: always()
        uses: actions/upload-artifact@v7
        with:
          name: spike-profile-{job}
          path: ${{{{ runner.temp }}}}/spike/
          if-no-files-found: warn
          retention-days: 14
"""


def apply(text: str, exclude: str) -> str:
    text = sub(r"save-cache: \$\{\{[^}]*\}\}", "save-cache: false", text, "uv save-cache", 4)
    text = sub(r"no-cache: \$\{\{[^}]*\}\}", "no-cache: true", text, "setup-bun no-cache", 4)
    text = sub(r"cache-save-if: [^\n]+", "cache-save-if: false", text, "rust cache-save-if", 4)
    text = sub(r"(- uses: actions/cache/save@v\d+\n\s+)if: [^\n]+", r"\1if: false", text, "actions/cache/save", 2)
    text = sub(r"CACHE_READ_ONLY: [^\n]+", "CACHE_READ_ONLY: '1'", text, "Nx sidecar read-only", 4)
    text = sub(r"name: nx-cache-entries-\$\{\{ github\.job \}\}", "name: spike-nx-executions-${{ github.job }}", text,
               "Nx report artifact name", 4)
    text = sub(r"(\n  lint:\n    )if: [^\n]+", r"\1if: false", text, "lint off")
    text = sub(r"(\n  budgets:\n    needs: [^\n]*\n    )if: [^\n]+", r"\1if: false", text, "budgets off")
    for job in TRIPLES:
        block = re.search(rf"\n  {job}:\n(?:    [^\n]*\n|\n)+", text)
        if not block:
            raise SystemExit(f"measurement_only: no {job} job")
        body = block.group(0)
        command = f"just ci-{job}" + (f" --exclude={exclude}" if exclude else "")
        wrapped = (f"        shell: bash\n        run: |\n"
                   f"          \"$SPIKE_PY\" spike/ci-profile/sampler.py \"$RUNNER_TEMP/spike/samples.jsonl\" &\n"
                   f"          sampler=$!\n"
                   f"          status=0\n"
                   f"          {command} || status=$?\n"
                   f"          touch \"$RUNNER_TEMP/spike/samples.jsonl.stop\"\n"
                   f"          wait \"$sampler\" || true\n"
                   f"          exit \"$status\"\n")
        body = sub(rf"        run: just ci-{job}\n", wrapped, body, f"{job} test step")
        body = sub(r"(\n      - name: Test the affected projects\n)", "\n" + recorder_steps(job).rstrip("\n") + r"\1",
                   body, f"{job} recorder install")
        body = sub(r"(\n      - name: Report cache entries and task executions\n)", "\n" + upload_step(job).rstrip("\n") + r"\1",
                   body, f"{job} upload")
        text = text.replace(block.group(0), body)
    return text


def main() -> int:
    path = sys.argv[1]
    exclude = next((a.split("=", 1)[1] for a in sys.argv[2:] if a.startswith("--exclude=")), "")
    with open(path, encoding="utf-8") as handle:
        text = handle.read()
    with open(path, "w", encoding="utf-8", newline="\n") as handle:
        handle.write(apply(text, exclude))
    return 0


if __name__ == "__main__":
    sys.exit(main())
