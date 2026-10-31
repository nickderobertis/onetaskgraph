# Sourced by every bash a job starts while BASH_ENV names this file: scripts/ci-job.sh sets
# it on test-windows, where scripts:test is the job's last task, so it can say how long each
# guard took. A guard scripts:test starts (Nx names the task each command runs for) appends
# one line to ONETASKGRAPH_SCRIPT_TIMINGS_LOG, the time and its path, and marks its
# environment so the guards it runs in turn append nothing. It also says so on standard error,
# which the job's log timestamps as it streams, so a job the timeout cancels before ci-job.sh's
# closing summary still shows when each guard began. Nothing is trapped, so no guard's own EXIT
# trap is replaced; a guard's duration is read as the gap to the next one's start.
if [ -n "${ONETASKGRAPH_SCRIPT_TIMINGS_LOG:-}" ] && [ -z "${ONETASKGRAPH_SCRIPT_TIMINGS_INSIDE:-}" ] \
  && [ "${NX_TASK_TARGET_PROJECT:-}:${NX_TASK_TARGET_TARGET:-}" = scripts:test ]; then
  case "$0" in
    scripts/check-*.sh | */scripts/check-*.sh)
      echo "script-start-log: $0 started" >&2
      if ! printf '%s %s\n' "$(date +%s)" "$0" >>"$ONETASKGRAPH_SCRIPT_TIMINGS_LOG"; then
        echo "script-start-log: could not record $0's start in $ONETASKGRAPH_SCRIPT_TIMINGS_LOG, so the guard timings will leave it out; next: check that its directory exists and is writable. The guard itself still runs." >&2
      fi
      export ONETASKGRAPH_SCRIPT_TIMINGS_INSIDE=1
      ;;
  esac
fi
