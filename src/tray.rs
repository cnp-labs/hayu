//! Minimal StatusNotifierItem host: own `org.kde.StatusNotifierWatcher` when
//! nobody else does, follow registered items, and activate them on demand.
//!
//! Deliberately small: no context menus, no scroll, no NewIcon/NewTitle
//! signals. An item's icon and title are read when it registers and when its
//! bus name disappears. Another host is never displaced.
use futures_lite::StreamExt;
use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use zbus::fdo::{NameOwnerChanged, RequestNameFlags, RequestNameReply};
use zbus::message::Header;
use zbus::{Connection, MatchRule, Proxy, interface};

pub const WATCHER_NAME: &str = "org.kde.StatusNotifierWatcher";
pub const WATCHER_PATH: &str = "/StatusNotifierWatcher";
pub const ITEM_INTERFACE: &str = "org.kde.StatusNotifierItem";
const DEFAULT_ITEM_PATH: &str = "/StatusNotifierItem";
const MAX_ITEMS: usize = 64;
const MAX_ICON_DIM: u32 = 512;
const MAX_ICON_BYTES: usize = MAX_ICON_DIM as usize * MAX_ICON_DIM as usize * 4;
const MAX_PNG_FILE_BYTES: u64 = 4 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq)]
pub struct TrayItem {
    pub id: usize,
    pub title: String,
    pub service: String,
    pub path: String,
    pub icon: Option<Icon>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Icon {
    pub width: u32,
    pub height: u32,
    pub rgba: Arc<[u8]>,
}

#[derive(Debug)]
pub struct Message {
    pub generation: u64,
    pub event: Event,
}

#[derive(Debug)]
pub enum Event {
    Items(Vec<TrayItem>),
    /// The tray is unavailable and no items will arrive.
    Disabled(String),
}

/// Inert until started. Owns at most one worker; stop joins it before restart.
pub struct Service {
    worker: Option<(
        async_channel::Sender<Command>,
        async_channel::Sender<()>,
        JoinHandle<()>,
    )>,
}

impl Service {
    pub fn start(generation: u64, tx: calloop::channel::Sender<Message>) -> Result<Self, String> {
        Self::start_with_address(generation, tx, None)
    }

    fn start_with_address(
        generation: u64,
        tx: calloop::channel::Sender<Message>,
        address: Option<String>,
    ) -> Result<Self, String> {
        let mut service = Self { worker: None };
        let (cmd_tx, cmd_rx) = async_channel::bounded(MAX_ITEMS);
        let (cancel_tx, cancel_rx) = async_channel::bounded::<()>(1);
        let worker_tx = cmd_tx.clone();
        let handle = thread::Builder::new()
            .name("hayu-tray".into())
            .spawn(move || {
                let worker = run(generation, tx.clone(), cmd_tx, cmd_rx, address);
                if let Err(error) = zbus::block_on(futures_lite::future::race(worker, async {
                    let _ = cancel_rx.recv().await;
                    Ok(())
                })) {
                    let _ = tx.send(Message {
                        generation,
                        event: Event::Disabled(error),
                    });
                }
            })
            .map_err(|error| format!("cannot spawn tray worker: {error}"))?;
        service.worker = Some((worker_tx, cancel_tx, handle));
        Ok(service)
    }

    pub fn activate(&self, item: &TrayItem) -> Result<(), ActivateError> {
        let Some((commands, _, worker)) = &self.worker else {
            return Err(ActivateError::Stopped);
        };
        // A finished worker leaves the sender alive, so a queued activation
        // would be accepted and silently dropped. Report it instead.
        if worker.is_finished() {
            return Err(ActivateError::Stopped);
        }
        commands
            .try_send(Command::Activate {
                service: item.service.clone(),
                path: item.path.clone(),
            })
            .map_err(|error| match error {
                async_channel::TrySendError::Full(_) => ActivateError::Full,
                async_channel::TrySendError::Closed(_) => ActivateError::Stopped,
            })
    }

