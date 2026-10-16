#!/usr/bin/env bash
# Time one of this repository's own recipes on this host, with every Nx task's start and end.
#
# Usage: spike/onetaskgraph-ci/local-profile.sh <out-dir> <nx-home> <recipe...>
#   e.g. local-profile.sh runs/gate-cold /tmp/nxhome just gate
#
# <nx-home> holds the Nx cache and workspace data this run reads and writes, so an empty one
# is a cold run and the one a previous identical run left is a warm one; the shared cache of
# the pool's clone is never touched. Every credential and nomination a live lane reads is
# unset, so no live tier can run. Writes <out-dir>/output.log, <out-dir>/wall.json and
# <out-dir>/tasks.json (every Nx task this run recorded: project, target, status, seconds).
set -uo pipefail

out="${1:?out-dir}"
nxhome="${2:?nx-home}"
shift 2
mkdir -p "$out" "$nxhome"
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

export NX_CACHE_DIRECTORY="$nxhome/cache"
export NX_WORKSPACE_DATA_DIRECTORY="$nxhome/workspace-data"
export NX_DAEMON=false
unset GH_PROJECTS_TOKEN LINEAR_API_KEY ONETASKGRAPH_LIVE_REQUIRED GH_PROJECTS_OWNER \
  GH_PROJECTS_NUMBER GH_PROJECTS_REPOSITORY LINEAR_WRITE_TEAM GITHUB_LIVE_SESSION_ID \
  GITHUB_LIVE_SESSION_DIR

cache_entries_before="$(find "$NX_CACHE_DIRECTORY" -maxdepth 1 2>/dev/null | wc -l | tr -d ' ')"
start_ms="$(python3 -c 'import time; print(int(time.time()*1000))')"
"$@" >"$out/output.log" 2>&1
code=$?
end_ms="$(python3 -c 'import time; print(int(time.time()*1000))')"

python3 - "$out" "$NX_WORKSPACE_DATA_DIRECTORY" "$start_ms" "$end_ms" "$code" "$cache_entries_before" "$*" <<'EOF'
import glob, json, sqlite3, sys
out, data, start, end, code, before, command = sys.argv[1:]
start, end = int(start), int(end)
tasks = []
for db in glob.glob(f"{data}/*.db"):
    con = sqlite3.connect(db)
    rows = con.execute(
        "select d.project, d.target, h.status, h.code, h.start, h.end from task_history h "
        "join task_details d using(hash) where h.start >= ? and h.end <= ? order by h.start", (start, end))
    for project, target, status, rc, a, b in rows:
        tasks.append({"task": f"{project}:{target}", "status": status, "code": rc,
                      "start_offset_s": round((a - start) / 1000, 1), "seconds": round((b - a) / 1000, 1)})
json.dump({"command": command, "exit": int(code), "wall_s": round((end - start) / 1000, 1),
           "nx_cache_entries_before": int(before)}, open(f"{out}/wall.json", "w"), indent=1)
json.dump(tasks, open(f"{out}/tasks.json", "w"), indent=1)
print(f"{command}: exit {code}, {round((end - start) / 1000)} s, {len(tasks)} Nx tasks")
EOF
exit "$code"
