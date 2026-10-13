#!/usr/bin/env python3
"""Own one native FerryDock proof: launch, doctor, user input, evidence, cleanup.

No app setters or debug endpoints. Docker commands set up a fixture and observe
side effects; product operations go through the real window.
"""
import argparse
import csv
import fcntl
import hashlib
import io
import json
import os
from pathlib import Path
import re
import secrets
import shutil
import subprocess
import sys
import time
import uuid
from datetime import datetime, timezone

ROOT = Path(__file__).resolve().parents[4]
sys.path.insert(0, str(ROOT / "scripts"))
from verification_evidence import keep_runs, prune_runs

FIXTURE = Path(__file__).with_name("fixture.compose.yaml")
LOCK = Path("/tmp") / f"ferrydock-native-verification-{os.getuid()}.lock"


def normalize(text):
    return re.sub(r"[^a-z0-9]", "", text.lower())


def command(argv, env, timeout=30, check=True):
    result = subprocess.run(argv, env=env, cwd=ROOT, text=True,
                            capture_output=True, timeout=timeout)
    if check and result.returncode:
        raise RuntimeError(f"{argv[0]} exited {result.returncode}: {result.stderr.strip()}")
    return result


def endpoint(env):
    context = env.get("DOCKER_CONTEXT")
    if context:
        host = command(["docker", "context", "inspect", context, "--format",
                        "{{json .Endpoints.docker.Host}}"], env).stdout
        return context, json.loads(host)
    if env.get("DOCKER_HOST"):
        return "DOCKER_HOST", env["DOCKER_HOST"]
    context = command(["docker", "context", "show"], env).stdout.strip()
    host = command(["docker", "context", "inspect", context, "--format",
                    "{{json .Endpoints.docker.Host}}"], env).stdout
    return context, json.loads(host)


def base_doctor(binary, env):
    tools = {name: shutil.which(name) for name in ("docker", "xdotool", "magick", "tesseract")}
    if not all(tools.values()):
        raise RuntimeError(f"Missing native verification tools: {tools}")
    import PIL  # Pixel inspection is used only for mouse-hover calibration.
    if not env.get("DISPLAY"):
        raise RuntimeError("DISPLAY is missing. On this observed desktop use --display :0.")
    if not binary.is_file():
        raise RuntimeError("Build first: cargo build --release --locked")
    # This module is compiled only for the optional headless unit tests. Editing
    # it should not make a current shipping binary appear stale.
    sources = [path for path in (ROOT / "src").rglob("*.rs")
               if path != ROOT / "src/headless_tests.rs"] + [ROOT / "Cargo.toml", ROOT / "Cargo.lock"]
    if any(path.stat().st_mtime > binary.stat().st_mtime for path in sources):
        raise RuntimeError("Release binary predates source/config; rebuild with cargo build --release --locked")
    command(["xdotool", "getdisplaygeometry"], env)
    context, host = endpoint(env)
    if not host.startswith("unix://"):
        raise RuntimeError(f"FerryDock requires a local Unix socket; active endpoint is {host}")
    # Pin this run's daemon in child environments, without changing Docker's
    # global context or the user's environment.
    env.pop("DOCKER_CONTEXT", None)
    env["DOCKER_HOST"] = host
    server = command(["docker", "version", "--format", "{{.Server.Version}}"], env).stdout.strip()
    command(["docker", "compose", "version"], env)
    busy = False
    if LOCK.exists():
        with LOCK.open("r") as lock:
            try:
                fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
                fcntl.flock(lock, fcntl.LOCK_UN)
            except BlockingIOError:
                busy = True
    return {"ready": not busy, "binary": str(binary),
            "binary_sha256": hashlib.sha256(binary.read_bytes()).hexdigest(),
            "display": env["DISPLAY"], "docker_context_at_launch": context,
            "docker_endpoint": host, "docker_server": server,
            "native_input_busy": busy, "pillow_version": PIL.__version__, "tools": tools}


