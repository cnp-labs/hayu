use std::os::raw::c_void;

use cosmic_text::FontSystem;
use smithay_client_toolkit::{
    compositor::CompositorState,
    shell::{
        WaylandSurface,
        wlr_layer::{Anchor, KeyboardInteractivity, Layer, LayerShell, LayerSurface},
    },
};
use wayland_client::{Proxy, QueueHandle, protocol::wl_output};
use wayland_protocols::wp::fractional_scale::v1::client::{
    wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1,
    wp_fractional_scale_v1::WpFractionalScaleV1,
};
use wayland_protocols::wp::viewporter::client::{
    wp_viewport::WpViewport, wp_viewporter::WpViewporter,
};

use crate::app::State;
use crate::desktop::{self, Entry};
use crate::render::{self, Atlas, Frame, Theme, gl::GlPresenter};

pub const WIDTH: u32 = 560;
pub const HEIGHT: u32 = 420;
const MARGIN_TOP: i32 = 80;
const VISIBLE_ROWS: usize = 8;
/// Logical surface metrics shared with the text-input cursor rectangle.
pub const PAD: f32 = 20.0;
pub const INPUT_SIZE: f32 = 18.0;
pub const INPUT_Y: f32 = 17.0;

pub struct Surface {
    // Must drop before `layer`: the EGL surface wraps `layer`'s wl_surface.
    presenter: Option<GlPresenter>,
    atlas: Atlas,
    pub layer: LayerSurface,
    pub output_id: u32,
    pub width: u32,
    pub height: u32,
    /// Pixels per logical unit; fractional when the compositor offers it.
    pub scale: f32,
    fractional: Option<WpFractionalScaleV1>,
    viewport: Option<WpViewport>,
    pub configured: bool,
}

