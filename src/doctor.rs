use std::os::unix::fs::FileTypeExt;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};

use crate::hyprland::Hyprland;

pub fn run() -> i32 {
    let mut mandatory_ok = true;

    let runtime = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from);
    match &runtime {
        Some(path) if path.is_absolute() && path.is_dir() => {
            println!("XDG_RUNTIME_DIR: ok ({})", path.display());
        }
        Some(path) => {
            mandatory_ok = false;
            println!(
                "XDG_RUNTIME_DIR: FAIL ({} is not an absolute directory)",
                path.display()
            );
        }
        None => {
            mandatory_ok = false;
            println!("XDG_RUNTIME_DIR: FAIL (not set)");
        }
    }

    let display = std::env::var("WAYLAND_DISPLAY").ok();
    match &display {
        Some(name) => {
            let path = if Path::new(name).is_absolute() {
                PathBuf::from(name)
            } else if let Some(runtime) = &runtime {
                runtime.join(name)
            } else {
                PathBuf::from(name)
            };
            if is_socket(&path) {
                println!("WAYLAND_DISPLAY: ok ({name} -> {})", path.display());
            } else {
                mandatory_ok = false;
                println!(
                    "WAYLAND_DISPLAY: FAIL ({name} -> {} is not a socket)",
                    path.display()
                );
            }
        }
        None if std::env::var("WAYLAND_SOCKET").is_ok() => {
            println!("WAYLAND_DISPLAY: not set; WAYLAND_SOCKET is set");
        }
        None => {
            mandatory_ok = false;
            println!("WAYLAND_DISPLAY: FAIL (not set and WAYLAND_SOCKET is unset)");
        }
    }

    match Hyprland::discover() {
        None => {
            println!(
                "Hyprland: unavailable (no request socket under $XDG_RUNTIME_DIR/hypr); \
                 bar will run without workspace/title data"
            );
        }
        Some(hypr) => {
            println!(
                "Hyprland request socket: {} (reachable)",
                hypr.request_socket().display()
            );
            let event_reachable = UnixStream::connect(hypr.event_socket()).is_ok();
            println!(
                "Hyprland event socket: {} ({})",
                hypr.event_socket().display(),
                if event_reachable {
                    "reachable"
                } else {
                    "unreachable"
                }
            );
            match hypr.version() {
                Ok(version) => println!("Hyprland version: {version}"),
                Err(err) => println!("Hyprland version: FAIL ({err})"),
            }
            match hypr.snapshot() {
                Ok(snapshot) => {
                    println!("Monitors: {}", snapshot.monitors.len());
                    let workspace = if snapshot.active_workspace.name.is_empty() {
                        snapshot.active_workspace.id.to_string()
                    } else {
                        snapshot.active_workspace.name.clone()
                    };
                    println!("Active workspace: {workspace}");
                }
                Err(err) => println!("Hyprland snapshot: FAIL ({err})"),
            }
        }
    }

    println!("D-Bus services (audio, UPower, NetworkManager, backlight, tray): not probed in M0");

    println!("IPC protocol: v{}", crate::ipc::PROTOCOL_VERSION);
    match crate::ipc::socket_path() {
        Ok(path) => println!("Control socket: {}", path.display()),
        Err(error) => println!("Control socket: unavailable ({error})"),
    }

    if mandatory_ok { 0 } else { 1 }
}

fn is_socket(path: &Path) -> bool {
    std::fs::metadata(path)
        .map(|m| m.file_type().is_socket())
        .unwrap_or(false)
}
