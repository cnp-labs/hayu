#!/usr/bin/python3
"""Live AT-SPI checks for Hayu launcher; logs evidence to /tmp/opencode."""
import json
import os
import pathlib
import re
import shutil
import signal
import subprocess
import sys
import tempfile
import time

import gi
gi.require_version("Atspi", "2.0")
from gi.repository import Atspi

ROOT = pathlib.Path(__file__).resolve().parents[2]
BIN = pathlib.Path(os.environ.get("HAYU_BIN") or ROOT / "target/release/hayu")
KEYIN = pathlib.Path(os.environ.get("KEYIN_BIN") or ROOT / "tools/live/keyin/target/release/keyin")
BUS_DEST = "org.a11y.Bus"
BUS_PATH = "/org/a11y/bus"
LOG_DIR = pathlib.Path("/tmp/opencode/hayu-lifecycle/live")
LOG = LOG_DIR / f"a11y-suite-{os.getpid()}.log"

def log(*args):
    print(*args, flush=True)

def live_instance():
    """Resolve the live Hyprland instance, not the possibly stale env signature."""
    real = pathlib.Path(os.environ.get("XDG_RUNTIME_DIR") or f"/run/user/{os.getuid()}")
    if (real / "hypr").is_symlink():
        real = (real / "hypr").resolve().parent
    out = subprocess.check_output(
        ["hyprctl", "instances", "-j"],
        env={**os.environ, "XDG_RUNTIME_DIR": str(real)}, text=True)
    inst = json.loads(out)
    if len(inst) != 1:
        raise RuntimeError(f"need exactly one Hyprland instance, got {len(inst)}")
    return real, inst[0]["instance"], inst[0]["wl_socket"]

def status():
    out = subprocess.check_output([
        "gdbus", "call", "--session", "--dest", BUS_DEST,
        "--object-path", BUS_PATH, "--method",
        "org.freedesktop.DBus.Properties.Get", "org.a11y.Status", "IsEnabled"
    ], text=True, timeout=5).strip()
    return out, "true" in out.lower()

def set_enabled(value):
    subprocess.run(["gdbus", "call", "--session", "--dest", BUS_DEST,
        "--object-path", BUS_PATH, "--method", "org.freedesktop.DBus.Properties.Set",
        "org.a11y.Status", "IsEnabled", f"<{str(value).lower()}>"],
        check=True, text=True, capture_output=True, timeout=5)

def at_address():
    p = subprocess.run([
        "gdbus", "call", "--session", "--dest", BUS_DEST,
        "--object-path", BUS_PATH, "--method", "org.a11y.Bus.GetAddress"
    ], text=True, capture_output=True, timeout=5)
    if p.returncode:
        return None, p.stderr.strip()
    match = re.match(r"\('([^']+)',\)", p.stdout.strip())
    if not match:
        return None, f"unexpected GetAddress reply: {p.stdout.strip()}"
    return match.group(1), ""

def unix_path(address):
    if not address.startswith("unix:path="):
        return None
    return address[len("unix:path="):].split(",", 1)[0]

def at_bus_ok():
    """Check the user's AT-SPI bus; GetAddress also activates org.a11y.Bus."""
    address, error = None, ""
    for _ in range(10):
        address, error = at_address()
        if not address:
            time.sleep(0.2)
            continue
        path = unix_path(address)
        if path is not None and not os.path.exists(path):
            error = f"{address}: socket path missing"
            time.sleep(0.2)
            continue
        break
    if not address:
        return False, error or "no address"
    path = unix_path(address)
    if path is not None and not os.path.exists(path):
        return False, f"{address}: socket path missing (stale AT-SPI bus)"
    listed = subprocess.run([
        "gdbus", "call", "--address", address, "--dest", "org.freedesktop.DBus",
        "--object-path", "/org/freedesktop/DBus",
        "--method", "org.freedesktop.DBus.ListNames"
    ], text=True, capture_output=True, timeout=5)
    if listed.returncode:
        return False, f"{address}: {listed.stderr.strip()}"
    if "org.a11y.atspi.Registry" not in listed.stdout:
        return False, f"{address}: org.a11y.atspi.Registry absent"
    return True, address

