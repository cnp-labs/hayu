//! Throwaway input injector for the live checks in `tools/live`.
//!
//! It binds `zwp_virtual_keyboard_v1` on the first seat, uploads an xkb keymap,
//! and sends raw evdev keycodes. Arguments are processed in order:
//!
//! ```text
//! keyin type:hayu test          # printable text, press/release per character
//! keyin press:Down              # press and release a named key
//! keyin hold:BackSpace          # one press held long enough for the compositor to repeat
//! keyin raw:44                  # evdev keycode 28..=255, press and release
//! ```
//!
//! `KEYIN_LAYOUT` selects the uploaded keymap (default `us`). `type:` maps
//! characters through a fixed `us` table, so it is layout-independent by
//! design: the check sends the same physical keys either way. `raw:` is what a
//! layout switch test needs, because the character then comes from the
//! uploaded keymap instead of this table.
use std::io::Write;
use std::os::fd::AsFd;
use std::time::{Duration, SystemTime};

use wayland_client::globals::{registry_queue_init, GlobalListContents};
use wayland_client::protocol::{wl_registry, wl_seat};
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle};
use wayland_protocols_misc::zwp_virtual_keyboard_v1::client::{
    zwp_virtual_keyboard_manager_v1::ZwpVirtualKeyboardManagerV1,
    zwp_virtual_keyboard_v1::ZwpVirtualKeyboardV1,
};

