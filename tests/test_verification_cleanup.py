import importlib.util
import json
from pathlib import Path
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import Mock, patch


SCRIPT = Path(__file__).resolve().parents[1] / 'scripts/verify-log-performance.py'
SPEC = importlib.util.spec_from_file_location('verify_log_performance', SCRIPT)
VERIFIER = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(VERIFIER)


class VerificationCleanupTests(unittest.TestCase):
    def finish_run(self, app, kill_error=None, child_exists=False, fixture_error=None,
                   remaining='', log_error=None):
        run = Mock(side_effect=fixture_error if fixture_error else [
            SimpleNamespace(stdout='', stderr=''),
            SimpleNamespace(stdout=remaining, stderr=''),
            SimpleNamespace(stdout='', stderr=''),
        ])
        lock = Mock()
        app_log = Mock()
        app_log.close.side_effect = log_error
        result = {'status': 'passed'}
        with tempfile.TemporaryDirectory() as directory:
            evidence = Path(directory)
            with patch.object(VERIFIER.os, 'kill', side_effect=kill_error) as kill, \
                    patch.object(VERIFIER, 'Path') as process_path, patch('builtins.print'):
                process_path.return_value.exists.side_effect = (
                    lambda: True if kill_error is not None and not kill.called else child_exists)
                VERIFIER.finish_run(
                    app=app, child_pid=12345, app_log=app_log, run=run,
                    compose=['docker', 'compose', '--project-name', 'owned'],
                    endpoint='unix:///test.sock', project='owned',
                    evidence=evidence, lock=lock, result=result,
                )
            saved = json.loads((evidence / 'result.json').read_text())
        self.assertEqual(run.call_args_list[0].args[0][-3:],
                         ['down', '--volumes', '--remove-orphans'])
        self.assertEqual(run.call_count, 1 if fixture_error else 3)
        lock.close.assert_called_once()
        app_log.close.assert_called_once()
        return saved

    def debugger(self):
        app = Mock()
        app.poll.side_effect = [None, 0]
        return app

    def test_broken_debugger_pipe_still_cleans_fixture_and_writes_report(self):
        app = self.debugger()
        app.stdin.write.side_effect = BrokenPipeError('debugger exited')
        saved = self.finish_run(app)
        app.terminate.assert_called_once()
        app.wait.assert_called_once_with(timeout=5)
        self.assertTrue(saved['cleanup_success'])
        self.assertTrue(saved['owned_process_exited'])
        self.assertEqual(saved['status'], 'passed')

    def test_child_exit_race_still_cleans_fixture_and_writes_report(self):
        saved = self.finish_run(self.debugger(), kill_error=ProcessLookupError())
        self.assertTrue(saved['cleanup_success'])
        self.assertTrue(saved['owned_process_exited'])
        self.assertEqual(saved['status'], 'passed')

    def test_process_shutdown_failure_still_cleans_fixture_and_fails_report(self):
        saved = self.finish_run(self.debugger(), kill_error=PermissionError('cannot stop child'))
        self.assertTrue(saved['cleanup_success'])
        self.assertFalse(saved['owned_process_exited'])
        self.assertEqual(saved['status'], 'failed')
        self.assertEqual(saved['process_cleanup_error'], 'cannot stop child')

    def test_surviving_child_fails_report_after_fixture_cleanup(self):
        saved = self.finish_run(self.debugger(), child_exists=True)
        self.assertTrue(saved['cleanup_success'])
        self.assertFalse(saved['owned_process_exited'])
        self.assertEqual(saved['status'], 'failed')

    def test_fixture_shutdown_failure_still_writes_report_and_releases_lock(self):
        saved = self.finish_run(self.debugger(), fixture_error=RuntimeError('compose down failed'))
        self.assertTrue(saved['owned_process_exited'])
        self.assertFalse(saved['cleanup_success'])
        self.assertEqual(saved['cleanup_error'], 'compose down failed')
        self.assertEqual(saved['status'], 'failed')

    def test_remaining_fixture_resources_fail_the_report(self):
        saved = self.finish_run(self.debugger(), remaining='owned-container')
        self.assertTrue(saved['owned_process_exited'])
        self.assertFalse(saved['cleanup_success'])
        self.assertEqual(saved['error'], 'Owned fixture resources remain')
        self.assertEqual(saved['status'], 'failed')

    def test_log_close_failure_does_not_skip_fixture_cleanup(self):
        saved = self.finish_run(self.debugger(), log_error=OSError('cannot close log'))
        self.assertTrue(saved['owned_process_exited'])
        self.assertTrue(saved['cleanup_success'])
        self.assertEqual(saved['process_cleanup_error'], 'cannot close log')
        self.assertEqual(saved['status'], 'failed')


if __name__ == '__main__':
    unittest.main()