class Proof:
    def __init__(self, binary, env, feature, evidence):
        self.binary, self.env, self.feature, self.evidence = binary, env, feature, evidence
        # Avoid ambiguous 0/O and 1/l glyphs without fuzzy identity matching.
        self.project = "ferrydock-verify-" + "".join(secrets.choice("acehmnrsuvwxz") for _ in range(12))
        self.compose = ["docker", "compose", "--project-name", self.project, "--file", str(FIXTURE)]
        self.process = None
        self.window = None
        self.container = None
        self.container_name = None
        self.fixture_attempted = False
        self.created_ids = set()
        self.hover_transform = None
        self.serial = 0
        self.evidence.mkdir(parents=True, mode=0o700, exist_ok=False)
        self.app_log = None

    def save(self, name, data):
        (self.evidence / name).write_text(json.dumps(data, indent=2) + "\n")

    def action(self, label, **details):
        entry = {"at": datetime.now(timezone.utc).isoformat(), "action": label, **details}
        with (self.evidence / "actions.jsonl").open("a") as output:
            output.write(json.dumps(entry) + "\n")

    def project_ids(self):
        text = command(["docker", "ps", "-aq", "--no-trunc", "--filter",
                        f"label=com.docker.compose.project={self.project}"], self.env).stdout
        return set(text.split())

    def inspect(self):
        if not self.container:
            raise RuntimeError("No owned fixture container")
        info = json.loads(command(["docker", "inspect", self.container], self.env).stdout)[0]
        labels = info.get("Config", {}).get("Labels", {})
        if labels.get("dev.ferrydock.fixture") != "true" or labels.get("com.docker.compose.project") != self.project:
            raise RuntimeError("Fixture ownership changed; refusing further actions")
        if not info["Name"].startswith("/" + self.project + "-"):
            raise RuntimeError("Unexpected fixture name")
        return info

    def launch(self):
        if self.project_ids():
            raise RuntimeError("Generated project already exists; refusing to adopt it")
        self.action("create dedicated Compose fixture", project=self.project)
        self.fixture_attempted = True
        result = command(self.compose + ["up", "-d"], self.env, timeout=120)
        (self.evidence / "fixture-launch.log").write_text(result.stdout + result.stderr)
        self.created_ids = self.project_ids()
        if len(self.created_ids) != 1:
            raise RuntimeError("Expected exactly one probe container")
        self.container = next(iter(self.created_ids))
        info = self.inspect()
        self.container_name = info["Name"].lstrip("/")
        self.save("fixture-before.json", {"id": self.container, "name": self.container_name,
                                          "project": self.project, "state": info["State"]["Status"]})
        self.app_log = (self.evidence / "application.log").open("w")
        self.action("launch real application", binary=str(self.binary))
        self.process = subprocess.Popen([str(self.binary)], env=self.env, cwd=ROOT,
                                        stdout=self.app_log, stderr=self.app_log)
        deadline = time.monotonic() + 20
        while time.monotonic() < deadline:
            if self.process.poll() is not None:
                raise RuntimeError("FerryDock exited before a window appeared")
            found = command(["xdotool", "search", "--all", "--onlyvisible", "--pid",
                             str(self.process.pid), "--name", "^FerryDock$"], self.env, check=False)
            if found.returncode == 0 and len(found.stdout.split()) == 1:
                self.window = found.stdout.split()[0]
                break
            time.sleep(0.1)
        if not self.window:
            raise RuntimeError("No visible window for the launched PID")
        self.save("doctor-instance.json", self.doctor())
        self.focus()
        self.key("ctrl+f")
        self.type(self.project)
        self.wait_text(self.project, "01-filtered")
        self.key("Escape", "Down")
        self.wait_selected("02-selected")

    def doctor(self):
        if self.process.poll() is not None:
            raise RuntimeError("Owned application process is not running")
        executable = Path(f"/proc/{self.process.pid}/exe").resolve()
        if executable != self.binary:
            raise RuntimeError(f"Owned process is not the expected binary: {executable}")
        found = command(["xdotool", "search", "--all", "--onlyvisible", "--pid",
                         str(self.process.pid), "--name", "^FerryDock$"], self.env)
        if self.window not in found.stdout.split():
            raise RuntimeError("Window no longer belongs to the launched PID")
        info = self.inspect()
        return {"ready": True, "app_pid": self.process.pid, "window": self.window,
                "binary": str(executable), "project": self.project,
                "container_id": self.container, "container_name": self.container_name,
                "container_state": info["State"]["Status"], "docker_endpoint": self.env["DOCKER_HOST"]}

    def focus(self):
        self.doctor()
        command(["xdotool", "windowactivate", "--sync", self.window], self.env, timeout=10)
        self.check_focus()

    def check_focus(self):
        self.doctor()
        active = command(["xdotool", "getactivewindow"], self.env).stdout.strip()
        if active != self.window:
            raise RuntimeError("FerryDock lost focus; refusing input to another application")

    def key(self, *keys):
        self.check_focus()
        self.action("native key", keys=keys, window=self.window)
        # Targeted X11 events cannot become keystrokes in another application.
        command(["xdotool", "key", "--window", self.window, *keys], self.env)

    def type(self, text):
        self.check_focus()
        self.action("native typing", text=text, window=self.window)
        command(["xdotool", "type", "--window", self.window, "--delay", "20", text], self.env)

    def capture(self, label):
        self.doctor()
        self.serial += 1
        path = self.evidence / f"{self.serial:03d}-{label}.png"
        command(["magick", "import", "-window", self.window, str(path)], self.env)
        from PIL import Image
        with Image.open(path) as image:
            enlarged = image.resize((image.width * 2, image.height * 2))
            buffer = io.BytesIO()
            enlarged.save(buffer, format="PNG")
        ocr = subprocess.run(["tesseract", "stdin", "stdout", "-l", "eng", "--psm", "11"],
                             input=buffer.getvalue(), capture_output=True, env=self.env,
                             timeout=30, check=True)
        text = ocr.stdout.decode()
        path.with_suffix(".txt").write_text(text)
        return path, text

    def wait_text(self, expected, label, timeout=15, occurrences=1):
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            path, text = self.capture(label)
            if normalize(text).count(normalize(expected)) >= occurrences:
                return path, text
            time.sleep(0.2)
        raise RuntimeError(f"Native screen did not show {expected!r}; inspect retained screenshots/OCR")

    def wait_selected(self, label):
        deadline = time.monotonic() + 15
        while time.monotonic() < deadline:
            path, text = self.capture(label)
            if normalize(text).count(normalize(self.container_name)) >= 2:
                return path, text
            time.sleep(0.2)
        raise RuntimeError("Could not prove fixture appears in both the list and selected detail header")

    def text_box(self, label, screenshot):
        tsv = command(["tesseract", str(screenshot), "stdout", "-l", "eng", "--psm", "11", "tsv"], self.env).stdout
        rows = list(csv.DictReader(io.StringIO(tsv), delimiter="\t"))
        lines = {}
        for row in rows:
            if row["level"] == "5" and row["text"].strip() and float(row["conf"]) > 35:
                lines.setdefault((row["block_num"], row["par_num"], row["line_num"]), []).append(row)
        wanted = [normalize(word) for word in label.split()]
        matches = []
        for words in lines.values():
            words.sort(key=lambda item: int(item["left"]))
            for offset in range(len(words) - len(wanted) + 1):
                selected = words[offset:offset + len(wanted)]
                if [normalize(word["text"]) for word in selected] == wanted:
                    left = min(int(word["left"]) for word in selected)
                    top = min(int(word["top"]) for word in selected)
                    right = max(int(word["left"]) + int(word["width"]) for word in selected)
                    bottom = max(int(word["top"]) + int(word["height"]) for word in selected)
                    matches.append((left, top, right, bottom))
        if not matches:
            raise RuntimeError(f"No OCR control named {label!r}")
        # Refresh in the header wins over Refresh logs. No fixed coordinates.
        return min(matches, key=lambda box: box[1])

    def calibrate_mouse(self):
        from PIL import Image, ImageChops
        # Refresh is harmless, inactive, and has a distinct hover background.
        path, _ = self.capture("mouse-before")
        box = self.text_box("Refresh", path)
        geometry = command(["xdotool", "getwindowgeometry", "--shell", self.window], self.env).stdout
        values = dict(line.split("=", 1) for line in geometry.splitlines() if "=" in line)
        with Image.open(path) as image:
            width, height = image.size
            before = image.convert("RGB")
        sx, sy = int(values["WIDTH"]) / width, int(values["HEIGHT"]) / height
        center = ((box[0] + box[2]) / 2, (box[1] + box[3]) / 2)
        region = (max(0, box[0] - 7), max(0, box[1] - 6), min(width, box[2] + 7), min(height, box[3] + 6))
        for factor in (1, 0.5, 2, 0.625, 1.6):
            self.check_focus()
            command(["xdotool", "mousemove", "--window", self.window,
                     str(round(center[0] * sx * factor)), str(round(center[1] * sy * factor))], self.env)
            time.sleep(0.15)
            candidate, _ = self.capture("mouse-hover")
            with Image.open(candidate) as image:
                difference = ImageChops.difference(before.crop(region), image.convert("RGB").crop(region))
                # Pillow 12.3 renamed the iterator; older installations still
                # expose getdata without the new method.
                pixels = (difference.get_flattened_data() if hasattr(difference, "get_flattened_data")
                          else difference.getdata())
                changed = sum(max(pixel) > 20 for pixel in pixels)
                fraction = changed / (difference.width * difference.height)
            if fraction > 0.15:
                self.hover_transform = (sx * factor, sy * factor)
                self.action("calibrated pointer using visible Refresh hover", factor=factor,
                            transform=self.hover_transform, changed_fraction=round(fraction, 3))
                return
        raise RuntimeError("Cannot calibrate this display's native mouse coordinates; no click was sent")

    def click(self, label):
        self.wait_selected("before-click-" + label.replace(" ", "-"))
        if not self.hover_transform:
            self.calibrate_mouse()
        path, _ = self.capture("locate-" + label.replace(" ", "-"))
        box = self.text_box(label, path)
        x = round((box[0] + box[2]) / 2 * self.hover_transform[0])
        y = round((box[1] + box[3]) / 2 * self.hover_transform[1])
        self.check_focus()
        self.action("native click on OCR control", label=label, screenshot=path.name, box=box, point=[x, y])
        command(["xdotool", "mousemove", "--window", self.window, str(x), str(y)], self.env)
        self.check_focus()
        command(["xdotool", "click", "1"], self.env)

    def wait_state(self, status, timeout=20):
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            info = self.inspect()
            if info["State"]["Status"] == status:
                return info
            time.sleep(0.2)
        raise RuntimeError(f"Owned fixture did not reach {status}")

    def drive(self):
        self.action("begin mapped feature", feature=self.feature)
        if self.feature == "terminal":
            self.key("Return")
            self.wait_text("Connected", "03-shell-open")
            marker = "FERRYDOCK TERMINAL VERIFIED " + self.project.rsplit("-", 1)[-1]
            target = "/tmp/" + self.project + ".txt"
            size_file = "/tmp/" + self.project + "-size.txt"
            absent = command(["docker", "exec", self.container, "test", "!", "-e", target], self.env, check=False)
            if absent.returncode:
                raise RuntimeError("Proof file already exists before native typing")
            self.save("side-effect-before.json", {"path": target, "exists": False})
            shell_command = f"printf '%s\\n' '{marker}' | tee {target}; stty size | tee {size_file}"
            self.type(shell_command)
            self.key("Return")
            self.wait_text(marker, "04-terminal-result")
            deadline = time.monotonic() + 10
            while time.monotonic() < deadline:
                result = command(["docker", "exec", self.container, "cat", target], self.env, check=False)
                if result.returncode == 0 and result.stdout.strip() == marker:
                    break
                time.sleep(0.2)
            else:
                raise RuntimeError("Native shell path did not create the expected fixture file")
            dimensions = command(["docker", "exec", self.container, "cat", size_file], self.env).stdout.strip()
            rows, columns = map(int, dimensions.split())
            if rows < 1 or columns < 2:
                raise RuntimeError("Invalid PTY dimensions")
            self.save("side-effect-after.json", {"path": target, "contents": result.stdout,
                                                "pty_rows": rows, "pty_columns": columns})
            self.action("confirmed real fixture side effect", path=target, marker=marker,
                        pty_rows=rows, pty_columns=columns)
            # One occurrence can be echoed command input. Require the separate
            # output line too, after the independent execution observation.
            self.wait_text(marker, "04-confirmed-terminal-output", occurrences=2)
            self.type("exit")
            self.key("Return")
            self.wait_text("Shell exited", "05-shell-exited")
        elif self.feature == "containers":
            self.key("ctrl+f", "ctrl+a")
            self.type("no-match-" + self.project)
            self.wait_text("No containers match your search", "03-no-matches")
            self.key("ctrl+a")
            self.type(self.project)
            self.key("Escape", "Down")
            self.wait_selected("04-search-restored")
            self.click("Running only")
            self.wait_text("All containers", "05-all-filter")
        elif self.feature == "logs":
            self.key("ctrl+1")
            self.wait_text("FERRY_FIXTURE_READY", "03-log-output")
            self.wait_text("FERRY_HEARTBEAT", "04-live-log")
            self.click("Following")
            self.wait_text("Follow paused", "05-follow-paused")
            self.click("Follow paused")
            self.wait_text("Following", "06-follow-resumed")
        elif self.feature == "inspect":
            self.key("ctrl+3")
            self.wait_text(self.container, "03-inspect-id")
            self.wait_text("Config", "04-inspect-config")
            self.save("inspection-observed.json", {"id": self.container, "project": self.project})
        elif self.feature == "controls":
            self.click("Running only")
            self.wait_text("All containers", "03-show-stopped-enabled")
            self.click("Stop")
            self.wait_state("exited")
            self.wait_text("Start", "04-stopped")
            self.click("Start")
            self.wait_state("running")
            self.wait_text("Stop", "05-started")
            before = self.inspect()["State"]["StartedAt"]
            self.click("Restart")
            deadline = time.monotonic() + 20
            while time.monotonic() < deadline:
                info = self.inspect()
                if info["State"]["Status"] == "running" and info["State"]["StartedAt"] != before:
                    break
                time.sleep(0.2)
            else:
                raise RuntimeError("Restart did not change the fixture's StartedAt")
            self.capture("06-restarted")
            self.save("control-side-effects.json", {"id": self.container, "state": "running",
                                                    "started_before_restart": before,
                                                    "started_after_restart": info["State"]["StartedAt"]})
        self.action("mapped feature completed", feature=self.feature)

    def cleanup(self):
        errors = []
        app_exit = None
        if self.process:
            if self.process.poll() is None:
                self.process.terminate()
                try:
                    self.process.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    self.process.kill()
                    self.process.wait()
            app_exit = self.process.returncode
        if self.app_log:
            self.app_log.close()
        if self.fixture_attempted:
            try:
                current = self.project_ids()
                # On an incomplete up, there may be a partial probe. Check its
                # labels before scoped down; never remove a foreign orphan.
                for container in current:
                    info = json.loads(command(["docker", "inspect", container], self.env).stdout)[0]
                    labels = info.get("Config", {}).get("Labels", {})
                    if labels.get("dev.ferrydock.fixture") != "true" or labels.get("com.docker.compose.project") != self.project:
                        raise RuntimeError("Foreign container in fixture namespace; cleanup refused")
                    if self.created_ids and container not in self.created_ids:
                        raise RuntimeError("Unexpected container appeared in fixture namespace; cleanup refused")
                result = command(self.compose + ["down", "--volumes", "--remove-orphans"], self.env, timeout=45)
                (self.evidence / "fixture-cleanup.log").write_text(result.stdout + result.stderr)
                if self.project_ids():
                    raise RuntimeError("Fixture containers remain after cleanup")
                networks = command(["docker", "network", "ls", "-q", "--filter",
                                    f"label=com.docker.compose.project={self.project}"], self.env).stdout.split()
                if networks:
                    raise RuntimeError("Fixture networks remain after cleanup")
            except Exception as error:
                errors.append(str(error))
        self.save("cleanup.json", {"success": not errors, "errors": errors,
                                   "app_pid": self.process.pid if self.process else None,
                                   "app_exit_code": app_exit, "project": self.project,
                                   "evidence_png_count": len(list(self.evidence.glob("*.png")))})
        if errors:
            raise RuntimeError("; ".join(errors))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=("doctor", "run"))
    parser.add_argument("--display", default=os.environ.get("DISPLAY"))
    parser.add_argument("--binary", type=Path, default=ROOT / "target/release/ferrydock")
    parser.add_argument("--feature", choices=("containers", "logs", "terminal", "controls", "inspect"), default="terminal")
    parser.add_argument("--evidence", type=Path)
    parser.add_argument("--keep-runs", type=keep_runs, default=8,
                        help="Completed default-path runs to retain; 0 keeps all")
    args = parser.parse_args()
    env = os.environ.copy()
    # This harness finds and drives X11 windows; keep the override scoped to it.
    env.pop("WAYLAND_DISPLAY", None)
    if args.display:
        env["DISPLAY"] = args.display
    binary = args.binary.resolve()
    initial = base_doctor(binary, env)
    if args.command == "doctor":
        print(json.dumps(initial, indent=2))
        return 0 if initial["ready"] else 1
    timestamp = datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%SZ")
    evidence = (args.evidence or ROOT / ".verification/ferrydock" / (timestamp + "-" + uuid.uuid4().hex[:8])).resolve()
    with LOCK.open("a+") as lock:
        try:
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError:
            raise RuntimeError("Another FerryDock native verifier holds the desktop input lock")
        proof = Proof(binary, env, args.feature, evidence)
        proof.save("doctor-base.json", initial)
        error = None
        try:
            proof.launch()
            proof.drive()
        except Exception as failure:
            error = str(failure)
            if proof.window and proof.process and proof.process.poll() is None:
                try:
                    proof.capture("failure")
                except Exception:
                    pass
        finally:
            try:
                proof.cleanup()
            except Exception as failure:
                error = (error + "; " if error else "") + "cleanup: " + str(failure)
            proof.save("result.json", {"status": "failed" if error else "passed", "feature": args.feature,
                                       "error": error, "evidence": str(evidence), "project": proof.project,
                                       "custom_evidence": args.evidence is not None})
        summary = {"status": "failed" if error else "passed", "feature": args.feature,
                   "evidence": str(evidence), "error": error}
        if args.evidence is None:
            summary["retention"] = prune_runs(ROOT / ".verification/ferrydock", "ferrydock",
                                              args.keep_runs, current=evidence)
        print(json.dumps(summary, indent=2))
        return 1 if error else 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except Exception as error:
        print(json.dumps({"status": "blocked", "error": str(error)}), file=sys.stderr)
        sys.exit(1)
