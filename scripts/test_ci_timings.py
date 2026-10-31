"""Drive the timing recorders and their summaries the way test-windows does, on real files."""
from pathlib import Path
import importlib.util
import os
import shutil
import subprocess
import sys
import tempfile
import time
import unittest


SCRIPTS = Path(__file__).resolve().parent
PYTHON_EXECUTABLE = str(Path(sys.executable).resolve())


def summarise(*arguments: str) -> subprocess.CompletedProcess:
    return subprocess.run(
        [PYTHON_EXECUTABLE, str(SCRIPTS / 'ci_timings.py'), *arguments],
        capture_output=True, text=True, encoding='utf-8', check=False,
    )


class CiTimingsTest(unittest.TestCase):
    def setUp(self):
        self.scratch = Path(tempfile.mkdtemp(prefix='ci-timings-'))
        self.addCleanup(shutil.rmtree, self.scratch, True)

    def test_a_trace_names_its_slowest_lines_cases_and_ranges(self):
        trace = self.scratch / 'journey.trace'
        trace.write_text(
            '+10.0 test-distribution.sh:146: cd /work\n'
            'not a trace line\n'
            '++40.0 test-distribution.sh:458: python3 scripts/test-release-products.py\n'
            '+45.0 scratch-clone.sh:46: git checkout\n'
            '+46.0 test-distribution.sh:470: done\n',
            encoding='utf-8',
        )
        packaging = self.scratch / 'products.timings'
        packaging.write_text('12.0\tfirst install\n3.0\trefusal\n8.0\tfirst install\n', encoding='utf-8')
        result = summarise('trace', str(trace), str(packaging))
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(
            result.stdout,
            'distribution timing: slowest lines: 30s test-distribution.sh:146 cd /work; '
            '5s test-distribution.sh:458 python3 scripts/test-release-products.py; '
            '1s scratch-clone.sh:46 git checkout | slowest packaging cases: 20s first install; '
            '3s refusal | by line range: 100-199 30s; 400-499 5s\n',
        )

    def test_a_trace_without_packaging_records_still_summarises(self):
        trace = self.scratch / 'journey.trace'
        trace.write_text('+1.0 test-distribution.sh:5: a\n+3.0 test-distribution.sh:6: b\n', encoding='utf-8')
        result = summarise('trace', str(trace), str(self.scratch / 'absent'))
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn('slowest lines: 2s test-distribution.sh:5 a | slowest packaging cases:  |', result.stdout)

    def test_an_unreadable_recording_is_named_with_what_to_do(self):
        result = summarise('trace', str(self.scratch / 'absent'), str(self.scratch / 'absent'))
        self.assertEqual(result.returncode, 1)
        self.assertEqual(result.stdout, '')
        self.assertIn(f"could not summarise {self.scratch / 'absent'}", result.stderr)
        self.assertIn('next:', result.stderr)
        malformed = self.scratch / 'products.timings'
        malformed.write_text('twelve\tfirst install\n', encoding='utf-8')
        trace = self.scratch / 'journey.trace'
        trace.write_text('+1.0 test-distribution.sh:5: a\n', encoding='utf-8')
        result = summarise('trace', str(trace), str(malformed))
        self.assertEqual(result.returncode, 1)
        self.assertIn('ValueError', result.stderr)
        self.assertEqual(summarise('guards').returncode, 2)

    def test_guards_are_timed_start_to_start_and_the_last_to_the_end(self):
        starts = self.scratch / 'script-timings'
        starts.write_text(
            '100 scripts/check-a.sh\n130 /work/scripts/check-b.sh\n131 scripts/check-c.sh\n',
            encoding='utf-8',
        )
        result = summarise('guards', str(starts), '200')
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(
            result.stdout,
            "scripts:test guards, each to the next one's start: "
            '30s check-a.sh; 1s check-b.sh; 69s check-c.sh\n',
        )
        self.assertEqual(summarise('guards', str(starts), 'later').returncode, 1)

    def test_the_start_log_records_only_the_guards_scripts_test_starts(self):
        bash = shutil.which('bash')
        if bash is None:
            self.skipTest('no bash on PATH')
        work = self.scratch / 'work'
        (work / 'scripts').mkdir(parents=True)
        (work / 'scripts' / 'check-outer.sh').write_text('bash scripts/check-inner.sh\n', encoding='utf-8')
        (work / 'scripts' / 'check-inner.sh').write_text('echo inner\n', encoding='utf-8')
        (work / 'scripts' / 'other.sh').write_text('echo other\n', encoding='utf-8')
        log = self.scratch / 'starts'

        def run(script, project, target='test'):
            environment = dict(os.environ)
            environment.pop('ONETASKGRAPH_SCRIPT_TIMINGS_INSIDE', None)
            environment.update({
                'BASH_ENV': (SCRIPTS / 'script-start-log.sh').as_posix(),
                'ONETASKGRAPH_SCRIPT_TIMINGS_LOG': log.as_posix(),
                'NX_TASK_TARGET_PROJECT': project,
                'NX_TASK_TARGET_TARGET': target,
            })
            subprocess.run([bash, script], cwd=work, env=environment, check=True, capture_output=True)

        before = int(time.time())
        run('scripts/check-outer.sh', 'scripts')
        run('scripts/check-inner.sh', 'onetaskgraph-github-projects')
        run('scripts/check-inner.sh', 'scripts', target='lint')
        run('scripts/other.sh', 'scripts')
        lines = log.read_text(encoding='utf-8').splitlines()
        self.assertEqual(len(lines), 1, lines)
        moment, path = lines[0].split(' ', 1)
        self.assertEqual(path, 'scripts/check-outer.sh')
        self.assertGreaterEqual(int(moment), before)

    def test_the_packaging_recorder_writes_a_label_and_its_seconds(self):
        records = self.scratch / 'products.timings'
        previous = os.environ.get('ONETASKGRAPH_PRODUCTS_TIMINGS_FILE')
        os.environ['ONETASKGRAPH_PRODUCTS_TIMINGS_FILE'] = str(records)
        try:
            specification = importlib.util.spec_from_file_location(
                'test_release_products', SCRIPTS / 'test-release-products.py')
            module = importlib.util.module_from_spec(specification)
            specification.loader.exec_module(module)
            module.timed('first install', time.monotonic() - 1.5)
        finally:
            if previous is None:
                os.environ.pop('ONETASKGRAPH_PRODUCTS_TIMINGS_FILE')
            else:
                os.environ['ONETASKGRAPH_PRODUCTS_TIMINGS_FILE'] = previous
        seconds, label = records.read_text(encoding='utf-8').rstrip('\n').split('\t')
        self.assertEqual(label, 'first install')
        self.assertGreaterEqual(float(seconds), 1.5)


if __name__ == '__main__':
    unittest.main()
