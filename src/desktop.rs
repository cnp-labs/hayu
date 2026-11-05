use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Command;

const MAX_SCAN_DEPTH: usize = 16;
pub const MAX_RESULTS: usize = 50;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub desktop_id: String,
    pub name: String,
    pub exec: Vec<String>,
    pub terminal: bool,
}

/// A terminal emulator plus the arguments that make it run a program instead of
/// an interactive shell. The forms differ, so this cannot be a fixed `-e`.
struct Terminal {
    program: PathBuf,
    prefix: &'static [&'static str],
}

/// Each entry is the emulator's documented form: `foot` takes the command
/// positionally (its `-e` is accepted but ignored), `kitty` uses `--`, and
/// `wezterm` needs its `start` subcommand.
const TERMINAL_CANDIDATES: [(&str, &[&str]); 7] = [
    ("foot", &[]),
    ("alacritty", &["-e"]),
    ("kitty", &["--"]),
    ("wezterm", &["start", "--"]),
    ("ghostty", &["-e"]),
    ("konsole", &["-e"]),
    ("xterm", &["-e"]),
];

/// `$TERMINAL` wins when it names an executable on `PATH`, then a short list of
/// common emulators. Returns the program and its prefix, never a shell string.
fn pick_terminal(env_terminal: Option<&str>, path: &[PathBuf]) -> Option<Terminal> {
    let find = |name: &str| {
        path.iter()
            .map(|dir| dir.join(name))
            .find(|candidate| is_executable(candidate))
    };
    if let Some(value) = env_terminal
        && let Some(name) = value.split_whitespace().next()
        && let Some(program) = find(name)
    {
        // Match the resolved file name so `$TERMINAL=/usr/bin/kitty` and
        // `$TERMINAL=kitty` agree. An unknown emulator gets the widest
        // convention; the user can point $TERMINAL at one that accepts -e.
        let prefix = program
            .file_name()
            .and_then(|name| name.to_str())
            .and_then(|name| TERMINAL_CANDIDATES.iter().find(|(known, _)| *known == name))
            .map(|(_, prefix)| *prefix)
            .unwrap_or(&["-e"]);
        return Some(Terminal { program, prefix });
    }
    TERMINAL_CANDIDATES
        .iter()
        .find_map(|(name, prefix)| find(name).map(|program| Terminal { program, prefix }))
}

fn terminal_program() -> Option<Terminal> {
    let path: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|value| std::env::split_paths(&value).collect())
        .unwrap_or_default();
    pick_terminal(std::env::var("TERMINAL").ok().as_deref(), &path)
}

impl Entry {
    fn command(&self, terminal: Option<Terminal>) -> Result<Command, String> {
        let (program, args) = self
            .exec
            .split_first()
            .ok_or_else(|| "desktop entry has an empty Exec".to_string())?;
        if !self.terminal {
            let mut command = Command::new(program);
            command.args(args);
            return Ok(command);
        }
        let terminal = terminal.ok_or_else(|| {
            format!(
                "{} needs a terminal, but no terminal emulator was found in PATH (set $TERMINAL)",
                self.name
            )
        })?;
        let mut command = Command::new(&terminal.program);
        command.args(terminal.prefix);
        command.args(&self.exec);
        Ok(command)
    }

    pub fn launch(&self) -> Result<(), String> {
        let mut child = self
            .command(if self.terminal { terminal_program() } else { None })?
            .spawn()
            .map_err(|e| format!("cannot launch {}: {e}", self.name))?;
        // Reap the child without making the UI wait for the application to exit.
        std::thread::spawn(move || {
            let _ = child.wait();
        });
        Ok(())
    }
}

pub fn scan() -> Vec<Entry> {
    let started = std::time::Instant::now();
    let entries = scan_uncached();
    if std::env::var_os("HAYU_DEBUG_SCAN").is_some() {
        eprintln!(
            "hayu: desktop scan: {} entries in {:.2} ms",
            entries.len(),
            started.elapsed().as_secs_f64() * 1000.0
        );
    }
    entries
}