impl Surface {
    pub fn create(
        compositor: &CompositorState,
        shell: &LayerShell,
        qh: &QueueHandle<State>,
        output: &wl_output::WlOutput,
        scale: f32,
        fractional_manager: Option<&WpFractionalScaleManagerV1>,
        viewporter: Option<&WpViewporter>,
    ) -> Result<Self, String> {
        let surface = compositor.create_surface(qh);
        // Both objects must exist before the surface's first commit.
        let fractional = fractional_manager.map(|manager| {
            manager.get_fractional_scale(&surface, qh, crate::bar::ScaleTarget::Launcher)
        });
        let viewport = viewporter
            .map(|viewporter| viewporter.get_viewport(&surface, qh, crate::bar::NoEvents));
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
        Ok(Self {
            presenter: None,
            atlas: Atlas::new(),
            layer,
            output_id: output.id().protocol_id(),
            width: WIDTH,
            height: HEIGHT,
            scale: scale.max(1.0),
            fractional,
            viewport,
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
    preedit: String,
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
            preedit: String::new(),
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
        self.results
            .iter()
            .take(3)
            .map(|entry| entry.name.as_str())
            .collect()
    }

    pub fn is_open(&self) -> bool {
        self.open
    }
    pub fn query(&self) -> &str {
        &self.query
    }
    pub fn preedit(&self) -> &str {
        &self.preedit
    }
    /// Result rows as currently filtered, for the accessibility tree.
    pub fn results(&self) -> &[Entry] {
        &self.results
    }
    pub fn selected(&self) -> usize {
        self.selected
    }
    /// Append text committed by the input method at the end of the query.
    pub fn insert_text(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        self.query.push_str(text);
        self.recompute();
    }
    /// Remove `bytes` bytes just before the caret, stopping on a char boundary.
    pub fn delete_before(&mut self, bytes: u32) {
        if bytes == 0 {
            return;
        }
        let mut cut = self.query.len().saturating_sub(bytes as usize);
        while cut > 0 && !self.query.is_char_boundary(cut) {
            cut -= 1;
        }
        self.query.truncate(cut);
        self.recompute();
    }
    pub fn set_preedit(&mut self, text: String) {
        self.preedit = text;
    }

    pub fn begin_open(&mut self) {
        self.query.clear();
        self.preedit.clear();
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
        self.preedit.clear();
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

    /// Fractional or integer scale from the compositor's own signal.
    pub fn set_scale(&mut self, scale: f32) -> bool {
        let Some(surface) = &mut self.surface else {
            return false;
        };
        surface.scale = scale.max(1.0);
        true
    }
    /// Integer output scale, used only when fractional scale is unavailable.
    pub fn set_output_scale(&mut self, scale: i32) -> bool {
        let Some(surface) = &mut self.surface else {
            return false;
        };
        if surface.fractional.is_some() {
            return false;
        }
        surface.scale = scale.max(1) as f32;
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
            // The input method may leave a preedit on screen, so Escape drops
            // that before it can leave the launcher.
            ESCAPE if !self.preedit.is_empty() => {
                self.preedit.clear();
                Action::Changed
            }
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

    pub fn draw(&mut self, theme: &Theme, fonts: &mut FontSystem, display_ptr: *mut c_void) {
        let Some(surface) = &mut self.surface else {
            return;
        };
        if !self.open || !surface.configured {
            return;
        }
        let Some(frame) = draw_frame(
            theme,
            &self.query,
            &self.preedit,
            &self.results,
            self.selected,
            self.error.as_deref(),
            surface.width,
            surface.height,
            surface.scale,
            fonts,
            &mut surface.atlas,
        ) else {
            return;
        };
        let width = crate::bar::pixel_size(surface.width, surface.scale);
        let height = crate::bar::pixel_size(surface.height, surface.scale);
        let wl_surface = surface.layer.wl_surface();
        crate::bar::apply_scale(
            wl_surface,
            surface.viewport.as_ref(),
            surface.width,
            surface.height,
            surface.scale,
        );
        if surface.presenter.is_none() {
            let surface_ptr = wl_surface.id().as_ptr() as *mut c_void;
            match GlPresenter::new(display_ptr, surface_ptr, width, height) {
                Ok(presenter) => surface.presenter = Some(presenter),
                Err(error) => {
                    eprintln!("hayu: cannot create launcher GL surface: {error}");
                    return;
                }
            }
        }
        let Some(presenter) = surface.presenter.as_mut() else {
            return;
        };
        if let Err(error) = presenter.render(&frame, &surface.atlas) {
            eprintln!("hayu: launcher render failed: {error}");
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub fn draw_frame(
    theme: &Theme,
    query: &str,
    preedit: &str,
    results: &[Entry],
    selected: usize,
    error: Option<&str>,
    width: u32,
    height: u32,
    scale: f32,
    fonts: &mut FontSystem,
    atlas: &mut Atlas,
) -> Option<Frame> {
    let scale = scale.max(1.0);
    let pixel_width = crate::bar::pixel_size(width, scale);
    let pixel_height = crate::bar::pixel_size(height, scale);
    atlas.start_frame();
    let mut frame = Frame::new(pixel_width, pixel_height, theme.bg)?;
    if fonts.db().is_empty() {
        return Some(frame);
    }

    let pad = PAD * scale;
    let input_size = INPUT_SIZE * scale;
    let input_y = INPUT_Y * scale;
    let divider_y = 56.0 * scale;
    frame.fill(
        pad,
        divider_y,
        pixel_width as f32 - pad * 2.0,
        1.0 * scale,
        theme.muted,
    );

    let shown_query = render::ellipsize(query, pixel_width as f32 - pad * 3.0, |text| {
        render::text_width(fonts, text, input_size)
    });
    let query_width = render::text_width(fonts, &shown_query, input_size);
    let shown_preedit = render::ellipsize(
        preedit,
        (pixel_width as f32 - pad - (pad + query_width)).max(0.0),
        |text| render::text_width(fonts, text, input_size),
    );
    let query_x = pad;
    if query.is_empty() && preedit.is_empty() {
        frame.text(
            fonts,
            atlas,
            "Search applications",
            query_x,
            input_y,
            input_size,
            theme.muted,
        );
    } else {
        frame.text(
            fonts,
            atlas,
            &shown_query,
            query_x,
            input_y,
            input_size,
            theme.fg,
        );
    }
    let query_end = query_x + query_width;
    if !shown_preedit.is_empty() {
        frame.text(
            fonts,
            atlas,
            &shown_preedit,
            query_end,
            input_y,
            input_size,
            theme.accent,
        );
    }
    let cursor_x = query_end + render::text_width(fonts, &shown_preedit, input_size) + 1.0 * scale;
    frame.fill(
        cursor_x,
        input_y + 1.0 * scale,
        2.0 * scale,
        22.0 * scale,
        theme.accent,
    );

    let row_height = 36.0 * scale;
    let results_y = 70.0 * scale;
    if results.is_empty() {
        frame.text(
            fonts,
            atlas,
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
                frame.fill(
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
            frame.text(
                fonts,
                atlas,
                &name,
                pad,
                y + 8.0 * scale,
                14.0 * scale,
                text_color,
            );
            frame.text(
                fonts,
                atlas,
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
    frame.text(
        fonts,
        atlas,
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

    Some(frame)
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
        let mut atlas = Atlas::new();
        let frame = draw_frame(
            &theme, "fi", "", &results, 0, None, WIDTH, HEIGHT, 2.0, &mut fonts, &mut atlas,
        )
        .expect("launcher frame");
        let pixmap = render::rasterize(&frame, &atlas).expect("launcher pixels");
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

    #[test]
    fn fractional_scale_sizes_the_frame_in_pixels() {
        assert_eq!(crate::bar::pixel_size(WIDTH, 1.5), 840);
        assert_eq!(crate::bar::pixel_size(HEIGHT, 1.5), 630);
        assert_eq!(crate::bar::pixel_size(1366, 1.25), 1708);
        assert_eq!(crate::bar::pixel_size(32, 1.5), 48);
        assert_eq!(crate::bar::pixel_size(1, 1.5), 2);
        let theme = Theme::from_config("dark", "#8AB4F8");
        let mut fonts = FontSystem::new();
        let mut atlas = Atlas::new();
        let frame = draw_frame(
            &theme,
            "fi",
            "",
            &[],
            0,
            None,
            WIDTH,
            HEIGHT,
            1.5,
            &mut fonts,
            &mut atlas,
        )
        .expect("launcher frame at 1.5");
        assert_eq!((frame.width, frame.height), (840, 630));
        let pixmap = render::rasterize(&frame, &atlas).expect("launcher pixels at 1.5");
        assert_eq!((pixmap.width(), pixmap.height()), (840, 630));
    }
    fn write_ppm(path: &str, pixmap: &tiny_skia::Pixmap) {
        let mut file = std::fs::File::create(path).expect("create ppm");
        write!(file, "P6\n{} {}\n255\n", pixmap.width(), pixmap.height()).unwrap();
        for px in pixmap.data().chunks_exact(4) {
            file.write_all(&px[..3]).unwrap();
        }
    }

    #[test]
    fn escape_clears_preedit_before_closing() {
        use smithay_client_toolkit::seat::keyboard::KeyEvent;
        let escape = KeyEvent {
            time: 0,
            raw_code: 0,
            keysym: xkbcommon::xkb::Keysym::new(xkbcommon::xkb::keysyms::KEY_Escape),
            utf8: None,
        };
        let mut launcher = Launcher::new(vec![entry("a.desktop", "Alpha")]);
        launcher.begin_open();
        launcher.set_preedit("にほ".into());
        assert_eq!(launcher.key(&escape), Action::Changed);
        assert_eq!(launcher.preedit(), "");
        assert_eq!(launcher.key(&escape), Action::Close);
    }
    #[test]
    fn ime_edits_query_on_char_boundaries() {
        let mut launcher = Launcher::new(vec![entry("a.desktop", "Alpha")]);
        launcher.begin_open();
        launcher.insert_text("日本");
        launcher.set_preedit("にほ".into());
        assert_eq!(launcher.query(), "日本");
        assert_eq!(launcher.preedit(), "にほ");
        launcher.delete_before(3);
        assert_eq!(launcher.query(), "日", "delete stops on a char boundary");
        launcher.delete_before(100);
        assert_eq!(launcher.query(), "");
        launcher.close();
        assert_eq!(launcher.preedit(), "", "close drops the preedit");
        launcher.begin_open();
        assert_eq!(launcher.query(), "");
        assert_eq!(launcher.preedit(), "", "reopen starts clean");
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
