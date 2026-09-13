"""Real Windows Job controls; no user repositories or PID-based cleanup."""
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

WRAPPER = Path(__file__).with_name("windows-owned-generator-job.py").resolve()


@unittest.skipUnless(os.name == "nt", "Windows Job integration")
class OwnedJobTests(unittest.TestCase):
    def run_case(self, *, descendant=False, code=0, planned=False, abort=False, resources=False):
        with tempfile.TemporaryDirectory(prefix="devmap-job-policy-") as directory:
            root = Path(directory)
            worker = root / "worker.py"
            worker.write_text(
                "import subprocess, sys, time\n"
                + ("subprocess.Popen([sys.executable, '-c', 'import time; time.sleep(60)'])\n"
                   if descendant else "")
                + ("time.sleep(60)\n" if abort else "")
                + ("time.sleep(.7)\n" if resources else "")
                + f"sys.exit({code})\n", encoding="utf8")
            report_path = root / "report.json"
            args = [sys.executable, str(WRAPPER), "--report", str(report_path),
                    "--exe", sys.executable]
            if planned:
                args.append("--teardown-descendants-after-success")
            if resources:
                args += ['--resource-interval', '.1']
            args += ["--", str(worker)]
            with (root / "output.log").open("wb") as output:
                child = subprocess.Popen(args, stdin=subprocess.PIPE, stdout=output,
                                         stderr=output, creationflags=subprocess.CREATE_NO_WINDOW)
                try:
                    if abort:
                        child.stdin.close()
                    status = child.wait(timeout=20)
                finally:
                    if child.poll() is None:
                        child.stdin.close()  # Request cleanup of this owned Job.
                        try:
                            child.wait(timeout=10)
                        except subprocess.TimeoutExpired:
                            child.kill()  # Retained wrapper handle; OS closes its Job.
                            child.wait(timeout=5)
                    if not child.stdin.closed:
                        child.stdin.close()
                report = json.loads(report_path.read_text(encoding="utf8"))
                self.assertTrue(report["empty_confirmed"], report)
                self.assertNotIn("cleanup_error", report)
                return status, report

    def test_default_clean_root_succeeds(self):
        status, report = self.run_case()
        self.assertEqual(status, 0, report)
        self.assertNotIn("cleanup_policy", report)

    def test_resource_sampling_includes_owned_descendant_and_exact_sums(self):
        status, report = self.run_case(descendant=True, planned=True, resources=True)
        self.assertEqual(status, 0, report)
        rows = report['resources']['samples']
        self.assertIn(report['root_pid'], [p['pid'] for p in rows[0]['processes']])
        self.assertTrue(any(len(r['processes']) >= 2 for r in rows))
        root_rows = [p for r in rows for p in r['processes'] if p['pid'] == report['root_pid']]
        self.assertTrue(any('worker.py' in (p.get('command_line') or '') for p in root_rows))
        for row in rows:
            self.assertEqual(row['membership_count'], len(row['processes']) + len(row['unobserved']))
            self.assertEqual(row['sum_rss_bytes'], sum(p['rss_bytes'] for p in row['processes']))
            self.assertEqual(row['sum_private_bytes'], sum(p['private_bytes'] for p in row['processes']))
        self.assertFalse(report['natural_lifecycle_acceptance'])

    def test_default_surviving_descendant_is_failure(self):
        status, report = self.run_case(descendant=True)
        self.assertNotEqual(status, 0, report)
        self.assertTrue(report["descendants_after_root_exit"])

    def test_planned_teardown_is_explicit_and_not_lifecycle_acceptance(self):
        status, report = self.run_case(descendant=True, planned=True)
        self.assertEqual(status, 0, report)
        self.assertEqual(report["root_exit_code"], 0)
        # Python launchers may add a helper; exact membership count is reported,
        # while native empty accounting proves all owned processes were reaped.
        self.assertGreaterEqual(report["active_processes_at_planned_teardown"], 1)
        self.assertTrue(report["planned_descendant_teardown"])
        self.assertFalse(report["natural_lifecycle_acceptance"])

    def test_failed_root_cannot_authorize_planned_success(self):
        status, report = self.run_case(descendant=True, code=7, planned=True)
        self.assertNotEqual(status, 0, report)
        self.assertEqual(report["root_exit_code"], 7)
        self.assertFalse(report["planned_descendant_teardown"])
        self.assertTrue(report["descendants_after_root_exit"])

    def test_control_eof_is_failure_even_in_planned_mode(self):
        status, report = self.run_case(descendant=True, planned=True, abort=True)
        self.assertNotEqual(status, 0, report)
        self.assertTrue(report["aborted"])
        self.assertFalse(report["planned_descendant_teardown"])


if __name__ == "__main__":
    unittest.main()
