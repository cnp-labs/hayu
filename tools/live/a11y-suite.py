#!/usr/bin/python3
"""Live AT-SPI checks for Hayu launcher; logs evidence to /tmp/opencode."""
import os
import pathlib
import shutil
import subprocess
import sys
import tempfile
import time

import gi
gi.require_version("Atspi", "2.0")
from gi.repository import Atspi

ROOT = pathlib.Path(__file__).resolve().parents[2]
BIN = ROOT / "target/release/hayu"
KEYIN = ROOT / "tools/live/keyin/target/release/keyin"
BUS_DEST = "org.a11y.Bus"
BUS_PATH = "/org/a11y/bus"
LOG = pathlib.Path(f"/tmp/opencode/hayu-a11y-suite-{os.getpid()}.log")

def log(*args):
    print(*args, flush=True)

def status():
    out = subprocess.check_output([
        "gdbus", "call", "--session", "--dest", BUS_DEST,
        "--object-path", BUS_PATH, "--method",
        "org.freedesktop.DBus.Properties.Get", "org.a11y.Status", "IsEnabled"
    ], text=True).strip()
    return out, "true" in out.lower()

def command(args, env, check=True):
    p = subprocess.run([str(BIN), *args], cwd=ROOT, env=env, text=True, capture_output=True)
    log(f"COMMAND {' '.join(args)} exit={p.returncode}")
    if p.stdout.strip(): log("stdout:", p.stdout.strip())
    if p.stderr.strip(): log("stderr:", p.stderr.strip())
    if check and p.returncode:
        raise RuntimeError(f"hayu {args} failed ({p.returncode})")
    return p.returncode

def walk(app):
    yield app
    for i in range(app.get_child_count()):
        yield from walk(app.get_child_at_index(i))

def get_text(node):
    """Text interface value, not the accessible name (AT-SPI `GetText`)."""
    try:
        count = Atspi.Text.get_character_count(node)
        return Atspi.Text.get_text(node, 0, count) if count > 0 else ""
    except Exception:
        return ""

def showing(node):
    try:
        return node.get_state_set().contains(Atspi.StateType.SHOWING)
    except Exception:
        return None

def snapshot(pid):
    rows = []
    desktop = Atspi.get_desktop(0)
    for i in range(desktop.get_child_count()):
        app = desktop.get_child_at_index(i)
        try:
            if app.get_process_id() != pid:
                continue
            for node in walk(app):
                try:
                    value = get_text(node)
                    selected = node.get_state_set().contains(Atspi.StateType.SELECTED)
                    rows.append({"name": node.get_name() or "", "role": node.get_role_name() or "",
                                 "value": value, "selected": selected,
                                 "showing": showing(node)})
                except Exception:
                    continue
        except Exception:
            continue
    return rows

def inspect(pid, label, predicate, expected, timeout=8):
    deadline = time.monotonic() + timeout
    rows = snapshot(pid)
    while time.monotonic() < deadline and not predicate(rows):
        time.sleep(.2)
        rows = snapshot(pid)
    log(f"OBSERVATION {label}")
    for row in rows:
        log(" ", row)
    if not predicate(rows):
        raise RuntimeError(f"state not observed: {label}; expected {expected}")
    return rows

def key(env, *args):
    p = subprocess.run([str(KEYIN), *args], cwd=ROOT, env=env)
    log(f"COMMAND keyin {' '.join(args)} exit={p.returncode}")
    if p.returncode:
        raise RuntimeError(f"keyin failed: {args}")