struct State;

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for State {
    fn event(
        _: &mut Self,
        _: &wl_registry::WlRegistry,
        _: <wl_registry::WlRegistry as Proxy>::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<wl_seat::WlSeat, ()> for State {
    fn event(
        _: &mut Self,
        _: &wl_seat::WlSeat,
        _: <wl_seat::WlSeat as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ZwpVirtualKeyboardManagerV1, ()> for State {
    fn event(
        _: &mut Self,
        _: &ZwpVirtualKeyboardManagerV1,
        _: <ZwpVirtualKeyboardManagerV1 as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ZwpVirtualKeyboardV1, ()> for State {
    fn event(
        _: &mut Self,
        _: &ZwpVirtualKeyboardV1,
        _: <ZwpVirtualKeyboardV1 as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

fn usage() -> ! {
    eprintln!(
        "usage: keyin type:<text> | press:<Name> | hold:<Name> | raw:<evdev>\n\
         names: Return Escape BackSpace Tab Delete Up Down Left Right Home End\n\
         env: KEYIN_LAYOUT (default us), KEYIN_STEP_MS (default 20)"
    );
    std::process::exit(2)
}

/// `us` evdev keycode plus whether Shift is needed.
fn us_key(c: char) -> Option<(u32, bool)> {
    let key = match c {
        'a'..='z' => {
            let row = [
                30, 48, 46, 32, 18, 33, 34, 35, 23, 36, 37, 38, 50, 49, 24, 25, 16, 19, 31, 20,
                22, 47, 17, 45, 21, 44,
            ];
            return Some((row[(c as u8 - b'a') as usize], false));
        }
        'A'..='Z' => return us_key(c.to_ascii_lowercase()).map(|(key, _)| (key, true)),
        '1'..='9' => (c as u32 - '1' as u32) + 2,
        '0' => 11,
        ' ' => 57,
        '-' => 12,
        '=' => 13,
        ',' => 51,
        '.' => 52,
        '/' => 53,
        ';' => 39,
        '\'' => 40,
        '[' => 26,
        ']' => 27,
        '\\' => 43,
        '`' => 41,
        '!' => return Some((2, true)),
        '@' => return Some((3, true)),
        '#' => return Some((4, true)),
        '$' => return Some((5, true)),
        '%' => return Some((6, true)),
        '^' => return Some((7, true)),
        '&' => return Some((8, true)),
        '*' => return Some((9, true)),
        '(' => return Some((10, true)),
        ')' => return Some((11, true)),
        '_' => return Some((12, true)),
        '+' => return Some((13, true)),
        '{' => return Some((26, true)),
        '}' => return Some((27, true)),
        '|' => return Some((43, true)),
        '~' => return Some((41, true)),
        ':' => return Some((39, true)),
        '"' => return Some((40, true)),
        '<' => return Some((51, true)),
        '>' => return Some((52, true)),
        '?' => return Some((53, true)),
        _ => return None,
    };
    Some((key, false))
}

fn named_key(name: &str) -> Option<u32> {
    Some(match name {
        "Return" | "Enter" => 28,
        "Escape" => 1,
        "BackSpace" => 14,
        "Tab" => 15,
        "Delete" => 111,
        "Up" => 103,
        "Down" => 108,
        "Left" => 105,
        "Right" => 106,
        "Home" => 102,
        "End" => 107,
        _ => return None,
    })
}

fn keymap_text(layout: &str) -> Result<String, String> {
    let context = xkbcommon::xkb::Context::new(xkbcommon::xkb::CONTEXT_NO_FLAGS);
    let keymap = xkbcommon::xkb::Keymap::new_from_names(
        &context,
        "",
        "",
        layout,
        "",
        None,
        xkbcommon::xkb::KEYMAP_COMPILE_NO_FLAGS,
    )
    .ok_or_else(|| format!("cannot compile a `{layout}` keymap; is libxkbcommon-data installed?"))?;
    Ok(keymap.get_as_string(xkbcommon::xkb::KEYMAP_FORMAT_TEXT_V1))
}

fn upload_keymap(
    conn: &Connection,
    keyboard: &ZwpVirtualKeyboardV1,
    layout: &str,
) -> Result<(), String> {
    let text = keymap_text(layout)?;
    // A regular temp file is enough: the compositor only needs to mmap the fd.
    let path = std::env::temp_dir().join(format!("keyin-keymap-{}", std::process::id()));
    {
        let mut file = std::fs::File::create(&path)
            .map_err(|e| format!("cannot create {}: {e}", path.display()))?;
        file.write_all(text.as_bytes())
            .map_err(|e| format!("cannot write keymap: {e}"))?;
    }
    let file = std::fs::File::open(&path).map_err(|e| format!("cannot reopen keymap: {e}"))?;
    keyboard.keymap(
        1, // XKB_V1
        file.as_fd(),
        text.len() as u32,
    );
    let _ = std::fs::remove_file(&path);
    conn.roundtrip()
        .map_err(|e| format!("wayland roundtrip failed: {e}"))?;
    Ok(())
}

fn now_ms() -> u32 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.subsec_millis() + (d.as_secs() as u32 & 0xffff) * 1000)
        .unwrap_or(0)
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() {
        usage();
    }
    let layout = std::env::var("KEYIN_LAYOUT").unwrap_or_else(|_| "us".into());
    let step = std::env::var("KEYIN_STEP_MS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(20);

    let conn = match Connection::connect_to_env() {
        Ok(conn) => conn,
        Err(error) => {
            eprintln!("keyin: cannot connect to the compositor: {error}");
            std::process::exit(1);
        }
    };
    let (globals, queue) = match registry_queue_init::<State>(&conn) {
        Ok(pair) => pair,
        Err(error) => {
            eprintln!("keyin: cannot read the registry: {error}");
            std::process::exit(1);
        }
    };
    let qh = queue.handle();
    let seat: wl_seat::WlSeat = match globals.bind(&qh, 1..=9, ()) {
        Ok(seat) => seat,
        Err(error) => {
            eprintln!("keyin: no wl_seat: {error}");
            std::process::exit(1);
        }
    };
    let manager: ZwpVirtualKeyboardManagerV1 = match globals.bind(&qh, 1..=1, ()) {
        Ok(manager) => manager,
        Err(error) => {
            eprintln!("keyin: compositor has no zwp_virtual_keyboard_manager_v1: {error}");
            std::process::exit(1);
        }
    };
    let keyboard = manager.create_virtual_keyboard(&seat, &qh, ());
    if let Err(error) = upload_keymap(&conn, &keyboard, &layout) {
        eprintln!("keyin: {error}");
        std::process::exit(1);
    }

    let tap = |key: u32, shift: bool, hold: Duration| {
        if shift {
            keyboard.modifiers(1, 0, 0, 0);
        }
        keyboard.key(now_ms(), key, 1); // pressed
        let _ = conn.flush();
        std::thread::sleep(hold);
        keyboard.key(now_ms(), key, 0); // released
        if shift {
            keyboard.modifiers(0, 0, 0, 0);
        }
        let _ = conn.flush();
        std::thread::sleep(Duration::from_millis(step));
    };

    for arg in &args {
        let (kind, value) = match arg.split_once(':') {
            Some(parts) => parts,
            None => {
                eprintln!("keyin: expected <kind>:<value>, got {arg:?}");
                std::process::exit(2);
            }
        };
        match kind {
            "type" => {
                for c in value.chars() {
                    match us_key(c) {
                        Some((key, shift)) => tap(key, shift, Duration::from_millis(8)),
                        None => {
                            eprintln!("keyin: no us keycode for {c:?}");
                            std::process::exit(2);
                        }
                    }
                }
            }
            "press" | "hold" => {
                let Some(key) = named_key(value) else {
                    eprintln!("keyin: unknown key name {value:?}");
                    std::process::exit(2);
                };
                let hold = if kind == "hold" {
                    Duration::from_millis(900)
                } else {
                    Duration::from_millis(8)
                };
                tap(key, false, hold);
            }
            "raw" => {
                let key: u32 = match value.parse() {
                    Ok(key) => key,
                    Err(_) => {
                        eprintln!("keyin: raw needs an evdev keycode, got {value:?}");
                        std::process::exit(2);
                    }
                };
                tap(key, false, Duration::from_millis(8));
            }
            other => {
                eprintln!("keyin: unknown kind {other:?}");
                std::process::exit(2);
            }
        }
    }
    let _ = conn.flush();
    // Let the compositor process the last release before the socket closes.
    let _ = conn.roundtrip();
}
