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
                   remaining='', log_error=None, desktop=None):
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
                    evidence=evidence, lock=lock, result=result, desktop=desktop,
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

    def test_input_shutdown_failure_still_cleans_fixture_and_writes_report(self):
        desktop = Mock()
        desktop.close.side_effect = RuntimeError('cannot stop owned input daemon')
        saved = self.finish_run(self.debugger(), desktop=desktop)
        desktop.close.assert_called_once()
        self.assertTrue(saved['owned_process_exited'])
        self.assertTrue(saved['cleanup_success'])
        self.assertEqual(saved['status'], 'failed')
        self.assertEqual(saved['input_cleanup_error'], 'cannot stop owned input daemon')


class NativeDesktopTests(unittest.TestCase):
    def test_wayland_session_does_not_keep_x11_fallback(self):
        env = {'WAYLAND_DISPLAY': 'wayland-0', 'XDG_RUNTIME_DIR': '/run/user/1000',
               'DISPLAY': ':0'}
        backend, child = VERIFIER.desktop_environment('auto', ':0', env)
        self.assertEqual(backend, 'wayland')
        self.assertNotIn('DISPLAY', child)
        self.assertEqual(env['DISPLAY'], ':0')
        backend, child = VERIFIER.desktop_environment('x11', ':0', env)
        self.assertEqual(backend, 'x11')
        self.assertNotIn('WAYLAND_DISPLAY', child)
        with self.assertRaisesRegex(RuntimeError, 'Wayland requires'):
            VERIFIER.desktop_environment('wayland', ':0', {'DISPLAY': ':0'})

    def desktop(self, directory):
        desktop = VERIFIER.NativeDesktop('wayland', {}, Path(directory), Path('/test/binary'))
        desktop.pid, desktop.window = 12345, '{owned-window}'
        desktop.alive = Mock()
        return desktop

    def test_focus_or_owner_change_refuses_virtual_keyboard_events(self):
        with tempfile.TemporaryDirectory() as directory:
            for active, owner in [('{other-window}', '12345'), ('{owned-window}', '99999')]:
                with self.subTest(active=active, owner=owner):
                    desktop = self.desktop(directory)
                    desktop.run = Mock(side_effect=[SimpleNamespace(stdout=active), SimpleNamespace(stdout=owner)])
                    with self.assertRaisesRegex(AssertionError, 'refuse input'):
                        desktop.type('fixture-only-search')
                    self.assertEqual(desktop.run.call_count, 2)
                    self.assertFalse(any(call.args[0][0] == 'ydotool' for call in desktop.run.call_args_list))

    def test_empty_wayland_discovery_waits_for_owned_surface(self):
        with tempfile.TemporaryDirectory() as directory:
            desktop = self.desktop(directory)
            desktop.window = None
            desktop.run = Mock(side_effect=[
                SimpleNamespace(returncode=0, stdout=''),
                SimpleNamespace(returncode=0, stdout='{owned-window}\n'),
                SimpleNamespace(returncode=0, stdout=''),
            ])
            with patch.object(VERIFIER.time, 'sleep'):
                desktop.attach(Mock(), 12345)
            self.assertEqual(desktop.window, '{owned-window}')
            self.assertEqual(desktop.run.call_args_list[-1].args[0],
                             ['kdotool', 'windowactivate', '{owned-window}'])


if __name__ == '__main__':
    unittest.main()