fn scan_uncached() -> Vec<Entry> {
    let dirs: Vec<PathBuf> = data_dirs()
        .iter()
        .map(|dir| dir.join("applications"))
        .collect();
    let current = std::env::var("XDG_CURRENT_DESKTOP").ok();
    scan_in(&dirs, current.as_deref())
}

fn data_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let data_home = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| home.map(|h| h.join(".local/share")));
    if let Some(dir) = data_home {
        dirs.push(dir);
    }
    match std::env::var("XDG_DATA_DIRS") {
        Ok(value) => dirs.extend(
            value
                .split(':')
                .filter(|part| !part.is_empty())
                .map(PathBuf::from),
        ),
        Err(_) => {
            dirs.push(PathBuf::from("/usr/local/share"));
            dirs.push(PathBuf::from("/usr/share"));
        }
    }
    dirs
}

fn scan_in(dirs: &[PathBuf], current_desktop: Option<&str>) -> Vec<Entry> {
    let mut seen = HashSet::new();
    let mut entries = Vec::new();
    for dir in dirs {
        scan_dir(dir, dir, current_desktop, &mut seen, &mut entries, 0);
    }
    entries
}

fn scan_dir(
    root: &Path,
    dir: &Path,
    current_desktop: Option<&str>,
    seen: &mut HashSet<String>,
    entries: &mut Vec<Entry>,
    depth: usize,
) {
    if depth > MAX_SCAN_DEPTH {
        return;
    }
    let Ok(read) = std::fs::read_dir(dir) else {
        return;
    };
    for item in read.filter_map(Result::ok) {
        let Ok(kind) = item.file_type() else {
            continue;
        };
        let path = item.path();
        if kind.is_dir() {
            // Never follow directory symlinks: a symlink loop must not hang the scan.
            if !kind.is_symlink() {
                scan_dir(root, &path, current_desktop, seen, entries, depth + 1);
            }
            continue;
        }
        if !path.extension().is_some_and(|ext| ext == "desktop") {
            continue;
        }
        let Some(id) = desktop_id(root, &path) else {
            continue;
        };
        if !seen.insert(id.clone()) {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        if let Some(entry) = parse_entry(&id, &text, current_desktop) {
            entries.push(entry);
        }
    }
}

fn desktop_id(root: &Path, path: &Path) -> Option<String> {
    let relative = path.strip_prefix(root).ok()?;
    let parts: Vec<String> = relative
        .components()
        .map(|part| part.as_os_str().to_string_lossy().into_owned())
        .collect();
    if parts.is_empty() {
        return None;
    }
    Some(parts.join("-"))
}

pub fn parse_entry(desktop_id: &str, text: &str, current_desktop: Option<&str>) -> Option<Entry> {
    let mut fields: Vec<(String, String)> = Vec::new();
    let mut in_entry = false;
    for line in text.lines() {
        let line = line.trim_start();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') {
            in_entry = line.trim_end() == "[Desktop Entry]";
            continue;
        }
        if !in_entry {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        fields.push((key.trim().to_string(), value.trim().to_string()));
    }

    let get = |name: &str| {
        fields
            .iter()
            .rev()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    };

    if get("Type") != Some("Application") {
        return None;
    }
    if parse_bool(get("Hidden")) || parse_bool(get("NoDisplay")) {
        return None;
    }
    if let Some(try_exec) = get("TryExec").filter(|value| !value.is_empty())
        && !executable_found(try_exec)
    {
        return None;
    }
    if !desktop_matches(get("OnlyShowIn"), true, current_desktop)
        || !desktop_matches(get("NotShowIn"), false, current_desktop)
    {
        return None;
    }

    let name = unescape_value(get("Name")?);
    if name.is_empty() {
        return None;
    }
    let exec = parse_exec(get("Exec")?).ok()?;
    Some(Entry {
        desktop_id: desktop_id.to_string(),
        name,
        exec,
        terminal: parse_bool(get("Terminal")),
    })
}

fn parse_bool(value: Option<&str>) -> bool {
    value.is_some_and(|value| value.eq_ignore_ascii_case("true"))
}

fn unescape_value(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut chars = value.chars();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            out.push(ch);
            continue;
        }
        match chars.next() {
            Some('s') => out.push(' '),
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some('\\') => out.push('\\'),
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}

fn desktop_matches(value: Option<&str>, only_show: bool, current: Option<&str>) -> bool {
    let Some(current) = current else {
        return true;
    };
    let Some(value) = value else {
        return true;
    };
    let entries: Vec<String> = value
        .split(';')
        .map(|part| part.trim().to_lowercase())
        .filter(|part| !part.is_empty())
        .collect();
    if entries.is_empty() {
        return true;
    }

    let mut desktops: Vec<String> = current
        .split(':')
        .map(|part| part.trim().to_lowercase())
        .filter(|part| !part.is_empty())
        .collect();
    if desktops
        .iter()
        .any(|desktop| desktop == "hyprland" || desktop == "wlroots")
    {
        desktops.push("hyprland".into());
        desktops.push("wlroots".into());
    }

    let listed = entries
        .iter()
        .any(|entry| desktops.iter().any(|desktop| desktop == entry));
    if only_show { listed } else { !listed }
}

fn executable_found(value: &str) -> bool {
    let path = Path::new(value);
    if path.is_absolute() || value.contains('/') {
        return is_executable(path);
    }
    let Some(paths) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&paths).any(|dir| is_executable(&dir.join(value)))
}

fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path)
        .map(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

const FIELD_CODES: &[char] = &[
    'f', 'F', 'u', 'U', 'i', 'c', 'k', 'd', 'D', 'n', 'N', 'v', 'm',
];

/// Tokenize Exec with desktop-entry quoting and drop field codes. No shell is involved.
pub fn parse_exec(value: &str) -> Result<Vec<String>, String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    let mut has_token = false;
    let mut chars = value.chars();
    while let Some(ch) = chars.next() {
        match ch {
            '\\' => {
                if let Some(escaped) = chars.next() {
                    current.push(escaped);
                    has_token = true;
                } else {
                    return Err("Exec ends with a bare backslash".into());
                }
            }
            '"' => {
                in_quotes = !in_quotes;
                has_token = true;
            }
            ch if ch.is_whitespace() && !in_quotes => {
                if has_token {
                    tokens.push(std::mem::take(&mut current));
                    has_token = false;
                }
            }
            _ => {
                current.push(ch);
                has_token = true;
            }
        }
    }
    if in_quotes {
        return Err("Exec has an unterminated quote".into());
    }
    if has_token {
        tokens.push(current);
    }

    let mut command = Vec::new();
    for token in tokens {
        let token = strip_field_codes(&token)?;
        if !token.is_empty() {
            command.push(token);
        }
    }
    if command.is_empty() || command[0].is_empty() {
        return Err("Exec is empty after removing field codes".into());
    }
    Ok(command)
}

fn strip_field_codes(token: &str) -> Result<String, String> {
    let mut out = String::with_capacity(token.len());
    let mut chars = token.chars();
    while let Some(ch) = chars.next() {
        if ch != '%' {
            out.push(ch);
            continue;
        }
        let Some(code) = chars.next() else {
            return Err("Exec has a bare field-code marker".into());
        };
        if FIELD_CODES.contains(&code) {
            continue;
        }
        return Err(format!("unsupported field code %{code}"));
    }
    Ok(out)
}

