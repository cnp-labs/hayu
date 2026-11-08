#!/usr/bin/env bash
# Shared isolation for live suites. Source, then call iso_init before any
# `hayu` or `hyprctl` use. Every hayu client/shell gets a private short
# XDG_RUNTIME_DIR under /tmp/hr-* with only wayland and hypr symlinked; the
# user's runtime, hayu socket and focus stay untouched unless a suite asks
# iso_capture_focus/iso_restore_focus around a deliberate change.
#
# Required by the caller: BIN (path to the hayu binary to test).
# Sets: REAL_RUNTIME SIG HYPR_PID WL USER_PID USER_SOCK USER_SOCK_INODE.
#
#   . tools/live/isolation.sh
#   iso_init
#   RT=$(iso_rt hr-in) || exit 1
#   iso_start "$RT" "$LOG" env XDG_CONFIG_HOME="$WORK/config" "$BIN" run
#   SHELL_PID=$ISO_PID
#   iso_hayu "$RT" launcher toggle
#   iso_destroy "$RT"
set -u

iso_die() { printf 'ISOLATION ABORT: %s\n' "$*" >&2; exit 1; }

iso_json() { python3 -c "$1" "${@:2}"; }

iso_init() {
    command -v hyprctl >/dev/null || iso_die "hyprctl is required"
    command -v python3 >/dev/null || iso_die "python3 is required"
    [ -n "${BIN:-}" ] || iso_die "set BIN before iso_init"

    REAL_RUNTIME=${XDG_RUNTIME_DIR:-/run/user/$(id -u)}
    # A parent wrapper may already have handed us a private runtime whose
    # `hypr` is a symlink; resolve back to the real one.
    if [ -L "$REAL_RUNTIME/hypr" ]; then
        REAL_RUNTIME=$(dirname "$(readlink -f "$REAL_RUNTIME/hypr")")
    fi

    local info
    info=$(XDG_RUNTIME_DIR="$REAL_RUNTIME" hyprctl instances -j 2>/dev/null) \
        || iso_die "hyprctl instances failed (runtime $REAL_RUNTIME)"
    SIG= HYPR_PID= WL=
    read -r SIG HYPR_PID WL <<EOF || true
$(printf '%s' "$info" | iso_json '
import json, sys
inst = json.load(sys.stdin)
if len(inst) != 1:
    sys.exit("need exactly one Hyprland instance, got %d" % len(inst))
i = inst[0]
print(i["instance"], i["pid"], i["wl_socket"])
')
EOF
    [ -n "${SIG:-}" ] && [ -n "${HYPR_PID:-}" ] && [ -n "${WL:-}" ] \
        || iso_die "could not read the live Hyprland instance"

    # The user's shell is the hayu child of the live compositor, discovered
    # rather than assumed. A normal session with no shell yet is allowed: then
    # no user socket may exist. Guard the socket inode so no suite can target it.
    local find_user
    find_user=$(pgrep -P "$HYPR_PID" -x hayu 2>/dev/null | wc -l)
    [ "$find_user" -le 1 ] || iso_die "more than one user hayu shell under $HYPR_PID"
    USER_PID=
    USER_SOCK_INODE=
    USER_SOCK=$REAL_RUNTIME/hayu/$SIG/control.sock
    if [ "$find_user" -eq 1 ]; then
        USER_PID=$(pgrep -P "$HYPR_PID" -x hayu 2>/dev/null | head -1)
        [ -S "$USER_SOCK" ] || iso_die "user hayu socket missing: $USER_SOCK"
        USER_SOCK_INODE=$(stat -c %i "$USER_SOCK") || iso_die "cannot stat $USER_SOCK"
    else
        [ ! -e "$USER_SOCK" ] || iso_die "socket exists without an identifiable user shell: $USER_SOCK"
    fi

    printf 'ISOLATION live=%s hypr_pid=%s wl=%s user_pid=%s user_inode=%s\n' \
        "$SIG" "$HYPR_PID" "$WL" "${USER_PID:-none}" "${USER_SOCK_INODE:-none}"
}

# Fresh private runtime; echo its path. Only wayland + its lock + hypr are
# symlinked. Never the `hayu` directory, so the user socket cannot be reused.
iso_rt() {
    local prefix=${1:-hr} rt sock
    rt=$(mktemp -d "/tmp/${prefix}-XXXXXX") || return 1
    chmod 700 "$rt" || { rm -rf "$rt"; return 1; }
    ln -s "$REAL_RUNTIME/$WL" "$rt/$WL" || { rm -rf "$rt"; return 1; }
    [ -e "$REAL_RUNTIME/$WL.lock" ] && ln -s "$REAL_RUNTIME/$WL.lock" "$rt/$WL.lock"
    ln -s "$REAL_RUNTIME/hypr" "$rt/hypr" || { rm -rf "$rt"; return 1; }
    mkdir -p "$rt/home" "$rt/config" "$rt/data" "$rt/state" "$rt/cache" || return 1
    sock=$rt/hayu/$SIG/control.sock
    if [ "${#sock}" -ge 108 ]; then
        printf 'iso_rt: control socket path too long: %s\n' "$sock" >&2
        rm -rf "$rt"
        return 1
    fi
    printf '%s\n' "$rt"
}

iso_hyprctl() {
    XDG_RUNTIME_DIR="$REAL_RUNTIME" HYPRLAND_INSTANCE_SIGNATURE="$SIG" hyprctl "$@"
}

iso_hayu() {
    local rt=$1; shift
    XDG_RUNTIME_DIR="$rt" HYPRLAND_INSTANCE_SIGNATURE="${SIG:-}" \
        WAYLAND_DISPLAY="${WL:-}" "$BIN" "$@"
}

# Background start inside the private runtime. Sets ISO_PID (the caller's
# child, so `wait "$ISO_PID"` works). Extra env belongs in the command:
#   iso_start "$RT" "$LOG" env XDG_CONFIG_HOME="$WORK/config" "$BIN" run
iso_start() {
    local rt=$1 log=$2; shift 2
    env XDG_RUNTIME_DIR="$rt" HYPRLAND_INSTANCE_SIGNATURE="$SIG" \
        WAYLAND_DISPLAY="$WL" HOME="$rt/home" "$@" >"$log" 2>&1 &
    ISO_PID=$!
}

iso_pids() {
    local rt=$1 p
    for p in /proc/[0-9]*; do
        p=${p#/proc/}
        [ -r "/proc/$p/environ" ] || continue
        { tr '\0' '\n' <"/proc/$p/environ"; } 2>/dev/null \
            | grep -qxF "XDG_RUNTIME_DIR=$rt" && printf '%s\n' "$p"
    done
}

iso_assert_env() { # pid rt
    { tr '\0' '\n' <"/proc/$1/environ"; } 2>/dev/null | grep -qxF "XDG_RUNTIME_DIR=$2"
}

# Quit through the private socket, then kill only PIDs whose environ carries
# this exact runtime. No name-wide kill ever.
iso_stop() {
    local rt=$1 p
    iso_hayu "$rt" quit >/dev/null 2>&1 || true
    sleep 0.5
    for p in $(iso_pids "$rt"); do kill "$p" 2>/dev/null || true; done
    sleep 0.2
    for p in $(iso_pids "$rt"); do kill -9 "$p" 2>/dev/null || true; done
    return 0
}

iso_destroy() {
    local rt=$1
    [ -n "$rt" ] && [ -d "$rt" ] || return 0
    iso_stop "$rt"
    rm -rf "$rt"
}

iso_user_ok() {
    if [ -z "${USER_PID:-}" ]; then
        # No user shell existed at init; a socket must not have appeared.
        [ ! -e "$USER_SOCK" ]
        return
    fi
    kill -0 "$USER_PID" 2>/dev/null || return 1
    [ "$(stat -c %i "$USER_SOCK" 2>/dev/null)" = "$USER_SOCK_INODE" ]
}

# PID-filtered layer helpers: both shells use namespace `hayu`; counting the
# namespace alone sees the user bar.
iso_layer_count() { # pid namespace
    iso_hyprctl layers -j 2>/dev/null | iso_json '
import json, sys
d = json.load(sys.stdin)
pid, ns = int(sys.argv[1]), sys.argv[2]
print(sum(1 for v in d.values() for ls in v.get("levels", {}).values()
          for l in ls if l.get("pid") == pid and l.get("namespace") == ns))
' "$1" "$2"
}

iso_layer_rect() { # pid namespace -> "x y w h"
    iso_hyprctl layers -j 2>/dev/null | iso_json '
import json, sys
d = json.load(sys.stdin)
pid, ns = int(sys.argv[1]), sys.argv[2]
for v in d.values():
    for ls in v.get("levels", {}).values():
        for l in ls:
            if l.get("pid") == pid and l.get("namespace") == ns:
                print(l["x"], l["y"], l["w"], l["h"])
                raise SystemExit
raise SystemExit(1)
' "$1" "$2"
}

iso_state_ws() {
    iso_hyprctl -j activeworkspace | iso_json 'import json,sys; print(json.load(sys.stdin)["id"])'
}

iso_state_mon() {
    iso_hyprctl -j monitors | iso_json '
import json, sys
for m in json.load(sys.stdin):
    if m["focused"]:
        print(m["name"]); break
'
}

iso_state_win() {
    iso_hyprctl -j activewindow | iso_json 'import json,sys; print(json.load(sys.stdin).get("address") or "")'
}

iso_cursor_pos() {
    iso_hyprctl cursorpos 2>/dev/null || true
}

# Hyprland may answer a successful Lua command with `table: 0x...` (or exit 0
# with a rejection). Never trust reply text: apply, then verify compositor state.
iso_focus_ws() {
    local id=$1
    [ "$(iso_state_ws)" = "$id" ] && return 0
    iso_hyprctl dispatch workspace "$id" >/dev/null 2>&1
    [ "$(iso_state_ws)" = "$id" ] && return 0
    iso_hyprctl repl "hl.dispatch(hl.dsp.focus({ workspace = $id }))" >/dev/null 2>&1
    [ "$(iso_state_ws)" = "$id" ]
}

iso_focus_mon() {
    local name=$1
    [ "$(iso_state_mon)" = "$name" ] && return 0
    iso_hyprctl dispatch focusmonitor "$name" >/dev/null 2>&1
    [ "$(iso_state_mon)" = "$name" ]
}

iso_focus_win() {
    local addr=$1
    [ "$(iso_state_win)" = "$addr" ] && return 0
    iso_hyprctl dispatch focuswindow "address:$addr" >/dev/null 2>&1
    [ "$(iso_state_win)" = "$addr" ]
}

iso_move_cursor() {
    local x=$1 y=$2 got
    iso_hyprctl dispatch movecursor "$x" "$y" >/dev/null 2>&1
    got=$(iso_cursor_pos)
    python3 - "$x" "$y" "$got" <<'PY'
import sys
try:
    gx, gy = (float(v.strip()) for v in sys.argv[3].split(",", 1))
    ok = abs(gx - float(sys.argv[1])) <= 1 and abs(gy - float(sys.argv[2])) <= 1
except Exception:
    ok = False
raise SystemExit(0 if ok else 1)
PY
}

iso_capture_focus() {
    ORIG_WS=$(iso_state_ws)
    ORIG_MON=$(iso_state_mon)
    ORIG_WIN=$(iso_state_win)
    ORIG_CURSOR=$(iso_cursor_pos)
    printf 'ISOLATION focus before ws=%s mon=%s win=%s cursor=%s\n' \
        "$ORIG_WS" "$ORIG_MON" "$ORIG_WIN" "${ORIG_CURSOR:-}"
}

iso_restore_focus() {
    local cx cy
    if [ -n "${ORIG_MON:-}" ]; then
        iso_focus_mon "$ORIG_MON" \
            || printf 'ISOLATION WARN: focus monitor %s not confirmed\n' "$ORIG_MON" >&2
    fi
    if [ -n "${ORIG_WS:-}" ]; then
        iso_focus_ws "$ORIG_WS" \
            || printf 'ISOLATION WARN: workspace %s not confirmed\n' "$ORIG_WS" >&2
    fi
    if [ -n "${ORIG_WIN:-}" ]; then
        iso_focus_win "$ORIG_WIN" \
            || printf 'ISOLATION WARN: window %s not confirmed\n' "$ORIG_WIN" >&2
    fi
    if [ -n "${ORIG_CURSOR:-}" ]; then
        cx=${ORIG_CURSOR%%,*} cy=${ORIG_CURSOR#*,}
        cy=${cy// /}
        iso_move_cursor "$cx" "$cy" \
            || printf 'ISOLATION WARN: cursor %s not confirmed\n' "$ORIG_CURSOR" >&2
    fi
    printf 'ISOLATION focus restored ws=%s mon=%s win=%s cursor=%s\n' \
        "${ORIG_WS:-}" "${ORIG_MON:-}" "${ORIG_WIN:-}" "${ORIG_CURSOR:-}"
}

# Persist a log outside the throwaway WORK directory.
iso_evidence() { # file label
    local dir=${HAYU_LIVE_EVIDENCE:-/tmp/opencode/hayu-lifecycle/live}
    [ -f "$1" ] || return 0
    mkdir -p "$dir" || return 1
    cp "$1" "$dir/$2-$(date +%Y%m%d-%H%M%S)-$$.log"
}
