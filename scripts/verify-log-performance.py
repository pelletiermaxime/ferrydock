#!/usr/bin/env python3
"""Measure an owned native window under log load; prefer Wayland in Wayland sessions."""
import argparse
import fcntl
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import re
import signal
import shutil
import subprocess
import time
import uuid

ROOT = Path(__file__).resolve().parents[1]


def desktop_environment(requested, display, environment):
    env = environment.copy()
    backend = requested
    if backend == 'auto':
        backend = 'wayland' if env.get('WAYLAND_DISPLAY') else 'x11'
    if backend == 'wayland':
        if not env.get('WAYLAND_DISPLAY') or not env.get('XDG_RUNTIME_DIR'):
            raise RuntimeError('Wayland requires WAYLAND_DISPLAY and XDG_RUNTIME_DIR from your desktop session')
        env.pop('DISPLAY', None)
    else:
        if not display:
            raise RuntimeError('X11 requires DISPLAY or --display')
        env.pop('WAYLAND_DISPLAY', None)
        env['DISPLAY'] = display
    return backend, env


class NativeDesktop:
    """Own native input resources and validate the target before each interaction."""

    WAYLAND_KEYS = {
        'ctrl+f': ['29:1', '33:1', '33:0', '29:0'],
        'ctrl+a': ['29:1', '30:1', '30:0', '29:0'],
        'ctrl+4': ['29:1', '5:1', '5:0', '29:0'],
        'Escape': ['1:1', '1:0'],
        'Down': ['108:1', '108:0'],
    }

    def __init__(self, backend, env, evidence, binary):
        self.backend, self.env, self.evidence, self.binary = backend, env, evidence, binary
        self.app = self.pid = self.window = None
        self.input_daemon = self.input_log = None
        self.input_socket = evidence / 'input.sock'

    def run(self, command, timeout=30, check=True):
        return subprocess.run(command, env=self.env, capture_output=True, text=True,
                              timeout=timeout, check=check)

    def doctor(self):
        tools = ['tesseract'] + (['kdotool', 'qdbus6', 'spectacle', 'ydotool', 'ydotoold']
                                if self.backend == 'wayland' else ['xdotool', 'magick'])
        missing = [tool for tool in tools if not shutil.which(tool)]
        if missing:
            raise RuntimeError('Missing native verification tools: ' + ', '.join(missing))
        if not self.binary.is_file():
            raise RuntimeError('Build the requested binary before benchmarking')
        if self.backend == 'wayland':
            socket = Path(self.env['WAYLAND_DISPLAY'])
            if not socket.is_absolute():
                socket = Path(self.env['XDG_RUNTIME_DIR']) / socket
            if not socket.is_socket():
                raise RuntimeError('Wayland display socket is unavailable: ' + str(socket))
            if not os.access('/dev/uinput', os.R_OK | os.W_OK):
                raise RuntimeError('Wayland input requires access to /dev/uinput; system permissions are not changed by this helper')
            # This is the demonstrated KDE/KWin adapter. Refuse unsupported
            # compositors rather than silently measuring XWayland instead.
            try:
                self.run(['qdbus6', 'org.kde.KWin', '/KWin', 'activeOutputName'])
            except subprocess.CalledProcessError as error:
                raise RuntimeError('Native Wayland automation currently requires KDE/KWin; use --backend x11 explicitly for XWayland') from error
        else:
            self.run(['xdotool', 'getdisplaygeometry'])
        return {'ready': True, 'backend': self.backend, 'display': self.env.get('DISPLAY'),
                'wayland_display': self.env.get('WAYLAND_DISPLAY')}

    def start_input(self):
        if self.backend != 'wayland':
            return
        if self.input_socket.exists():
            raise RuntimeError('Refuse an existing input socket')
        self.input_log = (self.evidence / 'input-daemon.log').open('w')
        self.input_daemon = subprocess.Popen(
            ['ydotoold', '--mouse-off', '--socket-path=' + str(self.input_socket)],
            env=self.env, stdout=self.input_log, stderr=self.input_log)
        (self.evidence / 'input-instance.json').write_text(json.dumps(
            {'pid': self.input_daemon.pid, 'socket': str(self.input_socket)}, indent=2) + '\n')
        deadline = time.monotonic() + 5
        while (not self.input_socket.exists() and self.input_daemon.poll() is None
               and time.monotonic() < deadline):
            time.sleep(0.1)
        if not self.input_socket.exists() or self.input_daemon.poll() is not None:
            raise RuntimeError('Owned Wayland input daemon failed; see input-daemon.log')
        self.env['YDOTOOL_SOCKET'] = str(self.input_socket)
        time.sleep(1)  # Let the compositor register the owned virtual keyboard.

    def alive(self):
        assert self.app.poll() is None, 'Owned application exited'
        assert Path(f'/proc/{self.pid}/exe').resolve() == self.binary, 'Owned executable changed'

    def attach(self, app, pid):
        self.app, self.pid = app, pid
        deadline = time.monotonic() + 20
        while time.monotonic() < deadline:
            self.alive()
            command = (['kdotool', 'search', '--all', '--pid', str(pid), '--title', '^FerryDock$']
                       if self.backend == 'wayland' else
                       ['xdotool', 'search', '--all', '--onlyvisible', '--pid', str(pid), '--name', '^FerryDock$'])
            found = self.run(command, check=False)
            # kdotool returns exit 0 with empty output before the surface appears.
            windows = found.stdout.split() if found.returncode == 0 else []
            if windows:
                assert len(windows) == 1, 'Refuse an ambiguous owned window'
                self.window = windows[0]
                break
            time.sleep(0.1)
        if self.window is None:
            raise RuntimeError('No owned FerryDock window appeared')
        command = (['kdotool', 'windowactivate', self.window] if self.backend == 'wayland'
                   else ['xdotool', 'windowactivate', '--sync', self.window])
        self.run(command)

    def assert_focus(self):
        self.alive()
        if self.backend == 'wayland':
            active = self.run(['kdotool', 'getactivewindow']).stdout.strip()
            pid = self.run(['kdotool', 'getwindowpid', self.window]).stdout.strip()
            assert pid == str(self.pid), 'Window ownership changed; refuse input/capture'
        else:
            active = self.run(['xdotool', 'getwindowfocus']).stdout.strip()
        assert active == self.window, 'Focus changed; refuse input/capture'

    def keys(self, *sequence):
        self.assert_focus()
        if self.backend == 'wayland':
            events = [event for key in sequence for event in self.WAYLAND_KEYS[key]]
            self.run(['ydotool', 'key', *events])
        else:
            self.run(['xdotool', 'key', '--window', self.window, '--clearmodifiers', *sequence])

    def type(self, text):
        self.assert_focus()
        command = (['ydotool', 'type', '--key-delay', '20', '--', text]
                   if self.backend == 'wayland' else
                   ['xdotool', 'type', '--window', self.window, '--clearmodifiers', '--delay', '20', text])
        self.run(command)

    def capture(self, path):
        self.assert_focus()
        command = (['spectacle', '--background', '--activewindow', '--nonotify', '--no-decoration', '--no-shadow', '--output', str(path)]
                   if self.backend == 'wayland' else
                   ['magick', 'import', '-limit', 'thread', '1', '-window', self.window, str(path)])
        self.run(command)
        self.assert_focus()

    def close(self):
        try:
            exited = stop_owned_process(self.input_daemon)
            if not exited:
                raise RuntimeError('Owned input daemon remains')
        finally:
            if self.input_log is not None:
                self.input_log.close()
            if (self.input_daemon is not None and self.input_daemon.poll() is not None
                    and self.input_socket.exists()):
                self.input_socket.unlink()
        cleanup = {'owned_input_daemon_exited': exited, 'owned_input_socket_removed': not self.input_socket.exists()}
        (self.evidence / 'input-cleanup.json').write_text(json.dumps(cleanup, indent=2) + '\n')
        return cleanup