def command(args, env, check=True):
    p = subprocess.run([str(BIN), *args], cwd=ROOT, env=env, text=True, capture_output=True)
    log(f"COMMAND {' '.join(args)} exit={p.returncode}")
    if p.stdout.strip(): log("stdout:", p.stdout.strip())
    if p.stderr.strip(): log("stderr:", p.stderr.strip())
    if check and p.returncode:
        raise RuntimeError(f"hayu {args} failed ({p.returncode})")
    return p.returncode

def proc_env(pid):
    try:
        with open(f"/proc/{pid}/environ", "rb") as handle:
            raw = handle.read()
    except OSError:
        return {}
    env = {}
    for entry in raw.split(b"\0"):
        if b"=" in entry:
            key, value = entry.split(b"=", 1)
            env[key.decode(errors="replace")] = value.decode(errors="replace")
    return env

def wait_private_env(proc, runtime, wanted, timeout=5):
    """Popen can return before exec; poll our own child until it carries the
    private environment, and show exactly what was observed for evidence."""
    want = str(runtime)
    seen = {}
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        env = proc_env(proc.pid)
        seen = {k: v for k, v in env.items()
                if k.startswith("XDG_") or k in ("HOME", "HYPRLAND_INSTANCE_SIGNATURE", "WAYLAND_DISPLAY")}
        if env.get("XDG_RUNTIME_DIR") != want:
            if proc.poll() is not None:
                break
            time.sleep(.05)
            continue
        bad = [f"{k}={env.get(k)!r} expected {v!r}" for k, v in wanted.items() if env.get(k) != v]
        return not bad, seen, bad
    return False, seen, [f"XDG_RUNTIME_DIR={seen.get('XDG_RUNTIME_DIR')!r} expected {want!r}"]

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
    if not BIN.is_file() or not KEYIN.is_file():
        raise SystemExit(f"missing binary: {BIN} or {KEYIN}")
    LOG_DIR.mkdir(parents=True, exist_ok=True)
    real, session, wl = live_instance()
    initial, was_enabled = status()
    user_sock = real / "hayu" / session / "control.sock"
    user_inode = user_sock.stat().st_ino if user_sock.exists() else None
    with LOG.open("w", buffering=1) as stream:
        original_stdout, original_stderr = sys.stdout, sys.stderr
        sys.stdout = sys.stderr = stream
        log("cwd:", os.getcwd(), "script root:", ROOT, "python:", sys.executable)
        log("live signature:", session, "wayland:", wl, "real runtime:", real)
        log("user shell socket:", user_sock, "inode:", user_inode)
        log("AT-SPI initial IsEnabled:", initial)

        work = pathlib.Path(tempfile.mkdtemp(prefix=f"a11y-work-{os.getpid()}-", dir=LOG_DIR))
        # Wayland/Hyprland paths are symlinked, but the control socket lives in
        # this directory: keep it short enough for SUN_LEN (108 bytes).
        runtime = pathlib.Path(tempfile.mkdtemp(prefix="hr-", dir="/tmp"))
        for name in ("home", "config", "data", "state", "cache"):
            (runtime / name).mkdir()
        state = {"proc": None, "env": None, "restored": was_enabled, "user_sock_ok": True}

        def restore_at(where):
            if state["restored"]:
                try:
                    log(f"AT-SPI IsEnabled left unchanged ({where}):", status()[0])
                except Exception as exc:
                    log(f"WARNING: cannot re-read AT-SPI IsEnabled ({where}):", exc)
                return
            state["restored"] = True
            try:
                set_enabled(False)
                now = status()[0]
                log(f"AT-SPI restored IsEnabled ({where}):", now, "expected:", initial)
                if now != initial:
                    log("WARNING: AT-SPI IsEnabled did not read back as captured")
            except Exception as exc:
                log(f"WARNING: AT-SPI restore failed ({where}):", exc)

        def cleanup(signum=None):
            proc = state["proc"]
            if proc is not None and proc.poll() is None:
                subprocess.run([str(BIN), "quit"], cwd=ROOT, env=state["env"],
                               stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
                try: proc.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    proc.terminate()
                    try: proc.wait(timeout=3)
                    except subprocess.TimeoutExpired: proc.kill(); proc.wait()
            restore_at("signal" if signum is not None else "finally")
            shutil.rmtree(runtime, ignore_errors=True)
            if user_sock.exists():
                now_inode = user_sock.stat().st_ino
            else:
                now_inode = None
            state["user_sock_ok"] = now_inode == user_inode
            log("user shell socket after:", user_sock, "inode:", now_inode,
                "expected:", user_inode, "unchanged:", state["user_sock_ok"])
            log("work retained:", work)
            if signum is not None:
                stream.flush()
                os._exit(128 + signum)

        for signum in (signal.SIGINT, signal.SIGTERM):
            signal.signal(signum, lambda s, _f: cleanup(s))
        try:
            (runtime / wl).symlink_to(real / wl)
            if (real / f"{wl}.lock").exists():
                (runtime / f"{wl}.lock").symlink_to(real / f"{wl}.lock")
            (runtime / "hypr").symlink_to(real / "hypr")
            session_sig = session
            control = runtime / "hayu" / session_sig / "control.sock"
            if control.exists():
                raise RuntimeError(f"refusing: existing Hayu control socket {control}")
            log("preflight isolated runtime:", runtime, "control absent:", not control.exists())
            config = work / "config/hayu"
            data = work / "data/applications"
            config.mkdir(parents=True)
            data.mkdir(parents=True)
            (config / "config.toml").write_text(
                'schema_version = 1\n\n[bar]\nposition = "bottom"\nheight = 32\n\n'
                '[tray]\nenabled = false\n')
            for name, label in (("a", "A11y Cedar"), ("b", "A11y Maple"), ("c", "A11y Birch")):
                (data / f"a11y-{name}.desktop").write_text(
                    f"[Desktop Entry]\nType=Application\nName={label}\nExec=/usr/bin/true\n")
            if not was_enabled:
                reachable, detail = at_bus_ok()
                log("AT-SPI bus check:", detail)
                if not reachable:
                    raise RuntimeError(
                        f"AT-SPI bus/registry not reachable ({detail}); not enabling it")
                set_enabled(True)
                log("AT-SPI enabled for this run:", status()[0])
            env = os.environ.copy()
            env.update(XDG_RUNTIME_DIR=str(runtime), XDG_CONFIG_HOME=str(work / "config"),
                       XDG_DATA_HOME=str(work / "data"), XDG_STATE_HOME=str(runtime / "state"),
                       XDG_CACHE_HOME=str(runtime / "cache"), HOME=str(runtime / "home"),
                       HYPRLAND_INSTANCE_SIGNATURE=session_sig, WAYLAND_DISPLAY=wl)
            state["env"] = env
            proc = subprocess.Popen([str(BIN), "run"], cwd=ROOT, env=env,
                                   stdout=stream, stderr=subprocess.STDOUT)
            state["proc"] = proc
            for _ in range(60):
                if proc.poll() is not None:
                    raise RuntimeError(f"hayu run exited {proc.returncode}")
                if command(["launcher", "close"], env, check=False) == 0:
                    break
                time.sleep(.15)
            else:
                raise RuntimeError("Hayu control socket never became available")
            # Popen may return before exec, so /proc/<pid>/environ is only
            # trustworthy after the private control socket answers. Poll our
            # own child and record the exact observed environment.
            wanted = {"XDG_RUNTIME_DIR": str(runtime), "XDG_CONFIG_HOME": str(work / "config"),
                      "XDG_DATA_HOME": str(work / "data"), "XDG_STATE_HOME": str(runtime / "state"),
                      "XDG_CACHE_HOME": str(runtime / "cache"), "HOME": str(runtime / "home"),
                      "HYPRLAND_INSTANCE_SIGNATURE": session_sig, "WAYLAND_DISPLAY": wl}
            env_ok, seen, bad = wait_private_env(proc, runtime, wanted)
            log("test shell env observed:", seen)
            if control.exists():
                log("private control socket:", control, "inode:", control.stat().st_ino,
                    "user socket:", user_sock, "user inode:", user_inode)
            if not env_ok:
                raise RuntimeError("hayu test shell environment mismatch: " + "; ".join(bad))
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
            cleanup()
            sys.stdout, sys.stderr = original_stdout, original_stderr
    if not state["user_sock_ok"]:
        raise SystemExit("user shell control socket changed")
    print(f"LOG {LOG}")

if __name__ == "__main__":
    main()