def main():
    LOG.parent.mkdir(parents=True, exist_ok=True)
    with LOG.open("w", buffering=1) as stream:
        original_stdout, original_stderr = sys.stdout, sys.stderr
        sys.stdout = sys.stderr = stream
        initial, was_enabled = status()
        log("cwd:", os.getcwd(), "script root:", ROOT, "python:", sys.executable)
        log("AT-SPI initial IsEnabled:", initial)
        work = pathlib.Path(tempfile.mkdtemp(prefix="hayu-a11y-", dir="/tmp/opencode"))
        # Wayland/Hyprland paths are symlinked, but the control socket lives in
        # this directory: keep it short enough for SUN_LEN (108 bytes).
        runtime = pathlib.Path(tempfile.mkdtemp(prefix="hr-", dir="/tmp"))
        proc = None
        try:
            # A private runtime dir keeps every client command off an existing
            # shell; symlinks preserve Wayland and Hyprland discovery.
            base_runtime = pathlib.Path(os.environ["XDG_RUNTIME_DIR"])
            for entry in base_runtime.iterdir():
                if entry.name.startswith("wayland-") or entry.name == "hypr":
                    (runtime / entry.name).symlink_to(entry)
            session = os.environ["HYPRLAND_INSTANCE_SIGNATURE"]
            control = runtime / "hayu" / session / "control.sock"
            if control.exists():
                raise RuntimeError(f"refusing: existing Hayu control socket {control}")
            log("preflight isolated runtime:", runtime, "control absent:", not control.exists())
            config = work / "config/hayu"
            data = work / "data/applications"
            config.mkdir(parents=True)
            data.mkdir(parents=True)
            (config / "config.toml").write_text('schema_version = 1\n\n[bar]\nposition = "top"\nheight = 32\n')
            for name, label in (("a", "A11y Cedar"), ("b", "A11y Maple"), ("c", "A11y Birch")):
                (data / f"a11y-{name}.desktop").write_text(
                    f"[Desktop Entry]\nType=Application\nName={label}\nExec=/usr/bin/true\n")
            if not was_enabled:
                subprocess.run(["gdbus", "call", "--session", "--dest", BUS_DEST,
                    "--object-path", BUS_PATH, "--method", "org.freedesktop.DBus.Properties.Set",
                    "org.a11y.Status", "IsEnabled", "<true>"], check=True, text=True, capture_output=True)
            env = os.environ.copy()
            env.update(XDG_RUNTIME_DIR=str(runtime), XDG_CONFIG_HOME=str(work / "config"),
                       XDG_DATA_HOME=str(work / "data"))
            proc = subprocess.Popen([str(BIN), "run"], cwd=ROOT, env=env,
                                   stdout=stream, stderr=subprocess.STDOUT)
            for _ in range(60):
                if proc.poll() is not None:
                    raise RuntimeError(f"hayu run exited {proc.returncode}")
                if command(["launcher", "close"], env, check=False) == 0:
                    break
                time.sleep(.15)
            else:
                raise RuntimeError("Hayu control socket never became available")
            pid = proc.pid
            log("hayu pid:", pid)
            inspect(pid, "initial closed", lambda r: not any(x["name"] in ("Search applications", "Results") for x in r),
                    "no search/list children")
            command(["launcher", "open"], env)
            rows = inspect(pid, "opened", lambda r: any(x["name"] == "Search applications" for x in r),
                           "search field and results")
            names = [x["name"] for x in rows]
            if not {"A11y Cedar", "A11y Maple", "A11y Birch"}.issubset(names):
                raise RuntimeError("fixture result names missing")
            # Plain wl_keyboard typing needs no bound IME; text-input only added
            # inline composition on top of this path.
            key(env, "type:A11y")
            rows = inspect(pid, "query value", lambda r: any(
                x["name"] == "Search applications" and x["value"] == "A11y" for x in r)
                and sum(x["role"] == "list item" for x in r) >= 2,
                "entry value 'A11y' and at least two matching items")
            log("QUERY value:", next(x["value"] for x in rows if x["name"] == "Search applications"))
            before = next((x["name"] for x in rows if x["role"] == "list item" and x["selected"]), None)
            key(env, "press:Down")
            rows = inspect(pid, "Down selection", lambda r: any(x["role"] == "list item" and x["selected"] for x in r),
                           "selected result after Down")
            after = next((x["name"] for x in rows if x["role"] == "list item" and x["selected"]), None)
            if not before or not after or before == after:
                raise RuntimeError(f"Down did not change selected name: before={before!r}, after={after!r}")
            log(f"SELECTION changed: {before!r} -> {after!r}")
            key(env, "type: B")
            rows = inspect(pid, "filtered query", lambda r: any(
                x["name"] == "Search applications" and x["value"] == "A11y B" for x in r)
                and [x["name"] for x in r if x["role"] == "list item"] == ["A11y Birch"],
                "entry value 'A11y B' and one filtered item")
            log("QUERY value:", next(x["value"] for x in rows if x["name"] == "Search applications"))
            command(["launcher", "close"], env)
            inspect(pid, "closed after query", lambda r: not any(x["name"] in ("Search applications", "Results") for x in r),
                    "no search/list children")
            command(["launcher", "open"], env)
            rows = inspect(pid, "reopened reset", lambda r: any(
                x["name"] == "Search applications" and x["value"] == "" for x in r)
                and any(x["name"] == "A11y Birch" and x["selected"] for x in r),
                "empty query value, reset selection and fixture results")
            log("LIMIT: no Orca installed, so this checks the AT-SPI tree only, not speech output.")
        finally:
            if proc is not None and proc.poll() is None:
                subprocess.run([str(BIN), "quit"], cwd=ROOT, env=env,
                               stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
                try: proc.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    proc.terminate()
                    try: proc.wait(timeout=3)
                    except subprocess.TimeoutExpired: proc.kill(); proc.wait()
            if not was_enabled:
                subprocess.run(["gdbus", "call", "--session", "--dest", BUS_DEST,
                    "--object-path", BUS_PATH, "--method", "org.freedesktop.DBus.Properties.Set",
                    "org.a11y.Status", "IsEnabled", "<false>"], check=True, text=True, capture_output=True)
            shutil.rmtree(runtime, ignore_errors=True)
            shutil.rmtree(work, ignore_errors=True)
            log("AT-SPI restored IsEnabled:", status()[0])
            sys.stdout, sys.stderr = original_stdout, original_stderr
    print(f"LOG {LOG}")

if __name__ == "__main__":
    main()
