"""report-job:budgets: judge what report-job:test recorded, with the released reader. Reruns nothing."""
import os
from pathlib import Path
import platform
import subprocess
import sys
import tarfile

ROOT = Path(__file__).resolve().parents[2]


def main():
    if platform.system() != 'Linux':
        print(f'report-job budgets: skipped on {platform.system()}: the job and its recording are Linux only')
        return 0
    sys.path.insert(0, str(Path(__file__).resolve().parent))
    from test_report_job import STATE, released_action
    _, tree, _ = released_action()
    tools = STATE / 'reader'
    tools.mkdir(parents=True, exist_ok=True)
    with tarfile.open(next((tree / 'dist').glob('*.tar.gz'))) as archive:
        archive.extractall(tools, filter='data')
    path = os.pathsep.join([str(tools), str(ROOT / 'node_modules/.bin'), os.environ['PATH']])
    return subprocess.run(['onebudgetspec', 'check', '.github/reporting/budgets.yaml',
                           '--save-results', 'target/budget-results/report-job-budget-results.json'], cwd=ROOT,
                          env={**os.environ, 'PATH': path}, check=False).returncode


if __name__ == '__main__':
    sys.exit(main())