pub fn filter(entries: &[Entry], query: &str) -> Vec<Entry> {
    let query = query.to_lowercase();
    let mut results: Vec<Entry> = if query.is_empty() {
        entries.to_vec()
    } else {
        entries
            .iter()
            .filter_map(|entry| {
                let name_score = fuzzy_score(&entry.name.to_lowercase(), &query);
                let id_score = fuzzy_score(&entry.desktop_id.to_lowercase(), &query);
                match name_score.max(id_score) {
                    Some(score) => Some((entry, score)),
                    None => None,
                }
            })
            .map(|(entry, _)| entry.clone())
            .collect()
    };

    results.sort_by(|a, b| {
        if query.is_empty() {
            return a
                .name
                .to_lowercase()
                .cmp(&b.name.to_lowercase())
                .then_with(|| a.desktop_id.cmp(&b.desktop_id));
        }
        let score_a = fuzzy_score(&a.name.to_lowercase(), &query)
            .max(fuzzy_score(&a.desktop_id.to_lowercase(), &query))
            .unwrap_or(0);
        let score_b = fuzzy_score(&b.name.to_lowercase(), &query)
            .max(fuzzy_score(&b.desktop_id.to_lowercase(), &query))
            .unwrap_or(0);
        score_b
            .cmp(&score_a)
            .then_with(|| a.name.chars().count().cmp(&b.name.chars().count()))
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
            .then_with(|| a.desktop_id.cmp(&b.desktop_id))
    });
    results.truncate(MAX_RESULTS);
    results
}