def stop_owned_process(app, child_pid=None):
    if child_pid is not None:
        try:
            os.kill(child_pid, signal.SIGTERM)
        except ProcessLookupError:
            pass
    if app is not None and app.poll() is None:
        if child_pid is not None:
            try:
                app.stdin.write(b'-gdb-exit\n')
                app.stdin.flush()
            except BrokenPipeError:
                pass
        try:
            app.terminate()
        except ProcessLookupError:
            pass
        try:
            app.wait(timeout=5)
        except subprocess.TimeoutExpired:
            try:
                app.kill()
            except ProcessLookupError:
                pass
            app.wait(timeout=5)
    return ((app is None or app.poll() is not None)
            and (child_pid is None or not Path(f'/proc/{child_pid}').exists()))


def finish_run(*, app, child_pid, app_log, run, compose, endpoint, project, evidence, lock, result, desktop=None):
    """Attempt process and fixture cleanup independently, then persist the report."""
    try:
        result['owned_process_exited'] = stop_owned_process(app, child_pid)
        if not result['owned_process_exited']:
            result.update(status='failed', process_cleanup_error='Owned application processes remain')
    except Exception as error:
        result.update(status='failed', owned_process_exited=False, process_cleanup_error=str(error))
    try:
        if app_log is not None:
            app_log.close()
    except OSError as error:
        result.update(status='failed', process_cleanup_error=str(error))
    if desktop is not None:
        try:
            result['input_cleanup'] = desktop.close()
        except Exception as error:
            result.update(status='failed', input_cleanup_error=str(error))
    try:
        cleanup = run(compose + ['down', '--volumes', '--remove-orphans'], timeout=45)
        (evidence / 'cleanup.log').write_text(cleanup.stdout + cleanup.stderr)
        remaining = run(['docker', '--host', endpoint, 'ps', '-aq', '--filter', f'label=com.docker.compose.project={project}']).stdout.strip()
        networks = run(['docker', '--host', endpoint, 'network', 'ls', '-q', '--filter', f'label=com.docker.compose.project={project}']).stdout.strip()
        result['cleanup_success'] = not remaining and not networks
        if not result['cleanup_success']:
            result.update(status='failed', error='Owned fixture resources remain')
    except Exception as error:
        result.update(status='failed', cleanup_success=False, cleanup_error=str(error))
    (evidence / 'result.json').write_text(json.dumps(result, indent=2) + '\n')
    lock.close()
    print(json.dumps(result, indent=2))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, default=ROOT / 'target/debug/ferrydock')
    parser.add_argument('--display', default=os.environ.get('DISPLAY'))
    parser.add_argument('--backend', choices=('auto', 'wayland', 'x11'), default='auto',
                        help='Prefer native Wayland when WAYLAND_DISPLAY is set; use x11 explicitly for comparison')
    parser.add_argument('--profile', action='store_true', help='Capture owned-process thread CPU and three debugger stack samples')
    parser.add_argument('--ansi', action='store_true', help='Emit colored UTF-8 fixture records to exercise ANSI processing under load')
    args = parser.parse_args()
    binary = args.binary.resolve()
    backend, env = desktop_environment(args.backend, args.display, os.environ)
    stamp = datetime.now(timezone.utc).strftime('%Y%m%dT%H%M%SZ')
    evidence = ROOT / '.verification/log-performance' / (stamp + '-' + uuid.uuid4().hex[:8])
    evidence.mkdir(parents=True)
    project = 'ferrydock-log-perf-' + uuid.uuid4().hex[:8]
    lock = Path(f'/tmp/ferrydock-native-verification-{os.getuid()}.lock').open('a+')
    fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)

    def run(command, timeout=30, check=True):
        return subprocess.run(command, env=env, capture_output=True, text=True, timeout=timeout, check=check)

    context = env.get('DOCKER_CONTEXT')
    endpoint = (run(['docker', 'context', 'inspect', context, '--format', '{{.Endpoints.docker.Host}}']).stdout.strip()
                if context else env.get('DOCKER_HOST') or run(['docker', 'context', 'inspect', '--format', '{{.Endpoints.docker.Host}}']).stdout.strip())
    if not endpoint.startswith('unix://'):
        raise RuntimeError('This check requires a local Unix Docker endpoint')
    for name in list(env):
        if name.startswith('COMPOSE_') or name in ('DOCKER_CONTEXT', 'ZED_HEADLESS'):
            env.pop(name)
    env.update(DOCKER_HOST=endpoint, XDG_CONFIG_HOME=str(evidence / 'settings'))
    desktop = NativeDesktop(backend, env, evidence, binary)
    services = {}
    for service in ('probe', 'worker'):
        emit = (f'printf "\\033[32mFERRY_PERF_{service}_TICK:%s café\\033[0m\\n" "$$i"'
                if args.ansi else f'echo FERRY_PERF_{service}_TICK:$$i')
        command = f'echo FERRY_PERF_{service}_BOOT; i=0; while true; do j=0; while [ "$$j" -lt 5 ]; do {emit}; i=$$((i+1)); j=$$((j+1)); done; sleep 0.1; done'
        services[service] = {'image': 'alpine:3.23', 'command': ['sh', '-c', command], 'stop_grace_period': '1s', 'labels': {'dev.ferrydock.fixture': 'true'}}
    fixture = evidence / 'compose.json'
    fixture.write_text(json.dumps({'services': services}))
    compose = ['docker', '--host', endpoint, 'compose', '--project-name', project, '--file', str(fixture)]
    build_profile = 'release' if binary == (ROOT / 'target/release/ferrydock').resolve() else 'development' if binary == (ROOT / 'target/debug/ferrydock').resolve() else 'custom'
    result = {'status': 'failed', 'project': project, 'endpoint': endpoint, 'binary': str(binary), 'build_profile': build_profile, 'binary_sha256': hashlib.sha256(binary.read_bytes()).hexdigest(),
              'backend': 'Wayland' if backend == 'wayland' else 'X11/XWayland',
              'display': env.get('DISPLAY'), 'wayland_display': env.get('WAYLAND_DISPLAY'),
              'fixture_containers': 10, 'nominal_lines_per_second': 500, 'ansi_fixture': args.ansi, 'evidence': str(evidence)}
    app = None
    app_pid = None
    app_log = None
    try:
        doctor = desktop.doctor()
        run(['docker', '--host', endpoint, 'version', '--format', '{{.Server.Version}}'])
        run(['docker', '--host', endpoint, 'compose', 'version'])
        (evidence / 'doctor.json').write_text(json.dumps(doctor, indent=2) + '\n')
        desktop.start_input()
        started = run(compose + ['up', '-d', '--scale', 'probe=5', '--scale', 'worker=5'], timeout=60)
        (evidence / 'launch.log').write_text(started.stdout + started.stderr)
        ids = run(['docker', '--host', endpoint, 'ps', '-aq', '--filter', f'label=com.docker.compose.project={project}']).stdout.split()
        assert len(ids) == 10, 'Fixture must have ten containers'
        for container in json.loads(run(['docker', '--host', endpoint, 'inspect', *ids]).stdout):
            labels = container['Config']['Labels']
            assert labels.get('dev.ferrydock.fixture') == 'true' and labels.get('com.docker.compose.project') == project
        app_log = (evidence / 'application.log').open('w')
        app = subprocess.Popen(['gdb', '--interpreter=mi', '--args', str(binary)] if args.profile else [str(binary)], env=env, stdin=subprocess.PIPE if args.profile else None, stdout=app_log, stderr=app_log)
        if args.profile:
            app.stdin.write(b'-gdb-set pagination off\n-gdb-set mi-async on\n-exec-run\n'); app.stdin.flush()
            deadline = time.monotonic() + 10
            while time.monotonic() < deadline:
                children = Path(f'/proc/{app.pid}/task/{app.pid}/children').read_text().split()
                if children:
                    app_pid = int(children[0]); break
                time.sleep(0.1)
            if app_pid is None:
                raise RuntimeError('Debugger did not launch its owned child')
        else:
            app_pid = app.pid
        assert Path(f'/proc/{app_pid}/exe').resolve() == binary, 'Owned child executable does not match the requested binary'
        result['pid'] = app_pid
        desktop.attach(app, app_pid)
        result['window'] = desktop.window
        if backend == 'wayland':
            result['kwin_window_info'] = run(['qdbus6', 'org.kde.KWin', '/KWin', 'getWindowInfo', desktop.window]).stdout
        result['input_method'] = 'ydotool (owned virtual keyboard)' if backend == 'wayland' else 'xdotool'
        result['capture_method'] = 'Spectacle' if backend == 'wayland' else 'ImageMagick X11 capture'

        last_capture_completed = 0.0

        def screenshot(name):
            nonlocal last_capture_completed
            started = time.monotonic()
            path = evidence / (name + '.png')
            desktop.capture(path)
            captured = time.monotonic()
            last_capture_completed = captured
            text = subprocess.run(['tesseract', str(path), 'stdout', '--psm', '11'], env=dict(env, OMP_THREAD_LIMIT='1'), capture_output=True, text=True, timeout=30, check=True).stdout
            (evidence / (name + '.txt')).write_text(text)
            result.setdefault('screenshot_timings', []).append({'name': name, 'capture_seconds': round(captured - started, 3), 'ocr_seconds': round(time.monotonic() - captured, 3)})
            return text

        time.sleep(1)
        desktop.keys('ctrl+f', 'ctrl+a')
        desktop.type(project)
        desktop.keys('Escape', 'Down', 'ctrl+4')
        selected_at = time.monotonic()
        deadline = selected_at + 15
        # OCR commonly confuses these glyphs in hex fixture IDs. Ownership is
        # established independently by PID/window and Docker labels; compare
        # only the selected Compose header with the same normalized ID.
        normalize = lambda text: text.casefold().translate(str.maketrans({'o': '0', 'l': '1', 'i': '1', '@': '0'}))
        while time.monotonic() < deadline:
            text = screenshot('busy-logs')
            if re.search(r'c0mp0se.{0,12}' + re.escape(normalize(project)), normalize(text), re.S) and 'FERRY_PERF_' in text:
                break
            time.sleep(0.2)
        else:
            raise RuntimeError('The selected owned project did not render live log output')
        result['selection_to_live_output_seconds'] = round(time.monotonic() - selected_at, 3)
        result['selection_to_live_capture_seconds'] = round(last_capture_completed - selected_at, 3)

        def cpu_ticks():
            fields = Path(f'/proc/{app_pid}/stat').read_text().split(')', 1)[1].split()
            return int(fields[11]) + int(fields[12])

        def thread_ticks():
            result = {}
            for stat in Path(f'/proc/{app_pid}/task').glob('*/stat'):
                try:
                    raw = stat.read_text()
                    fields = raw.split(')', 1)[1].split()
                    result[stat.parent.name] = (raw.split('(', 1)[1].split(')', 1)[0], int(fields[11]) + int(fields[12]))
                except FileNotFoundError:
                    pass
            return result

        time.sleep(1)
        before = cpu_ticks()
        threads_before = thread_ticks()
        sample_started = time.monotonic()
        time.sleep(5)
        result['cpu_percent_over_5_seconds'] = round((cpu_ticks() - before) / os.sysconf('SC_CLK_TCK') / (time.monotonic() - sample_started) * 100, 2)
        result['resident_mib'] = round(int(Path(f'/proc/{app_pid}/statm').read_text().split()[1]) * os.sysconf('SC_PAGE_SIZE') / 1048576, 1)
        result['thread_cpu_ticks'] = sorted([(tid, name, ticks - threads_before.get(tid, ('', 0))[1]) for tid, (name, ticks) in thread_ticks().items()], key=lambda row: row[2], reverse=True)[:10]
        if args.profile:
            for sample in range(3):
                app.stdin.write(b'-exec-interrupt\n'); app.stdin.flush()
                time.sleep(0.2)
                app.stdin.write(b'-interpreter-exec console "thread 1"\n-interpreter-exec console "bt 18"\n'); app.stdin.flush()
                time.sleep(0.4)
                app.stdin.write(b'-exec-continue\n'); app.stdin.flush()
                time.sleep(0.2)
        text = screenshot('busy-logs-after-sample')
        assert 'FERRY_PERF_' in text, 'Live output disappeared during the sample'
        search_at = time.monotonic()
        desktop.keys('ctrl+f', 'ctrl+a')
        desktop.type(project + '-absent')
        deadline = search_at + 3
        while time.monotonic() < deadline:
            time.sleep(0.2)
            text = screenshot('responsive-search')
            if 'No containers match' in text:
                break
        assert 'No containers match' in text, 'Search did not respond under load'
        result['search_response_seconds_including_capture_and_ocr'] = round(time.monotonic() - search_at, 3)
        result['search_visible_capture_seconds_including_typing'] = round(last_capture_completed - search_at, 3)
        result.update(status='passed', controls_respond_under_load=True, scope=f'One {build_profile} build and {result["backend"]} window under nominal 500 lines/s; CPU is measured, not a portable threshold. Clipboard not driven. Search timings include backend-specific input/capture overhead.')
    except Exception as error:
        result['error'] = str(error)
    finally:
        finish_run(app=app, child_pid=app_pid if args.profile else None, app_log=app_log,
                   run=run, compose=compose, endpoint=endpoint, project=project,
                   evidence=evidence, lock=lock, result=result, desktop=desktop)
    return 0 if result['status'] == 'passed' else 1


if __name__ == '__main__':
    raise SystemExit(main())
