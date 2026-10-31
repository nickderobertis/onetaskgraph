# Sourced by every bash a job starts while BASH_ENV names this file: scripts/ci-job.sh sets
# it on test-windows, where scripts:test is the job's last task, so it can say how long each
# guard took. A guard scripts:test starts (Nx names the task each command runs for) appends
# one line to ONETASKGRAPH_SCRIPT_TIMINGS_LOG, the time and its path, and marks its
# environment so the guards it runs in turn append nothing.
# Nothing is trapped, so no guard's own EXIT trap is replaced; a guard's duration is read as
# the gap to the next one's start.
if [ -n "${ONETASKGRAPH_SCRIPT_TIMINGS_LOG:-}" ] && [ -z "${ONETASKGRAPH_SCRIPT_TIMINGS_INSIDE:-}" ] \
  && [ "${NX_TASK_TARGET_PROJECT:-}:${NX_TASK_TARGET_TARGET:-}" = scripts:test ]; then
  case "$0" in
    scripts/check-*.sh | */scripts/check-*.sh)
      printf '%s %s\n' "$(date +%s)" "$0" >>"$ONETASKGRAPH_SCRIPT_TIMINGS_LOG"
      export ONETASKGRAPH_SCRIPT_TIMINGS_INSIDE=1
      ;;
  esac
fi
