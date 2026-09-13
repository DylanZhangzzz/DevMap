import copy
import importlib.util
from pathlib import Path
import subprocess
import sys
import unittest

spec = importlib.util.spec_from_file_location('roles', Path(__file__).with_name('windows-job-roles.py'))
roles = importlib.util.module_from_spec(spec); spec.loader.exec_module(roles)


class RoleTests(unittest.TestCase):
    def fixture(self):
        source = str(Path(__file__).parent.resolve())
        worker = {'executable': sys.executable, 'source': source,
                  'cohorts': [{'pid': p} for p in range(1, 5)], 'cold': {'samples': []}}
        processes = [{'pid': p, 'image': sys.executable, 'created_filetime': str(p), 'rss_bytes': 10,
                      'command_line': subprocess.list2cmdline([sys.executable, 'mcp', '--source', source])}
                     for p in range(1, 5)]
        job = {'root_exit_code': 0, 'empty_confirmed': True, 'aborted': False, 'root_pid': 99,
               'resources': {'samples': [{'elapsed_seconds': 1, 'complete_snapshot': True, 'processes': processes}]}}
        return job, worker

    def test_valid_proxy_commands(self):
        job, worker = self.fixture()
        self.assertEqual(roles.analyze(job, worker)['max_simultaneous_owner_commands'], 0)

    def test_missing_or_mismatched_command_cannot_be_classified(self):
        job, worker = self.fixture()
        for command in [None, subprocess.list2cmdline([sys.executable, 'runtime', '--owner', '--source', worker['source']]),
                        subprocess.list2cmdline([sys.executable, 'mcp', '--source', str(Path(worker['source']).parent)])]:
            changed = copy.deepcopy(job)
            changed['resources']['samples'][0]['processes'][0]['command_line'] = command
            with self.assertRaises(AssertionError):
                roles.analyze(changed, worker)


if __name__ == '__main__':
    unittest.main()
