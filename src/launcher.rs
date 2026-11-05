use cosmic_text::{FontSystem, SwashCache};
use smithay_client_toolkit::{
    compositor::CompositorState,
    shell::{
        WaylandSurface,
        wlr_layer::{Anchor, KeyboardInteractivity, Layer, LayerShell, LayerSurface},
    },
    shm::{Shm, slot::SlotPool},
};
use tiny_skia::Pixmap;
use wayland_client::{
    Proxy, QueueHandle,
    protocol::{wl_output, wl_shm},
};

use crate::bar::State;
use crate::desktop::{self, Entry};
use crate::render::{self, Theme};

pub const WIDTH: u32 = 560;
pub const HEIGHT: u32 = 420;
const MARGIN_TOP: i32 = 80;
const VISIBLE_ROWS: usize = 8;

pub struct Surface {
    pub layer: LayerSurface,
    pool: SlotPool,
    pub output_id: u32,
    pub width: u32,
    pub height: u32,
    pub scale: i32,
    pub configured: bool,
}

impl Surface {
    pub fn create(
        compositor: &CompositorState,
        shell: &LayerShell,
        shm: &Shm,
        qh: &QueueHandle<State>,
        output: &wl_output::WlOutput,
        scale: i32,
    ) -> Result<Self, String> {
        let surface = compositor.create_surface(qh);
        let layer = shell.create_layer_surface(
            qh,
            surface,
            Layer::Overlay,
            Some("hayu-launcher"),
            Some(output),
        );
        layer.set_anchor(Anchor::TOP);
        layer.set_margin(MARGIN_TOP, 0, 0, 0);
        layer.set_size(WIDTH, HEIGHT);
        layer.set_exclusive_zone(0);
        layer.set_keyboard_interactivity(KeyboardInteractivity::Exclusive);
        layer.commit();
        let pool =
            SlotPool::new(1, shm).map_err(|e| format!("cannot create launcher shm pool: {e}"))?;
        Ok(Self {
            layer,
            pool,
            output_id: output.id().protocol_id(),
            width: WIDTH,
            height: HEIGHT,
            scale: scale.max(1),
            configured: false,
        })
    }

    pub fn owns(&self, layer: &LayerSurface) -> bool {
        self.layer.wl_surface() == layer.wl_surface()
    }

    fn hide(&mut self) {
        self.layer
            .set_keyboard_interactivity(KeyboardInteractivity::None);
        self.layer.wl_surface().attach(None, 0, 0);
        self.layer.commit();
        // wlroots requires a fresh configure before the next buffer, and a
        // configure event must not paint a closed launcher.
        self.configured = false;
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    None,
    Changed,
    Close,
    Launch(Entry),
}

pub struct Launcher {
    entries: Vec<Entry>,
    results: Vec<Entry>,
    query: String,
    selected: usize,
    error: Option<String>,
    open: bool,
    pub surface: Option<Surface>,
}

impl Launcher {
    pub fn new(entries: Vec<Entry>) -> Self {
        let results = desktop::filter(&entries, "");
        Self {
            entries,
            results,
            query: String::new(),
            selected: 0,
            error: None,
            open: false,
            surface: None,
        }
    }

    /// Replace the catalogue, e.g. after rescanning when the launcher opens.
    pub fn set_entries(&mut self, entries: Vec<Entry>) {
        self.entries = entries;
        self.recompute();
    }

    /// State for `HAYU_DEBUG_KEYS=1`: lets a live input test observe what the
    /// key produced without a screenshot tool. Never logged unless asked.
    pub fn debug_state(&self) -> (&str, usize, usize) {
        (&self.query, self.selected, self.results.len())
    }

