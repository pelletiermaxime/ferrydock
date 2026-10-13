#!/usr/bin/env python3
"""Measure one owned native development window with ten noisy fixture containers."""
import argparse
import fcntl
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import re
import signal
import subprocess
import time
import uuid

ROOT = Path(__file__).resolve().parents[1]


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


def finish_run(*, app, child_pid, app_log, run, compose, endpoint, project, evidence, lock, result):
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
    parser.add_argument('--profile', action='store_true', help='Capture owned-process thread CPU and three debugger stack samples')
    parser.add_argument('--ansi', action='store_true', help='Emit colored UTF-8 fixture records to exercise ANSI processing under load')
    args = parser.parse_args()
    binary = args.binary.resolve()
    if not args.display:
        raise RuntimeError('This native check requires an X11/XWayland DISPLAY')
    stamp = datetime.now(timezone.utc).strftime('%Y%m%dT%H%M%SZ')
    evidence = ROOT / '.verification/log-performance' / (stamp + '-' + uuid.uuid4().hex[:8])
    evidence.mkdir(parents=True)
    project = 'ferrydock-log-perf-' + uuid.uuid4().hex[:8]
    lock = Path(f'/tmp/ferrydock-native-verification-{os.getuid()}.lock').open('a+')
    fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
    env = os.environ.copy()

    def run(command, timeout=30, check=True):
        return subprocess.run(command, env=env, capture_output=True, text=True, timeout=timeout, check=check)

    context = env.get('DOCKER_CONTEXT')
    endpoint = (run(['docker', 'context', 'inspect', context, '--format', '{{.Endpoints.docker.Host}}']).stdout.strip()
                if context else env.get('DOCKER_HOST') or run(['docker', 'context', 'inspect', '--format', '{{.Endpoints.docker.Host}}']).stdout.strip())
    if not endpoint.startswith('unix://'):
        raise RuntimeError('This check requires a local Unix Docker endpoint')
    for name in list(env):
        if name.startswith('COMPOSE_') or name in ('DOCKER_CONTEXT', 'WAYLAND_DISPLAY', 'ZED_HEADLESS'):
            env.pop(name)
    env.update(DOCKER_HOST=endpoint, DISPLAY=args.display, XDG_CONFIG_HOME=str(evidence / 'settings'))
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
              'backend': 'X11/XWayland', 'fixture_containers': 10, 'nominal_lines_per_second': 500, 'ansi_fixture': args.ansi, 'evidence': str(evidence)}
    app = None
    app_pid = None
    app_log = None
    try:
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
        deadline = time.monotonic() + 20
        window = None
        while time.monotonic() < deadline:
            if app.poll() is not None:
                raise RuntimeError('Application exited before a window appeared')
            found = run(['xdotool', 'search', '--all', '--onlyvisible', '--pid', str(app_pid), '--name', '^FerryDock$'], check=False)
            if found.returncode == 0:
                windows = found.stdout.split()
                assert len(windows) == 1, 'Refuse an ambiguous owned window'
                window = windows[0]
                break
            time.sleep(0.1)
        if window is None:
            raise RuntimeError('No owned FerryDock window appeared')
        result['window'] = window
        run(['xdotool', 'windowactivate', '--sync', window])

        def keys(*sequence):
            assert app.poll() is None
            assert run(['xdotool', 'getwindowfocus']).stdout.strip() == window, 'Focus changed; refuse input'
            run(['xdotool', 'key', '--window', window, '--clearmodifiers', *sequence])

        last_capture_completed = 0.0

        def screenshot(name):
            nonlocal last_capture_completed
            started = time.monotonic()
            path = evidence / (name + '.png')
            run(['magick', 'import', '-limit', 'thread', '1', '-window', window, str(path)])
            captured = time.monotonic()
            last_capture_completed = captured
            text = subprocess.run(['tesseract', str(path), 'stdout', '--psm', '11'], env=dict(env, OMP_THREAD_LIMIT='1'), capture_output=True, text=True, timeout=30, check=True).stdout
            (evidence / (name + '.txt')).write_text(text)
            result.setdefault('screenshot_timings', []).append({'name': name, 'capture_seconds': round(captured - started, 3), 'ocr_seconds': round(time.monotonic() - captured, 3)})
            return text

        time.sleep(1)
        keys('ctrl+f', 'ctrl+a')
        run(['xdotool', 'type', '--window', window, '--clearmodifiers', '--delay', '20', project])
        keys('Escape', 'Down', 'ctrl+4')
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
        keys('ctrl+f', 'ctrl+a')
        run(['xdotool', 'type', '--window', window, '--clearmodifiers', '--delay', '20', project + '-absent'])
        deadline = search_at + 3
        while time.monotonic() < deadline:
            time.sleep(0.2)
            text = screenshot('responsive-search')
            if 'No containers match' in text:
                break
        assert 'No containers match' in text, 'Search did not respond under load'
        result['search_response_seconds_including_capture_and_ocr'] = round(time.monotonic() - search_at, 3)
        result['search_visible_capture_seconds_including_typing'] = round(last_capture_completed - search_at, 3)
        result.update(status='passed', controls_respond_under_load=True, scope=f'One {build_profile} build and native window under nominal 500 lines/s; CPU is measured, not a portable threshold. Clipboard/Wayland not driven.')
    except Exception as error:
        result['error'] = str(error)
    finally:
        finish_run(app=app, child_pid=app_pid if args.profile else None, app_log=app_log,
                   run=run, compose=compose, endpoint=endpoint, project=project,
                   evidence=evidence, lock=lock, result=result)
    return 0 if result['status'] == 'passed' else 1


if __name__ == '__main__':
    raise SystemExit(main())