fn fuzzy_score(text: &str, query: &str) -> Option<i32> {
    if query.is_empty() {
        return Some(0);
    }
    let mut score = 0;
    let mut previous_char: Option<char> = None;
    let mut previous_match: Option<usize> = None;
    let mut first_match: Option<usize> = None;
    let mut matched = 0usize;
    let query_chars: Vec<char> = query.chars().collect();
    for (index, ch) in text.chars().enumerate() {
        if matched >= query_chars.len() {
            break;
        }
        if ch != query_chars[matched] {
            previous_char = Some(ch);
            continue;
        }
        let boundary = index == 0 || previous_char.is_some_and(|prev| !prev.is_alphanumeric());
        if first_match.is_none() {
            first_match = Some(index);
        }
        score += 10;
        if boundary {
            score += 40;
        }
        if previous_match == Some(index.saturating_sub(1)) {
            score += 15;
        }
        previous_match = Some(index);
        previous_char = Some(ch);
        matched += 1;
    }
    if matched != query_chars.len() {
        return None;
    }
    if first_match == Some(0) {
        score += 300;
    }
    if text.starts_with(query) {
        score += 1000;
    }
    // Prefer shorter labels when the match quality is otherwise similar.
    score -= text.chars().count() as i32;
    Some(score)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::os::unix::fs::PermissionsExt;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static NEXT_TEMP: AtomicUsize = AtomicUsize::new(0);

    fn temp_dir(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "hayu-desktop-{}-{}-{label}",
            std::process::id(),
            NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write(path: &Path, text: &str) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        let mut file = std::fs::File::create(path).unwrap();
        file.write_all(text.as_bytes()).unwrap();
    }

    fn entry(id: &str, name: &str) -> Entry {
        Entry {
            desktop_id: id.into(),
            name: name.into(),
            exec: vec!["app".into()],
            terminal: false,
        }
    }

    const FIREFOX: &str = "[Desktop Entry]\n\
        Type=Application\n\
        Name=Firefox\n\
        Exec=firefox %u\n";

    #[test]
    fn parses_minimal_entry() {
        let entry = parse_entry("firefox.desktop", FIREFOX, None).unwrap();
        assert_eq!(entry.name, "Firefox");
        assert_eq!(entry.exec, ["firefox"]);
        assert!(!entry.terminal);
    }

    #[test]
    fn skips_hidden_nodisplay_non_application_and_bad_exec() {
        assert!(
            parse_entry(
                "x.desktop",
                "[Desktop Entry]\nType=Application\nName=X\nExec=x\nHidden=true\n",
                None
            )
            .is_none()
        );
        assert!(
            parse_entry(
                "x.desktop",
                "[Desktop Entry]\nType=Application\nName=X\nExec=x\nNoDisplay=true\n",
                None
            )
            .is_none()
        );
        assert!(
            parse_entry(
                "x.desktop",
                "[Desktop Entry]\nType=Link\nName=X\nExec=x\n",
                None
            )
            .is_none()
        );
        assert!(
            parse_entry(
                "x.desktop",
                "[Desktop Entry]\nType=Application\nExec=x\n",
                None
            )
            .is_none()
        );
        assert!(
            parse_entry(
                "x.desktop",
                "[Desktop Entry]\nType=Application\nName=X\n",
                None
            )
            .is_none()
        );
        assert!(
            parse_entry(
                "x.desktop",
                "[Desktop Entry]\nType=Application\nName=X\nExec=x %\n",
                None
            )
            .is_none()
        );
        assert!(
            parse_entry(
                "x.desktop",
                "[Desktop Entry]\nType=Application\nName=X\nExec=x %Z\n",
                None
            )
            .is_none()
        );
    }

    #[test]
    fn parses_only_desktop_entry_group() {
        let text = "[Other]\nName=Wrong\nExec=wrong\n[Desktop Entry]\nType=Application\nName=Right\nExec=right\n";
        let entry = parse_entry("right.desktop", text, None).unwrap();
        assert_eq!(entry.name, "Right");
        assert_eq!(entry.exec, ["right"]);
    }

    #[test]
    fn only_show_in_and_not_show_in() {
        let only =
            "[Desktop Entry]\nType=Application\nName=X\nExec=x\nOnlyShowIn=GNOME;Hyprland;\n";
        assert!(parse_entry("x.desktop", only, Some("Hyprland")).is_some());
        assert!(parse_entry("x.desktop", only, Some("wlroots")).is_some());
        assert!(parse_entry("x.desktop", only, Some("KDE")).is_none());
        let not = "[Desktop Entry]\nType=Application\nName=X\nExec=x\nNotShowIn=wlroots;\n";
        assert!(parse_entry("x.desktop", not, Some("Hyprland")).is_none());
        assert!(parse_entry("x.desktop", not, Some("KDE")).is_some());
        assert!(parse_entry("x.desktop", only, None).is_some());
    }

    #[test]
    fn try_exec_missing_is_skipped() {
        let text = "[Desktop Entry]\nType=Application\nName=X\nExec=x\nTryExec=/definitely/missing/hayu-test\n";
        assert!(parse_entry("x.desktop", text, None).is_none());
    }

    #[test]
    fn tokenizer_handles_quotes_and_backslashes() {
        assert_eq!(
            parse_exec(r#"app --title "Hello world" --path "/tmp/a b" tail"#).unwrap(),
            [
                "app",
                "--title",
                "Hello world",
                "--path",
                "/tmp/a b",
                "tail"
            ]
        );
        assert_eq!(
            parse_exec(r#"app one\ two "a\"b""#).unwrap(),
            ["app", "one two", "a\"b"]
        );
        assert!(parse_exec(r#"app "unterminated"#).is_err());
    }

    #[test]
    fn field_codes_are_dropped_and_unknown_rejected() {
        assert_eq!(
            parse_exec("app %f %F %u %U %i %c %k %d %D %n %N %v %m --keep").unwrap(),
            ["app", "--keep"]
        );
        assert_eq!(parse_exec("app --file=%f").unwrap(), ["app", "--file="]);
        assert!(parse_exec("app %X").is_err());
        assert!(parse_exec("app %%").is_err());
        assert!(parse_exec("app %").is_err());
    }

    #[test]
    fn desktop_ids_are_relative_to_the_applications_directory() {
        let data = temp_dir("data");
        write(
            &data.join("applications/app.desktop"),
            "[Desktop Entry]\nType=Application\nName=App\nExec=app\n",
        );
        write(
            &data.join("applications/sub/nested.desktop"),
            "[Desktop Entry]\nType=Application\nName=Nested\nExec=nested\n",
        );
        write(
            &data.join("elsewhere/other.desktop"),
            "[Desktop Entry]\nType=Application\nName=Other\nExec=other\n",
        );
        let entries = scan_in(&[data.join("applications")], None);
        let ids: Vec<&str> = entries.iter().map(|e| e.desktop_id.as_str()).collect();
        assert!(ids.contains(&"app.desktop"), "{ids:?}");
        assert!(ids.contains(&"sub-nested.desktop"), "{ids:?}");
        assert!(!ids.iter().any(|id| id.starts_with("applications-")), "{ids:?}");
        let _ = std::fs::remove_dir_all(data);
    }
    #[test]
    fn deduplicates_by_desktop_id_with_first_directory_winning() {
        let first = temp_dir("first");
        let second = temp_dir("second");
        write(
            &first.join("firefox.desktop"),
            "[Desktop Entry]\nType=Application\nName=First Firefox\nExec=first\n",
        );
        write(
            &second.join("firefox.desktop"),
            "[Desktop Entry]\nType=Application\nName=Second Firefox\nExec=second\n",
        );
        write(
            &second.join("nested/app.desktop"),
            "[Desktop Entry]\nType=Application\nName=Nested\nExec=nested\n",
        );
        let entries = scan_in(&[first.clone(), second.clone()], None);
        let firefox: Vec<_> = entries
            .iter()
            .filter(|entry| entry.desktop_id == "firefox.desktop")
            .collect();
        assert_eq!(firefox.len(), 1);
        assert_eq!(firefox[0].name, "First Firefox");
        assert!(
            entries
                .iter()
                .any(|entry| entry.desktop_id == "nested-app.desktop")
        );
        let _ = std::fs::remove_dir_all(first);
        let _ = std::fs::remove_dir_all(second);
    }

    #[test]
    fn malformed_files_do_not_abort_scan() {
        let dir = temp_dir("malformed");
        write(&dir.join("bad.desktop"), "this is not a desktop file\n");
        write(&dir.join("good.desktop"), FIREFOX);
        let entries = scan_in(&[dir.clone()], None);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "Firefox");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn directory_symlinks_are_not_followed() {
        let root = temp_dir("symlink-root");
        let target = temp_dir("symlink-target");
        write(&target.join("loop.desktop"), FIREFOX);
        #[cfg(unix)]
        std::os::unix::fs::symlink(&target, root.join("loop")).unwrap();
        let entries = scan_in(&[root.clone()], None);
        assert!(entries.is_empty());
        let _ = std::fs::remove_dir_all(root);
        let _ = std::fs::remove_dir_all(target);
    }

    #[test]
    fn try_exec_requires_executable_file() {
        let dir = temp_dir("tryexec");
        let bin = dir.join("bin");
        write(&bin, "#!/bin/sh\n");
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(is_executable(&bin));
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(!is_executable(&bin));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn fuzzy_filter_orders_prefixes_first_and_caps_results() {
        let entries = vec![
            entry("writer.desktop", "LibreOffice Writer"),
            entry("firefox.desktop", "Firefox"),
            entry("files.desktop", "File Manager"),
            entry("terminal.desktop", "Terminal"),
        ];
        let results = filter(&entries, "fi");
        assert_eq!(results[0].name, "Firefox");
        assert_eq!(results[1].name, "File Manager");
        let results = filter(&entries, "WRIT");
        assert_eq!(results[0].desktop_id, "writer.desktop");
        assert!(filter(&entries, "zzzz").is_empty());
    }

    #[test]
    fn empty_query_sorts_by_name_and_caps_at_fifty() {
        let mut entries: Vec<Entry> = (0..60)
            .map(|i| entry(&format!("a{i}.desktop"), &format!("App {i:02}")))
            .collect();
        entries.push(entry("z.desktop", "Aardvark"));
        let results = filter(&entries, "");
        assert_eq!(results.len(), MAX_RESULTS);
        assert_eq!(results[0].name, "Aardvark");
        assert_eq!(results[1].name, "App 00");
    }

    fn install(path: &Path) {
        write(path, "#!/bin/sh\n");
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    #[test]
    fn terminal_program_prefers_env_then_known_emulators() {
        let dir = temp_dir("term");
        let bin = dir.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let fake = bin.join("myterm");
        install(&fake);
        let path = vec![bin.clone()];
        let from_env = pick_terminal(Some("myterm"), &path).unwrap();
        assert_eq!(from_env.program, fake);
        assert_eq!(from_env.prefix, ["-e"]);
        assert!(pick_terminal(Some("missing --flag"), &path).is_none());
        assert!(pick_terminal(None, &path).is_none(), "only listed names count");
        install(&bin.join("foot"));
        let fallback = pick_terminal(None, &path).unwrap();
        assert_eq!(fallback.program, bin.join("foot"));
        assert!(fallback.prefix.is_empty(), "foot takes the command directly");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn each_fallback_uses_its_own_program_arguments() {
        // Documented invocations: alacritty -e, kitty --, wezterm start --,
        // ghostty/konsole/xterm -e, foot positionally.
        let cases: [(&str, &[&str]); 7] = [
            ("foot", &[]),
            ("alacritty", &["-e"]),
            ("kitty", &["--"]),
            ("wezterm", &["start", "--"]),
            ("ghostty", &["-e"]),
            ("konsole", &["-e"]),
            ("xterm", &["-e"]),
        ];
        for (name, prefix) in cases {
            let dir = temp_dir(name);
            let bin = dir.join("bin");
            std::fs::create_dir_all(&bin).unwrap();
            let exe = bin.join(name);
            install(&exe);
            let found = pick_terminal(None, &[bin.clone()]).unwrap();
            assert_eq!(found.program, exe, "{name}");
            assert_eq!(found.prefix, prefix, "{name}");

            let by_name = pick_terminal(Some(name), &[bin.clone()]).unwrap();
            assert_eq!(by_name.program, exe, "{name} in $TERMINAL by name");
            assert_eq!(by_name.prefix, prefix, "{name} in $TERMINAL by name");

            let absolute = exe.to_str().unwrap();
            let by_path = pick_terminal(Some(absolute), &[dir.join("empty")]).unwrap();
            assert_eq!(by_path.program, exe, "{name} in $TERMINAL by path");
            assert_eq!(by_path.prefix, prefix, "{name} in $TERMINAL by path");

            let mut entry = entry("x.desktop", "X");
            entry.exec = vec!["btop".into(), "--utf".into()];
            entry.terminal = true;
            let mut expected: Vec<&str> = prefix.to_vec();
            expected.extend(["btop", "--utf"]);
            let command = entry.command(Some(found)).unwrap();
            assert_eq!(command.get_program(), exe, "{name}");
            assert_eq!(command.get_args().collect::<Vec<_>>(), expected, "{name}");
            let _ = std::fs::remove_dir_all(dir);
        }
    }

    #[test]
    fn argument_vector_survives_spaces_empty_and_shell_characters() {
        let dir = temp_dir("argv");
        let bin = dir.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let exe = bin.join("kitty");
        install(&exe);
        let found = pick_terminal(Some("kitty"), &[bin]).unwrap();

        let argv = [
            "echo",
            "",
            "two words",
            "$HOME",
            "a;b|c&d",
            "quote's \"double\"",
        ];
        let mut entry = entry("x.desktop", "X");
        entry.exec = argv.iter().map(|arg| (*arg).to_string()).collect();
        entry.terminal = true;
        let command = entry.command(Some(found)).unwrap();
        let mut expected = vec!["--"];
        expected.extend(argv);
        assert_eq!(command.get_program(), exe);
        assert_eq!(command.get_args().collect::<Vec<_>>(), expected);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn terminal_entries_wrap_the_command_or_explain() {
        let mut entry = entry("x.desktop", "X");
        entry.exec = vec!["btop".into(), "--utf".into()];
        let plain = entry.command(None).unwrap();
        assert_eq!(plain.get_program(), "btop");
        assert_eq!(plain.get_args().collect::<Vec<_>>(), ["--utf"]);
        entry.terminal = true;
        let wrapped = entry
            .command(Some(Terminal {
                program: PathBuf::from("/usr/bin/foot"),
                prefix: &[],
            }))
            .unwrap();
        assert_eq!(wrapped.get_program(), "/usr/bin/foot");
        assert_eq!(wrapped.get_args().collect::<Vec<_>>(), ["btop", "--utf"]);
        let error = entry.command(None).unwrap_err();
        assert!(error.contains("no terminal emulator"), "{error}");
    }
}