    /// Names of the first three results, for `HAYU_DEBUG_KEYS` only.
    pub fn debug_results(&self) -> Vec<&str> {
        self.results.iter().take(3).map(|entry| entry.name.as_str()).collect()
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    pub fn begin_open(&mut self) {
        self.query.clear();
        self.selected = 0;
        self.error = None;
        self.recompute();
        self.open = true;
        if let Some(surface) = &mut self.surface {
            surface.configured = false;
            surface
                .layer
                .set_keyboard_interactivity(KeyboardInteractivity::Exclusive);
            // Commit without a buffer so the compositor sends a configure for
            // the new interactivity state before anything is attached.
            surface.layer.commit();
        }
    }

    pub fn show(&mut self, surface: Surface) {
        surface
            .layer
            .set_keyboard_interactivity(KeyboardInteractivity::Exclusive);
        self.surface = Some(surface);
        self.open = true;
    }

    pub fn close(&mut self) {
        if let Some(surface) = &mut self.surface {
            surface.hide();
        }
        self.open = false;
    }

    pub fn on_layer_closed(&mut self, layer: &LayerSurface) -> bool {
        if self
            .surface
            .as_ref()
            .is_some_and(|surface| surface.owns(layer))
        {
            self.surface = None;
            self.open = false;
            true
        } else {
            false
        }
    }

    pub fn forget_surface(&mut self) {
        self.surface = None;
        self.open = false;
    }

    pub fn resize(&mut self, width: u32, height: u32) -> bool {
        let Some(surface) = &mut self.surface else {
            return false;
        };
        surface.width = if width == 0 { WIDTH } else { width };
        surface.height = if height == 0 { HEIGHT } else { height };
        surface.configured = true;
        true
    }

    pub fn set_scale(&mut self, scale: i32) -> bool {
        let Some(surface) = &mut self.surface else {
            return false;
        };
        surface.scale = scale.max(1);
        true
    }

    pub fn output_id(&self) -> Option<u32> {
        self.surface.as_ref().map(|surface| surface.output_id)
    }

    pub fn set_error(&mut self, error: String) {
        self.error = Some(error);
    }

    pub fn key(&mut self, event: &smithay_client_toolkit::seat::keyboard::KeyEvent) -> Action {
        if !self.open {
            return Action::None;
        }
        use xkbcommon::xkb::keysyms::{
            KEY_BackSpace as BACKSPACE, KEY_Down as DOWN, KEY_Escape as ESCAPE,
            KEY_KP_Enter as KP_ENTER, KEY_Return as RETURN, KEY_Up as UP,
        };
        match event.keysym.raw() {
            ESCAPE => Action::Close,
            UP => {
                if self.selected > 0 {
                    self.selected -= 1;
                }
                Action::Changed
            }
            DOWN => {
                if self.selected + 1 < self.results.len() {
                    self.selected += 1;
                }
                Action::Changed
            }
            RETURN | KP_ENTER => match self.results.get(self.selected) {
                Some(entry) => Action::Launch(entry.clone()),
                None => Action::None,
            },
            BACKSPACE => {
                if self.query.pop().is_some() {
                    self.recompute();
                    Action::Changed
                } else {
                    Action::None
                }
            }
            _ => {
                let Some(text) = &event.utf8 else {
                    return Action::None;
                };
                if text.is_empty() || text.chars().any(char::is_control) {
                    return Action::None;
                }
                self.query.push_str(text);
                self.recompute();
                Action::Changed
            }
        }
    }

    fn recompute(&mut self) {
        self.results = desktop::filter(&self.entries, &self.query);
        self.selected = self.selected.min(self.results.len().saturating_sub(1));
    }

    pub fn draw(&mut self, theme: &Theme, fonts: &mut FontSystem, cache: &mut SwashCache) {
        let Some(surface) = &mut self.surface else {
            return;
        };
        if !self.open || !surface.configured {
            return;
        }
        let Some(pixmap) = draw_frame(
            theme,
            &self.query,
            &self.results,
            self.selected,
            self.error.as_deref(),
            surface.width,
            surface.height,
            surface.scale,
            fonts,
            cache,
        ) else {
            return;
        };
        let width = surface.width.saturating_mul(surface.scale as u32);
        let height = surface.height.saturating_mul(surface.scale as u32);
        let stride = (width * 4) as i32;
        let (buffer, canvas) = match surface.pool.create_buffer(
            width as i32,
            height as i32,
            stride,
            wl_shm::Format::Argb8888,
        ) {
            Ok(pair) => pair,
            Err(error) => {
                eprintln!("hayu: cannot create launcher shm buffer: {error}");
                return;
            }
        };
        render::copy_to_shm(&pixmap, canvas);
        let wl_surface = surface.layer.wl_surface();
        let _ = surface.layer.set_buffer_scale(surface.scale as u32);
        wl_surface.damage_buffer(0, 0, width as i32, height as i32);
        if let Err(error) = buffer.attach_to(wl_surface) {
            eprintln!("hayu: cannot attach launcher buffer: {error}");
            return;
        }
        surface.layer.commit();
    }
}

#[allow(clippy::too_many_arguments)]
pub fn draw_frame(
    theme: &Theme,
    query: &str,
    results: &[Entry],
    selected: usize,
    error: Option<&str>,
    width: u32,
    height: u32,
    scale: i32,
    fonts: &mut FontSystem,
    cache: &mut SwashCache,
) -> Option<Pixmap> {
    if width == 0 || height == 0 {
        return None;
    }
    let scale = scale.max(1) as f32;
    let pixel_width = width.saturating_mul(scale as u32);
    let pixel_height = height.saturating_mul(scale as u32);
    let mut pixmap = Pixmap::new(pixel_width, pixel_height)?;
    render::fill(
        &mut pixmap,
        0.0,
        0.0,
        pixel_width as f32,
        pixel_height as f32,
        theme.bg,
    );
    if fonts.db().is_empty() {
        return Some(pixmap);
    }

    let pad = 20.0 * scale;
    let input_size = 18.0 * scale;
    let input_y = 17.0 * scale;
    let divider_y = 56.0 * scale;
    render::fill(
        &mut pixmap,
        pad,
        divider_y,
        pixel_width as f32 - pad * 2.0,
        1.0 * scale,
        theme.muted,
    );

    let measure_input = |text: &str| render::text_width(fonts, text, input_size);
    let shown_query = render::ellipsize(query, pixel_width as f32 - pad * 3.0, measure_input);
    let query_x = pad;
    if query.is_empty() {
        render::draw_text(
            &mut pixmap,
            fonts,
            cache,
            "Search applications",
            query_x,
            input_y,
            input_size,
            theme.muted,
        );
    } else {
        render::draw_text(
            &mut pixmap,
            fonts,
            cache,
            &shown_query,
            query_x,
            input_y,
            input_size,
            theme.fg,
        );
    }
    let cursor_x = query_x + render::text_width(fonts, &shown_query, input_size) + 1.0 * scale;
    render::fill(
        &mut pixmap,
        cursor_x,
        input_y + 1.0 * scale,
        2.0 * scale,
        22.0 * scale,
        theme.accent,
    );

    let row_height = 36.0 * scale;
    let results_y = 70.0 * scale;
    if results.is_empty() {
        render::draw_text(
            &mut pixmap,
            fonts,
            cache,
            "No matching applications",
            pad,
            results_y + 8.0 * scale,
            14.0 * scale,
            theme.muted,
        );
    } else {
        let visible = results.len().min(VISIBLE_ROWS);
        let start = if selected < VISIBLE_ROWS {
            0
        } else {
            selected + 1 - VISIBLE_ROWS
        };
        let start = start.min(results.len().saturating_sub(visible));
        for (row, entry) in results[start..start + visible].iter().enumerate() {
            let index = start + row;
            let y = results_y + row as f32 * row_height;
            let selected_row = index == selected;
            if selected_row {
                render::fill(
                    &mut pixmap,
                    10.0 * scale,
                    y,
                    pixel_width as f32 - 20.0 * scale,
                    row_height - 4.0 * scale,
                    theme.accent,
                );
            }
            let text_color = if selected_row { theme.bg } else { theme.fg };
            let id_color = if selected_row { theme.bg } else { theme.muted };
            let id_size = 12.0 * scale;
            let max_id = pixel_width as f32 * 0.42;
            let id_text = render::ellipsize(&entry.desktop_id, max_id, |text| {
                render::text_width(fonts, text, id_size)
            });
            let id_width = render::text_width(fonts, &id_text, id_size);
            let name_max = pixel_width as f32 - pad * 2.0 - id_width - 20.0 * scale;
            let name = render::ellipsize(&entry.name, name_max.max(20.0), |text| {
                render::text_width(fonts, text, 14.0 * scale)
            });
            render::draw_text(
                &mut pixmap,
                fonts,
                cache,
                &name,
                pad,
                y + 8.0 * scale,
                14.0 * scale,
                text_color,
            );
            render::draw_text(
                &mut pixmap,
                fonts,
                cache,
                &id_text,
                pixel_width as f32 - pad - id_width,
                y + 10.0 * scale,
                id_size,
                id_color,
            );
        }
    }

    let footer_size = 12.0 * scale;
    let footer_y = pixel_height as f32 - 28.0 * scale;
    let footer = match error {
        Some(error) => error,
        None => "Up/Down select   Enter launch   Esc close",
    };
    let footer = render::ellipsize(footer, pixel_width as f32 - pad * 2.0, |text| {
        render::text_width(fonts, text, footer_size)
    });
    render::draw_text(
        &mut pixmap,
        fonts,
        cache,
        &footer,
        pad,
        footer_y,
        footer_size,
        if error.is_some() {
            theme.error
        } else {
            theme.muted
        },
    );

    Some(pixmap)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn entry(id: &str, name: &str) -> Entry {
        Entry {
            desktop_id: id.into(),
            name: name.into(),
            exec: vec!["app".into()],
            terminal: false,
        }
    }

    #[test]
    fn renders_launcher_frame_at_scale_two() {
        let entries = vec![
            entry("firefox.desktop", "Firefox"),
            entry("org.gnome.Files.desktop", "Files"),
            entry("alacritty.desktop", "Alacritty"),
        ];
        let results = desktop::filter(&entries, "fi");
        let theme = Theme::from_config("dark", "#8AB4F8");
        let mut fonts = FontSystem::new();
        let mut cache = SwashCache::new();
        let pixmap = draw_frame(
            &theme, "fi", &results, 0, None, WIDTH, HEIGHT, 2, &mut fonts, &mut cache,
        )
        .expect("launcher frame");
        assert_eq!(pixmap.width(), WIDTH * 2);
        assert_eq!(pixmap.height(), HEIGHT * 2);
        let accent = theme.accent;
        let has_accent = pixmap.data().chunks_exact(4).any(|px| {
            px[0] == accent[0] && px[1] == accent[1] && px[2] == accent[2] && px[3] == 255
        });
        assert!(
            has_accent,
            "selected row or cursor should use the accent color"
        );
        write_ppm("/tmp/hayu-launcher.ppm", &pixmap);
    }

    fn write_ppm(path: &str, pixmap: &Pixmap) {
        let mut file = std::fs::File::create(path).expect("create ppm");
        write!(file, "P6\n{} {}\n255\n", pixmap.width(), pixmap.height()).unwrap();
        for px in pixmap.data().chunks_exact(4) {
            file.write_all(&px[..3]).unwrap();
        }
    }

    #[test]
    fn key_actions_are_deterministic() {
        use smithay_client_toolkit::seat::keyboard::KeyEvent;
        use xkbcommon::xkb::keysyms;

        let entries = vec![
            entry("a.desktop", "Alpha"),
            entry("b.desktop", "Beta"),
            entry("c.desktop", "Gamma"),
        ];
        let mut launcher = Launcher::new(entries);
        launcher.begin_open();
        let key = |sym: u32, text: Option<&str>| KeyEvent {
            time: 0,
            raw_code: 0,
            keysym: xkbcommon::xkb::Keysym::new(sym),
            utf8: text.map(str::to_string),
        };
        assert_eq!(launcher.key(&key(keysyms::KEY_Down, None)), Action::Changed);
        assert_eq!(launcher.key(&key(keysyms::KEY_Down, None)), Action::Changed);
        assert_eq!(launcher.key(&key(keysyms::KEY_Down, None)), Action::Changed);
        assert_eq!(launcher.key(&key(keysyms::KEY_Up, None)), Action::Changed);
        assert_eq!(launcher.key(&key(keysyms::KEY_Escape, None)), Action::Close);
        launcher.close();
        assert_eq!(launcher.key(&key(keysyms::KEY_Escape, None)), Action::None);
        launcher.begin_open();
        assert_eq!(
            launcher.key(&key(keysyms::KEY_Return, None)),
            Action::Launch(entry("a.desktop", "Alpha"))
        );
    }
}
