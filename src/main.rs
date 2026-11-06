use std::path::PathBuf;
use std::process::ExitCode;

use hayu::{bar, config, doctor, ipc};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.as_slice() {
        [command] if command == "doctor" => ExitCode::from(doctor::run() as u8),
        [command] if command == "run" => run(),
        [group, command] if group == "config" && command == "check" => check(None),
        [group, command, path] if group == "config" && command == "check" => {
            check(Some(PathBuf::from(path)))
        }
        [group, command] if group == "launcher" => match command.as_str() {
            "toggle" => ipc_command("launcher.toggle"),
            "open" => ipc_command("launcher.open"),
            "close" => ipc_command("launcher.close"),
            _ => {
                usage();
                ExitCode::from(2)
            }
        },
        [command] if command == "quit" => ipc_command("quit"),
        [command] if command == "reload" => ipc_command("reload"),
        [flag] if flag == "-h" || flag == "--help" => {
            usage();
            ExitCode::SUCCESS
        }
        _ => {
            usage();
            ExitCode::from(2)
        }
    }
}

fn check(path: Option<PathBuf>) -> ExitCode {
    let path = path.unwrap_or_else(config::default_path);
    match config::load(&path) {
        Ok(_) => {
            println!("config OK: {}", path.display());
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("hayu: {}: {err}", path.display());
            ExitCode::FAILURE
        }
    }
}

fn ipc_command(command: &str) -> ExitCode {
    match ipc::send_command(command) {
        Ok(Some(notice)) => {
            eprintln!("hayu: {notice}");
            ExitCode::SUCCESS
        }
        Ok(None) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("hayu: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> ExitCode {
    let path = config::default_path();
    let cfg = if path.exists() {
        match config::load(&path) {
            Ok(cfg) => cfg,
            Err(err) => {
                eprintln!("hayu: {}: {err}", path.display());
                return ExitCode::FAILURE;
            }
        }
    } else {
        eprintln!(
            "hayu: no config at {}; using built-in defaults",
            path.display()
        );
        config::Config::default()
    };
    match bar::run(cfg) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("hayu: {err}");
            ExitCode::FAILURE
        }
    }
}

fn usage() {
    eprintln!(
        "hayu 0.1.0 (M0 prototype)\n\
         \n\
         Usage:\n\
         \x20 hayu run\n\
         \x20 hayu doctor\n\
         \x20 hayu launcher toggle|open|close\n\
         \x20 hayu quit\n\
         \x20 hayu reload\n\
         \x20 hayu config check [PATH]\n"
    );
}