    pub fn stop(&mut self) {
        if let Some((commands, cancel, worker)) = self.worker.take() {
            let _ = cancel.try_send(());
            drop(cancel);
            drop(commands);
            let _ = worker.join();
        }
    }
}

impl Drop for Service {
    fn drop(&mut self) {
        self.stop();
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum ActivateError {
    Stopped,
    Full,
}

async fn run(
    generation: u64,
    tx: calloop::channel::Sender<Message>,
    cmd_tx: async_channel::Sender<Command>,
    cmd_rx: async_channel::Receiver<Command>,
    address: Option<String>,
) -> Result<(), String> {
    let builder = match address {
        Some(address) => zbus::connection::Builder::address(address.as_str()),
        None => zbus::connection::Builder::session(),
    }
    .map_err(|error| format!("session bus unavailable: {error}"))?;
    let conn = builder
        .method_timeout(std::time::Duration::from_secs(1))
        .build()
        .await
        .map_err(|error| format!("session bus unavailable: {error}"))?;
    run_with_connection(generation, tx, cmd_tx, cmd_rx, conn).await
}

async fn run_with_connection(
    generation: u64,
    tx: calloop::channel::Sender<Message>,
    cmd_tx: async_channel::Sender<Command>,
    cmd_rx: async_channel::Receiver<Command>,
    conn: Connection,
) -> Result<(), String> {
    // DoNotQueue: when another host owns the name we report it and stop
    // instead of taking the role away from it.
    let reply = conn
        .request_name_with_flags(WATCHER_NAME, RequestNameFlags::DoNotQueue.into())
        .await
        .map_err(|error| {
            format!("another StatusNotifier host owns {WATCHER_NAME}; tray disabled ({error})")
        })?;
    if reply != RequestNameReply::PrimaryOwner {
        return Err(format!(
            "another StatusNotifier host owns {WATCHER_NAME}; tray disabled"
        ));
    }
    let registered: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    conn.object_server()
        .at(
            WATCHER_PATH,
            Watcher {
                registered: registered.clone(),
                commands: cmd_tx.clone(),
            },
        )
        .await
        .map_err(|error| format!("cannot serve {WATCHER_PATH}: {error}"))?;

    let rule = MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        .interface("org.freedesktop.DBus")
        .map_err(|error| error.to_string())?
        .member("NameOwnerChanged")
        .map_err(|error| error.to_string())?
        .build();
    let mut changes = zbus::MessageStream::for_match_rule(rule, &conn, Some(256))
        .await
        .map_err(|error| format!("cannot watch bus names: {error}"))?;

    let mut items: Vec<Item> = Vec::new();
    loop {
        enum Incoming {
            Change(Option<zbus::Message>),
            Command(Option<Command>),
        }
        let incoming = futures_lite::future::or(
            async { Incoming::Change(changes.next().await.and_then(Result::ok)) },
            async { Incoming::Command(cmd_rx.recv().await.ok()) },
        )
        .await;
        let mut changed = false;
        match incoming {
            Incoming::Command(None) => break,
            Incoming::Command(Some(Command::Activate { service, path })) => {
                activate_on_bus(&conn, &service, &path).await;
            }
            Incoming::Command(Some(Command::Register { service, bus })) => {
                if let Some(item) = read_item(&conn, &service, &bus).await {
                    let already_known = items.iter().any(|known| same_destination(known, &item));
                    if !already_known && items.len() >= MAX_ITEMS {
                        eprintln!("hayu: tray item limit reached; ignoring {service}");
                    } else {
                        items.retain(|known| !same_destination(known, &item));
                        items.push(item);
                        changed = true;
                    }
                }
            }
            Incoming::Change(Some(message)) => {
                if let Some(signal) = NameOwnerChanged::from_message(message) {
                    if let Ok(args) = signal.args()
                        && args.new_owner().is_none()
                    {
                        let bus = args.name().to_string();
                        let before = items.len();
                        items.retain(|item| item.bus != bus);
                        changed = items.len() != before;
                    }
                }
            }
            Incoming::Change(None) => break,
        }
        if changed {
            *registered.lock().unwrap() =
                items.iter().map(|item| item.registered.clone()).collect();
            let snapshot: Vec<TrayItem> = items
                .iter()
                .enumerate()
                .map(|(id, item)| TrayItem {
                    id,
                    title: item.title.clone(),
                    service: item.service.clone(),
                    path: item.path.clone(),
                    icon: item.icon.clone(),
                })
                .collect();
            if tx
                .send(Message {
                    generation,
                    event: Event::Items(snapshot),
                })
                .is_err()
            {
                break;
            }
        }
    }
    Ok(())
}

struct Item {
    /// The exact argument the item registered with, for the watcher list.
    registered: String,
    /// Canonical bus name for activation; never a path or `bus/path`.
    service: String,
    bus: String,
    path: String,
    title: String,
    icon: Option<Icon>,
}

enum Command {
    Register { service: String, bus: String },
    Activate { service: String, path: String },
}

struct Watcher {
    registered: Arc<Mutex<Vec<String>>>,
    commands: async_channel::Sender<Command>,
}

#[interface(name = "org.kde.StatusNotifierWatcher")]
impl Watcher {
    async fn register_status_notifier_item(
        &self,
        service: &str,
        #[zbus(header)] header: Header<'_>,
    ) {
        let sender = header.sender().map(|name| name.to_string());
        let bus = service_bus(service, sender.as_deref());
        if self
            .commands
            .try_send(Command::Register {
                service: service.to_string(),
                bus,
            })
            .is_err()
        {
            eprintln!("hayu: tray registration queue full or closed; ignoring {service}");
        }
    }

    async fn register_status_notifier_host(&self, service: &str) {
        eprintln!("hayu: tray host registration from {service} ignored; hayu is the host");
    }

    #[zbus(property)]
    async fn registered_status_notifier_items(&self) -> Vec<String> {
        self.registered.lock().unwrap().clone()
    }

    #[zbus(property)]
    async fn is_status_notifier_host_registered(&self) -> bool {
        true
    }

    #[zbus(property)]
    async fn protocol_version(&self) -> i32 {
        0
    }
}

/// `service` may be a bus name, a bus name plus object path, or a bare path on
/// the caller's connection.
fn service_bus(service: &str, sender: Option<&str>) -> String {
    if service.starts_with('/') {
        return sender.unwrap_or_default().to_string();
    }
    match service.split_once('/') {
        Some((bus, _)) => bus.to_string(),
        None => service.to_string(),
    }
}

/// Canonical bus and object path for one registration. Bare paths belong to
/// the sender's unique bus name, so identical paths from different senders are
/// still distinct items.
fn item_identity(service: &str, bus: &str) -> (String, String) {
    if service.starts_with('/') {
        (bus.to_string(), service.to_string())
    } else if let Some((name, tail)) = service.split_once('/') {
        (name.to_string(), format!("/{tail}"))
    } else {
        (service.to_string(), DEFAULT_ITEM_PATH.to_string())
    }
}

fn same_destination(left: &Item, right: &Item) -> bool {
    left.service == right.service && left.path == right.path
}

async fn read_item(conn: &Connection, service: &str, bus: &str) -> Option<Item> {
    let (bus_name, path) = item_identity(service, bus);
    let proxy = match Proxy::new(conn, bus_name.as_str(), path.as_str(), ITEM_INTERFACE).await {
        Ok(proxy) => proxy,
        Err(error) => {
            eprintln!("hayu: tray item {service} unavailable: {error}");
            return None;
        }
    };
    let title: String = proxy.get_property("Title").await.unwrap_or_default();
    let icon = match proxy
        .get_property::<Vec<(i32, i32, Vec<u8>)>>("IconPixmap")
        .await
    {
        Ok(pixmaps) => pixmap_icon(&pixmaps).or_else(|| {
            // The name is only a fallback; a provided pixmap always wins.
            None
        }),
        Err(_) => None,
    };
    let icon = match icon {
        Some(icon) => Some(icon),
        None => proxy
            .get_property::<String>("IconName")
            .await
            .ok()
            .filter(|name| !name.is_empty())
            .and_then(|name| theme_icon(&name)),
    };
    Some(Item {
        registered: service.to_string(),
        service: bus_name.clone(),
        bus: bus_name,
        path,
        title,
        icon,
    })
}

fn pixmap_len(width: i32, height: i32) -> Option<usize> {
    let width = usize::try_from(width).ok()?;
    let height = usize::try_from(height).ok()?;
    if width == 0 || height == 0 || width > MAX_ICON_DIM as usize || height > MAX_ICON_DIM as usize
    {
        return None;
    }
    width.checked_mul(height)?.checked_mul(4)
}

fn pixmap_icon(pixmaps: &[(i32, i32, Vec<u8>)]) -> Option<Icon> {
    let (width, height, data) = pixmaps
        .iter()
        .filter(|(w, h, data)| pixmap_len(*w, *h) == Some(data.len()))
        .max_by_key(|(width, _, _)| *width)?;
    let mut rgba = Vec::with_capacity(data.len());
    for pixel in data.chunks_exact(4) {
        // Network byte order ARGB32.
        rgba.extend_from_slice(&[pixel[1], pixel[2], pixel[3], pixel[0]]);
    }
    Some(Icon {
        width: *width as u32,
        height: *height as u32,
        rgba: rgba.into(),
    })
}

/// Load `<name>.png` from a few icon roots. No theme inheritance and no SVG;
/// that is why a missing icon simply draws nothing.
fn theme_icon(name: &str) -> Option<Icon> {
    let mut roots = Vec::new();
    if let Some(data) = std::env::var_os("XDG_DATA_HOME") {
        roots.push(PathBuf::from(data).join("icons"));
    } else if let Some(home) = std::env::var_os("HOME") {
        roots.push(PathBuf::from(home).join(".local/share/icons"));
    }
    if let Some(home) = std::env::var_os("HOME") {
        roots.push(PathBuf::from(home).join(".icons"));
    }
    roots.push(PathBuf::from("/usr/share/icons"));
    let mut names = vec![name.to_string()];
    // Symbolic names are usually SVG-only; the full-colour PNG often shares
    // the name without the suffix.
    if let Some(base) = name.strip_suffix("-symbolic") {
        names.push(base.to_string());
    }
    for candidate in names {
        let wanted = format!("{candidate}.png");
        let mut best: Option<(u32, PathBuf)> = None;
        for root in &roots {
            find_named(root, &wanted, 0, &mut best);
        }
        if let Some((_, path)) = best
            && let Some(icon) = decode_png(&path)
        {
            return Some(icon);
        }
    }
    None
}

fn find_named(dir: &Path, wanted: &str, depth: u32, best: &mut Option<(u32, PathBuf)>) {
    if depth > 3 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            find_named(&path, wanted, depth + 1, best);
        } else if path.file_name().and_then(|name| name.to_str()) == Some(wanted) {
            // Prefer a size close to 22px, then the larger file.
            let size = size_from_path(&path).unwrap_or(0);
            let score = if size <= 22 { 22 - size } else { size - 22 };
            if best
                .as_ref()
                .is_none_or(|(best_score, _)| score < *best_score)
            {
                *best = Some((score, path));
            }
        }
    }
}

fn size_from_path(path: &Path) -> Option<u32> {
    path.components().find_map(|part| {
        let text = part.as_os_str().to_str()?;
        text.strip_suffix('x')?.parse().ok()
    })
}

fn decode_png(path: &Path) -> Option<Icon> {
    let file = std::fs::File::open(path).ok()?;
    if file.metadata().ok()?.len() > MAX_PNG_FILE_BYTES {
        return None;
    }
    let mut bytes = Vec::new();
    file.take(MAX_PNG_FILE_BYTES).read_to_end(&mut bytes).ok()?;
    decode_png_bytes(&bytes)
}

fn decode_png_bytes(bytes: &[u8]) -> Option<Icon> {
    let mut decoder = png::Decoder::new(Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = decoder.read_info().ok()?;
    let dimensions = reader.info();
    if dimensions.width == 0
        || dimensions.height == 0
        || dimensions.width > MAX_ICON_DIM
        || dimensions.height > MAX_ICON_DIM
    {
        return None;
    }
    let output_len = reader.output_buffer_size()?;
    if output_len > MAX_ICON_BYTES {
        return None;
    }
    let mut buffer = vec![0; output_len];
    let info = reader.next_frame(&mut buffer).ok()?;
    let decoded_len = info.buffer_size();
    if decoded_len > buffer.len() {
        return None;
    }
    let rgba: Vec<u8> = match info.color_type {
        png::ColorType::Rgba => buffer[..decoded_len].to_vec(),
        png::ColorType::Rgb => buffer[..decoded_len]
            .chunks_exact(3)
            .flat_map(|pixel| [pixel[0], pixel[1], pixel[2], 255])
            .collect(),
        png::ColorType::Grayscale => buffer[..decoded_len]
            .iter()
            .flat_map(|value| [*value, *value, *value, 255])
            .collect(),
        png::ColorType::GrayscaleAlpha => buffer[..decoded_len]
            .chunks_exact(2)
            .flat_map(|pixel| [pixel[0], pixel[0], pixel[0], pixel[1]])
            .collect(),
        png::ColorType::Indexed => return None,
    };
    if rgba.len() > MAX_ICON_BYTES {
        return None;
    }
    Some(Icon {
        width: info.width,
        height: info.height,
        rgba: rgba.into(),
    })
}

/// Perform one activation on the service worker. Builder's method timeout bounds D-Bus calls.
async fn activate_on_bus(conn: &Connection, service: &str, path: &str) {
    let result = async {
        let proxy = Proxy::new(conn, service, path, ITEM_INTERFACE).await?;
        proxy
            .call_method("Activate", &(0i32, 0i32))
            .await
            .map(|_| ())
    }
    .await;
    if let Err(error) = result {
        eprintln!("hayu: tray activate failed: {error}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::{Child, Command as ProcessCommand, Stdio};
    use std::sync::Mutex as StdMutex;
    use std::sync::atomic::{AtomicBool, Ordering};

    static DBUS_TEST_LOCK: StdMutex<()> = StdMutex::new(());

    struct TestSignal {
        started: AtomicBool,
        lock: StdMutex<()>,
        wake: std::sync::Condvar,
    }

    impl TestSignal {
        const fn new() -> Self {
            Self {
                started: AtomicBool::new(false),
                lock: StdMutex::new(()),
                wake: std::sync::Condvar::new(),
            }
        }
        fn signal(&self) {
            let _guard = self.lock.lock().unwrap();
            self.started.store(true, Ordering::Release);
            self.wake.notify_all();
        }
        fn wait_timeout(&self, timeout: std::time::Duration) -> Result<(), ()> {
            let guard = self.lock.lock().unwrap();
            let (guard, result) = self
                .wake
                .wait_timeout_while(guard, timeout, |_| !self.started.load(Ordering::Acquire))
                .unwrap();
            drop(guard);
            if result.timed_out() && !self.started.load(Ordering::Acquire) {
                Err(())
            } else {
                Ok(())
            }
        }
    }

    /// Each test owns its signals, so a parallel test can never observe or
    /// clear another one's state.
    struct SlowItem {
        title_started: Arc<TestSignal>,
        activate_started: Arc<TestSignal>,
    }

    #[interface(name = "org.kde.StatusNotifierItem")]
    impl SlowItem {
        #[zbus(property)]
        async fn title(&self) -> String {
            self.title_started.signal();
            std::future::pending::<String>().await
        }
        async fn activate(&self, _x: i32, _y: i32) {
            self.activate_started.signal();
            std::future::pending().await
        }
    }

    struct PrivateBus(Child);

    impl PrivateBus {
        fn start() -> (Self, String) {
            let mut child = ProcessCommand::new("dbus-daemon")
                .args(["--session", "--nofork", "--print-address=1"])
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn()
                .expect("start private dbus-daemon");
            use std::io::BufRead;
            let address = std::io::BufReader::new(child.stdout.take().expect("daemon stdout"))
                .lines()
                .next()
                .expect("address line")
                .expect("address");
            (Self(child), address)
        }
    }

    impl Drop for PrivateBus {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    fn name_has_owner(address: &str) -> bool {
        let connection = zbus::block_on(
            zbus::connection::Builder::address(address)
                .expect("private bus address")
                .build(),
        )
        .expect("connect to private bus");
        zbus::block_on(connection.call_method(
            Some("org.freedesktop.DBus"),
            "/org/freedesktop/DBus",
            Some("org.freedesktop.DBus"),
            "NameHasOwner",
            &WATCHER_NAME,
        ))
        .expect("NameHasOwner reply")
        .body()
        .deserialize::<bool>()
        .expect("NameHasOwner bool")
    }

    fn wait_for_name(address: &str) -> bool {
        let end = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while std::time::Instant::now() < end {
            if name_has_owner(address) {
                return true;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        false
    }

    /// The daemon releases a name asynchronously after the connection closes,
    /// so a single immediate check races the daemon's socket teardown.
    fn wait_for_name_release(address: &str) -> bool {
        let end = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while std::time::Instant::now() < end {
            if !name_has_owner(address) {
                return true;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        false
    }

    #[test]
    fn service_forms_resolve_to_a_bus_and_path() {
        assert_eq!(service_bus(":1.42", None), ":1.42");
        assert_eq!(
            service_bus("org.kde.StatusNotifierItem-9-1/StatusNotifierItem", None),
            "org.kde.StatusNotifierItem-9-1"
        );
        assert_eq!(service_bus("/StatusNotifierItem", Some(":1.7")), ":1.7");
    }

    #[test]
    fn canonical_identity_separates_senders_and_paths() {
        let first = item_identity("/StatusNotifierItem", ":1.7");
        let second = item_identity("/StatusNotifierItem", ":1.8");
        assert_ne!(first, second);

        let bus = "org.kde.StatusNotifierItem-9-1";
        assert_ne!(
            item_identity(&format!("{bus}/StatusNotifierItem"), ""),
            item_identity(&format!("{bus}/OtherItem"), "")
        );

        assert_eq!(first, item_identity(":1.7/StatusNotifierItem", ""));
    }

    #[test]
    fn pixmap_converts_argb_network_order_to_rgba() {
        // One 2x1 image: opaque red, half-transparent blue.
        let pixmaps = [(2, 1, vec![0xFF, 0xFF, 0x00, 0x00, 0x80, 0x00, 0x00, 0xFF])];
        let icon = pixmap_icon(&pixmaps).expect("icon");
        assert_eq!((icon.width, icon.height), (2, 1));
        assert_eq!(
            icon.rgba,
            Arc::<[u8]>::from([0xFF, 0x00, 0x00, 0xFF, 0x00, 0x00, 0xFF, 0x80])
        );
    }

    #[test]
    fn icon_clones_share_pixel_storage() {
        let icon = pixmap_icon(&[(1, 1, vec![255, 1, 2, 3])]).expect("icon");
        let clone = icon.clone();
        assert!(Arc::ptr_eq(&icon.rgba, &clone.rgba));
    }

    #[test]
    fn service_releases_name_and_restarts_on_private_bus() {
        let _lock = DBUS_TEST_LOCK.lock().unwrap();
        let (_daemon, address) = PrivateBus::start();
        let (tx, _rx) = calloop::channel::channel::<Message>();
        for generation in 0..2 {
            let mut service =
                Service::start_with_address(generation, tx.clone(), Some(address.clone()))
                    .expect("start tray");
            assert!(wait_for_name(&address));
            service.stop();
            assert!(wait_for_name_release(&address), "name not released");
        }
    }

    #[test]
    fn stop_cancels_worker_promptly() {
        let _lock = DBUS_TEST_LOCK.lock().unwrap();
        let (_daemon, address) = PrivateBus::start();
        let (tx, _rx) = calloop::channel::channel::<Message>();
        let mut service =
            Service::start_with_address(5, tx, Some(address.clone())).expect("start tray");
        std::thread::sleep(std::time::Duration::from_millis(50));
        let start = std::time::Instant::now();
        service.stop();
        assert!(start.elapsed() < std::time::Duration::from_secs(1));
        assert!(wait_for_name_release(&address), "name not released");
    }

    #[test]
    fn stop_cancels_slow_property_and_activate_calls() {
        let _lock = DBUS_TEST_LOCK.lock().unwrap();
        let (_daemon, address) = PrivateBus::start();
        let owner = zbus::block_on(
            zbus::connection::Builder::address(address.as_str())
                .unwrap()
                .build(),
        )
        .expect("private bus connection");
        zbus::block_on(owner.request_name("org.example.SlowItem")).expect("claim fake item name");
        let title_started = Arc::new(TestSignal::new());
        let activate_started = Arc::new(TestSignal::new());
        zbus::block_on(owner.object_server().at(
            "/SlowItem",
            SlowItem {
                title_started: title_started.clone(),
                activate_started: activate_started.clone(),
            },
        ))
        .unwrap();
        let (tx, _rx) = calloop::channel::channel::<Message>();
        let mut service =
            Service::start_with_address(8, tx, Some(address.clone())).expect("start tray");
        let item = TrayItem {
            id: 0,
            title: String::new(),
            service: "org.example.SlowItem".into(),
            path: "/SlowItem".into(),
            icon: None,
        };
        service
            .worker
            .as_ref()
            .expect("worker")
            .0
            .try_send(Command::Register {
                service: "org.example.SlowItem/SlowItem".into(),
                bus: "org.example.SlowItem".into(),
            })
            .expect("queue fake registration");
        assert!(
            title_started
                .wait_timeout(std::time::Duration::from_secs(2))
                .is_ok()
        );
        let start = std::time::Instant::now();
        service.stop();
        assert!(start.elapsed() < std::time::Duration::from_millis(500));
        assert!(wait_for_name_release(&address), "name not released");

        // New service; activate request remains pending on fake method until cancellation.
        let (tx, _rx) = calloop::channel::channel::<Message>();
        let mut service =
            Service::start_with_address(9, tx, Some(address.clone())).expect("restart tray");
        service.activate(&item).expect("queue activation");
        assert!(
            activate_started
                .wait_timeout(std::time::Duration::from_secs(2))
                .is_ok()
        );
        let start = std::time::Instant::now();
        service.stop();
        assert!(start.elapsed() < std::time::Duration::from_millis(500));
        assert!(wait_for_name_release(&address), "name not released");
        drop(owner);
    }

    #[test]
    fn activation_queue_reports_overflow() {
        let (commands, receiver) = async_channel::bounded(1);
        let (cancel, cancel_rx) = async_channel::bounded(1);
        let worker = thread::spawn(move || {
            let _ = cancel_rx.recv_blocking();
            drop(receiver);
        });
        let service = Service {
            worker: Some((commands, cancel, worker)),
        };
        let item = TrayItem {
            id: 0,
            title: String::new(),
            service: "org.example.Item".into(),
            path: DEFAULT_ITEM_PATH.into(),
            icon: None,
        };
        assert_eq!(service.activate(&item), Ok(()));
        assert_eq!(service.activate(&item), Err(ActivateError::Full));
    }

    #[test]
    fn activation_reports_stopped_when_the_worker_is_gone() {
        let item = TrayItem {
            id: 0,
            title: String::new(),
            service: "org.example.Item".into(),
            path: DEFAULT_ITEM_PATH.into(),
            icon: None,
        };
        // Closed command channel: the worker still exists but cannot receive.
        let (commands, receiver) = async_channel::bounded(1);
        drop(receiver);
        let (cancel, cancel_rx) = async_channel::bounded(1);
        let worker = thread::spawn(move || {
            let _ = cancel_rx.recv_blocking();
        });
        let service = Service {
            worker: Some((commands, cancel, worker)),
        };
        assert_eq!(service.activate(&item), Err(ActivateError::Stopped));

        // Finished worker: the sender would accept a command nobody reads.
        let (commands, _receiver) = async_channel::bounded(1);
        let (cancel, _cancel_rx) = async_channel::bounded(1);
        let worker = thread::spawn(|| {});
        while !worker.is_finished() {
            thread::yield_now();
        }
        let service = Service {
            worker: Some((commands, cancel, worker)),
        };
        assert_eq!(service.activate(&item), Err(ActivateError::Stopped));
    }

    #[test]
    fn empty_or_malformed_pixmaps_are_ignored() {
        assert!(pixmap_icon(&[]).is_none());
        assert!(pixmap_icon(&[(0, 2, vec![0; 8])]).is_none());
        assert!(pixmap_icon(&[(2, 2, vec![0; 4])]).is_none());
        assert!(theme_icon("hayu-no-such-icon-name").is_none());
    }

    #[test]
    fn huge_or_oversized_pixmaps_are_ignored_without_panicking() {
        assert!(pixmap_len(i32::MAX, i32::MAX).is_none());
        assert!(pixmap_icon(&[(i32::MAX, i32::MAX, Vec::new())]).is_none());
        assert!(pixmap_icon(&[(MAX_ICON_DIM as i32 + 1, 1, vec![0; 4])]).is_none());
        assert!(pixmap_icon(&[(1, MAX_ICON_DIM as i32 + 1, vec![0; 4])]).is_none());
        assert!(pixmap_icon(&[(-1, 1, vec![0; 4])]).is_none());
    }

    fn png_bytes(width: u32, height: u32) -> Vec<u8> {
        let mut bytes = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut bytes, width, height);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header().expect("png header");
            writer
                .write_image_data(&vec![0; (width * height * 4) as usize])
                .expect("png data");
        }
        bytes
    }

    #[test]
    fn png_over_dimension_cap_is_rejected() {
        assert!(decode_png_bytes(&png_bytes(MAX_ICON_DIM + 1, 1)).is_none());
        assert!(decode_png_bytes(&png_bytes(1, MAX_ICON_DIM + 1)).is_none());
        assert!(decode_png_bytes(&png_bytes(4, 4)).is_some());
    }

    #[test]
    fn oversized_png_file_is_rejected() {
        let path =
            std::env::temp_dir().join(format!("hayu-tray-png-cap-{}.png", std::process::id()));
        let file = std::fs::File::create(&path).expect("temp png");
        file.set_len(MAX_PNG_FILE_BYTES + 1).expect("sparse png");
        assert!(decode_png(&path).is_none());
        std::fs::remove_file(path).ok();
    }
}
