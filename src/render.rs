use std::collections::HashMap;

use chrono::{NaiveTime, Timelike};
use cosmic_text::{
    Attrs, Buffer, CacheKey, FontSystem, Metrics, Shaping, SwashCache, SwashContent,
};
use tiny_skia::{Color as SkColor, Pixmap};

use crate::config;
use crate::hyprland::Workspace;

pub mod gl;

/// Backend-neutral draw list. No backend shapes text or rasterizes glyphs: the
/// builder does that once and stores the bitmaps in [`Atlas`].
pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub bg: [u8; 3],
    pub rects: Vec<RectCmd>,
    pub glyphs: Vec<Glyph>,
}

#[derive(Clone, Copy, Debug)]
pub struct RectCmd {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub color: [u8; 3],
}

#[derive(Clone, Copy, Debug)]
pub struct Glyph {
    pub x: f32,
    pub y: f32,
    pub w: u32,
    pub h: u32,
    /// Atlas texel origin, used by the CPU rasterizer.
    pub ax: u32,
    pub ay: u32,
    pub uv: [f32; 4],
    pub color: [u8; 3],
}

impl Frame {
    pub fn new(width: u32, height: u32, bg: [u8; 3]) -> Option<Self> {
        if width == 0 || height == 0 {
            return None;
        }
        Some(Self {
            width,
            height,
            bg,
            rects: Vec::new(),
            glyphs: Vec::new(),
        })
    }

    pub fn fill(&mut self, x: f32, y: f32, w: f32, h: f32, color: [u8; 3]) {
        self.rects.push(RectCmd { x, y, w, h, color });
    }

    #[allow(clippy::too_many_arguments)]
    pub fn text(
        &mut self,
        fonts: &mut FontSystem,
        atlas: &mut Atlas,
        text: &str,
        x: f32,
        y: f32,
        size: f32,
        color: [u8; 3],
    ) {
        if text.is_empty() {
            return;
        }
        let mut buffer = Buffer::new(fonts, Metrics::new(size, size * 1.3));
        buffer.set_text(text, &Attrs::new(), Shaping::Advanced, None);
        buffer.set_size(None, Some(size * 1.3));
        buffer.shape_until_scroll(fonts, false);
        for run in buffer.layout_runs() {
            for glyph in run.glyphs {
                let physical = glyph.physical((x, y + run.line_y), 1.0);
                let Some(slot) = atlas.slot(fonts, physical.cache_key) else {
                    continue;
                };
                self.glyphs.push(Glyph {
                    x: (physical.x + slot.left) as f32,
                    y: (physical.y - slot.top) as f32,
                    w: slot.w,
                    h: slot.h,
                    ax: slot.ax,
                    ay: slot.ay,
                    uv: slot.uv,
                    color,
                });
            }
        }
    }
}

#[derive(Clone, Copy)]
struct Slot {
    ax: u32,
    ay: u32,
    w: u32,
    h: u32,
    left: i32,
    top: i32,
    uv: [f32; 4],
}

/// Dynamic glyph atlas. A glyph is rasterized the first time it is drawn and
/// then reused by every later frame. One surface's atlas is 1024×1024, which
/// holds thousands of unique glyphs; the destructive reset on overflow is a
/// ponytail ceiling, upgrade by growing the texture if a real frame ever hits it.
pub struct Atlas {
    pub width: u32,
    pub height: u32,
    /// RGBA8, white RGB with coverage in alpha.
    pub pixels: Vec<u8>,
    /// Bumped on every change so a presenter knows when to re-upload.
    pub version: u32,
    slots: HashMap<CacheKey, Slot>,
    cache: SwashCache,
    pen_x: u32,
    pen_y: u32,
    row_h: u32,
}

const ATLAS_SIZE: u32 = 1024;
const PAD: u32 = 1;

impl Atlas {
    pub fn new() -> Self {
        Self {
            width: ATLAS_SIZE,
            height: ATLAS_SIZE,
            pixels: vec![0u8; (ATLAS_SIZE * ATLAS_SIZE * 4) as usize],
            version: 0,
            slots: HashMap::new(),
            cache: SwashCache::new(),
            pen_x: PAD,
            pen_y: PAD,
            row_h: 0,
        }
    }

    fn slot(&mut self, fonts: &mut FontSystem, key: CacheKey) -> Option<Slot> {
        if let Some(slot) = self.slots.get(&key) {
            return Some(*slot);
        }
        let Some(image) = self.cache.get_image(fonts, key).clone() else {
            return None;
        };
        let (w, h) = (image.placement.width, image.placement.height);
        if w == 0 || h == 0 {
            return None;
        }
        if self.pen_x + w + PAD > self.width {
            self.pen_x = PAD;
            self.pen_y += self.row_h + PAD;
            self.row_h = 0;
        }
        if self.pen_y + h + PAD > self.height {
            // Full: drop everything and start over rather than grow.
            self.reset();
        }
        if self.pen_y + h + PAD > self.height {
            return None;
        }
        for gy in 0..h {
            for gx in 0..w {
                let index = (gy * w + gx) as usize;
                let (r, g, b, a) = match image.content {
                    SwashContent::Mask => (255, 255, 255, image.data[index]),
                    SwashContent::Color => {
                        let i = index * 4;
                        (
                            image.data[i],
                            image.data[i + 1],
                            image.data[i + 2],
                            image.data[i + 3],
                        )
                    }
                    SwashContent::SubpixelMask => {
                        let i = index * 3;
                        let alpha = image.data[i..i + 3].iter().copied().max().unwrap_or(0);
                        (255, 255, 255, alpha)
                    }
                };
                let dst = (((self.pen_y + gy) * self.width + self.pen_x + gx) * 4) as usize;
                self.pixels[dst] = r;
                self.pixels[dst + 1] = g;
                self.pixels[dst + 2] = b;
                self.pixels[dst + 3] = a;
            }
        }
        let slot = Slot {
            ax: self.pen_x,
            ay: self.pen_y,
            w,
            h,
            left: image.placement.left,
            top: image.placement.top,
            uv: [
                self.pen_x as f32 / self.width as f32,
                self.pen_y as f32 / self.height as f32,
                (self.pen_x + w) as f32 / self.width as f32,
                (self.pen_y + h) as f32 / self.height as f32,
            ],
        };
        self.pen_x += w + PAD;
        self.row_h = self.row_h.max(h);
        self.version = self.version.wrapping_add(1);
        self.slots.insert(key, slot);
        Some(slot)
    }

    fn reset(&mut self) {
        self.pixels.fill(0);
        self.slots.clear();
        self.pen_x = PAD;
        self.pen_y = PAD;
        self.row_h = 0;
        self.version = self.version.wrapping_add(1);
    }
}

impl Default for Atlas {
    fn default() -> Self {
        Self::new()
    }
}

/// Reference rasterizer: used by unit tests and only reachable from them.
pub fn rasterize(frame: &Frame, atlas: &Atlas) -> Option<Pixmap> {
    let mut pixmap = Pixmap::new(frame.width, frame.height)?;
    let bg = frame.bg;
    pixmap.fill(SkColor::from_rgba8(bg[0], bg[1], bg[2], 255));
    let width = frame.width as i32;
    let height = frame.height as i32;
    let data = pixmap.data_mut();

    for rect in &frame.rects {
        let x0 = (rect.x.round() as i32).clamp(0, width);
        let y0 = (rect.y.round() as i32).clamp(0, height);
        let x1 = ((rect.x + rect.w).round() as i32).clamp(0, width);
        let y1 = ((rect.y + rect.h).round() as i32).clamp(0, height);
        for py in y0..y1 {
            for px in x0..x1 {
                let index = ((py * width + px) * 4) as usize;
                data[index] = rect.color[0];
                data[index + 1] = rect.color[1];
                data[index + 2] = rect.color[2];
                data[index + 3] = 255;
            }
        }
    }

    for glyph in &frame.glyphs {
        for dy in 0..glyph.h as i32 {
            for dx in 0..glyph.w as i32 {
                let px = glyph.x.round() as i32 + dx;
                let py = glyph.y.round() as i32 + dy;
                if px < 0 || py < 0 || px >= width || py >= height {
                    continue;
                }
                let texel =
                    (((glyph.ay + dy as u32) * atlas.width + glyph.ax + dx as u32) * 4) as usize;
                let alpha = atlas.pixels[texel + 3] as u32;
                if alpha == 0 {
                    continue;
                }
                let index = ((py * width + px) * 4) as usize;
                let inverse = 255 - alpha;
                for channel in 0..3 {
                    data[index + channel] = ((glyph.color[channel] as u32 * alpha
                        + data[index + channel] as u32 * inverse)
                        / 255) as u8;
                }
                data[index + 3] = (alpha + data[index + 3] as u32 * inverse / 255).min(255) as u8;
            }
        }
    }

    Some(pixmap)
}

#[derive(Debug, Clone)]
pub struct Theme {
    pub bg: [u8; 3],
    pub fg: [u8; 3],
    pub muted: [u8; 3],
    pub accent: [u8; 3],
    pub error: [u8; 3],
}

impl Theme {
    pub fn from_config(mode: &str, accent: &str) -> Self {
        let (bg, fg, muted, error) = if mode == "light" {
            (
                [0xF5, 0xF5, 0xF5],
                [0x1E, 0x1E, 0x2E],
                [0x6C, 0x6F, 0x85],
                [0xA5, 0x1C, 0x30],
            )
        } else {
            (
                [0x1E, 0x1E, 0x2E],
                [0xCD, 0xD6, 0xF4],
                [0x6C, 0x70, 0x86],
                [0xFF, 0xB4, 0xAB],
            )
        };
        Self {
            bg,
            fg,
            muted,
            accent: config::parse_color(accent),
            error,
        }
    }
}

pub struct Scene<'a> {
    pub width: u32,
    pub height: u32,
    pub scale: f32,
    pub theme: &'a Theme,
    pub clock: &'a str,
    pub title: &'a str,
    pub workspaces: &'a [Workspace],
    pub active_workspace: i64,
}

pub fn build(scene: &Scene<'_>, fonts: &mut FontSystem, atlas: &mut Atlas) -> Option<Frame> {
    let mut frame = Frame::new(scene.width, scene.height, scene.theme.bg)?;

    if fonts.db().is_empty() {
        // No system fonts: keep the bar usable (solid background) instead of panicking.
        return Some(frame);
    }

    let scale = scene.scale.max(1.0);
    let font_size = 13.0 * scale;
    let text_h = font_size * 1.3;
    let text_y = ((scene.height as f32) - text_h) / 2.0;
    let fg = scene.theme.fg;

    let mut x = 8.0 * scale;
    let left_limit = scene.width as f32 * 0.45;
    for ws in scene.workspaces {
        let label = if ws.name.is_empty() {
            ws.id.to_string()
        } else {
            ws.name.clone()
        };
        let width = text_width(fonts, &label, font_size);
        let pill = width + 12.0 * scale;
        if x + pill > left_limit {
            break;
        }
        let active = ws.id == scene.active_workspace;
        if active {
            frame.fill(x, 4.0 * scale, pill, scene.height as f32 - 8.0 * scale, scene.theme.accent);
        }
        let color = if active { scene.theme.bg } else { fg };
        frame.text(
            fonts,
            atlas,
            &label,
            x + 6.0 * scale,
            text_y,
            font_size,
            color,
        );
        x += pill + 4.0 * scale;
    }

    let clock_w = text_width(fonts, scene.clock, font_size);
    let clock_x = (((scene.width as f32) - clock_w) / 2.0).max(0.0);
    frame.text(fonts, atlas, scene.clock, clock_x, text_y, font_size, fg);

    let title_x = clock_x + clock_w + 12.0 * scale;
    let title_max = (scene.width as f32 - 8.0 * scale) - title_x;
    if !scene.title.is_empty() && title_max > 16.0 * scale {
        let title = ellipsize(scene.title, title_max, |s| text_width(fonts, s, font_size));
        frame.text(
            fonts,
            atlas,
            &title,
            title_x,
            text_y,
            font_size,
            scene.theme.muted,
        );
    }

    Some(frame)
}

pub(crate) fn text_width(fonts: &mut FontSystem, text: &str, size: f32) -> f32 {
    let mut buffer = Buffer::new(fonts, Metrics::new(size, size * 1.3));
    buffer.set_text(text, &Attrs::new(), Shaping::Advanced, None);
    buffer.shape_until_scroll(fonts, false);
    buffer
        .layout_runs()
        .map(|run| run.line_w)
        .fold(0.0, f32::max)
}

pub fn ellipsize(text: &str, max_width: f32, mut measure: impl FnMut(&str) -> f32) -> String {
    if text.is_empty() || measure(text) <= max_width {
        return text.to_string();
    }
    if measure("…") > max_width {
        return String::new();
    }
    let chars: Vec<char> = text.chars().collect();
    let mut low = 0;
    let mut high = chars.len();
    while low < high {
        let mid = (low + high).div_ceil(2);
        let mut candidate: String = chars[..mid].iter().collect();
        candidate.push('…');
        if measure(&candidate) <= max_width {
            low = mid;
        } else {
            high = mid - 1;
        }
    }
    let mut result: String = chars[..low].iter().collect();
    result.push('…');
    result
}

pub fn format_clock(format: &str, time: NaiveTime) -> String {
    let mut out = String::new();
    let mut chars = format.chars();
    while let Some(ch) = chars.next() {
        if ch != '%' {
            out.push(ch);
            continue;
        }
        match chars.next() {
            Some('H') => out.push_str(&format!("{:02}", time.hour())),
            Some('M') => out.push_str(&format!("{:02}", time.minute())),
            Some('S') => out.push_str(&format!("{:02}", time.second())),
            Some('%') => out.push('%'),
            Some(other) => {
                out.push('%');
                out.push(other);
            }
            None => out.push('%'),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake_measure(text: &str) -> f32 {
        text.chars().count() as f32 * 10.0
    }

    #[test]
    fn ellipsis_fits_width() {
        assert_eq!(ellipsize("abcdef", 100.0, fake_measure), "abcdef");
        assert_eq!(ellipsize("abcdef", 45.0, fake_measure), "abc…");
        assert_eq!(ellipsize("abcdef", 10.0, fake_measure), "…");
        assert_eq!(ellipsize("abcdef", 5.0, fake_measure), "");
        let out = ellipsize("hello world", 65.0, fake_measure);
        assert!(out.ends_with('…'));
        assert!(fake_measure(&out) <= 65.0);
    }

    #[test]
    fn clock_minute_boundary() {
        let before = NaiveTime::from_hms_opt(9, 59, 59).unwrap();
        let after = NaiveTime::from_hms_opt(10, 0, 0).unwrap();
        assert_eq!(format_clock("%H:%M", before), "09:59");
        assert_eq!(format_clock("%H:%M", after), "10:00");
        assert_eq!(format_clock("%H.%M %%", before), "09.59 %");
    }

    #[test]
    fn frame_reuses_glyphs_from_the_atlas() {
        let mut fonts = FontSystem::new();
        if fonts.db().is_empty() {
            return;
        }
        let mut atlas = Atlas::new();
        let mut frame = Frame::new(120, 32, [0, 0, 0]).unwrap();
        frame.text(&mut fonts, &mut atlas, "09:41", 4.0, 4.0, 13.0, [255, 255, 255]);
        let first = frame.glyphs.len();
        assert!(first > 0);
        let version = atlas.version;
        let mut again = Frame::new(120, 32, [0, 0, 0]).unwrap();
        again.text(&mut fonts, &mut atlas, "09:41", 4.0, 4.0, 13.0, [255, 255, 255]);
        assert_eq!(again.glyphs.len(), first);
        assert_eq!(atlas.version, version, "second frame reused the cached glyphs");
        let pixmap = rasterize(&again, &atlas).unwrap();
        assert_eq!((pixmap.width(), pixmap.height()), (120, 32));
        assert!(
            pixmap.data().chunks_exact(4).any(|px| px[3] == 255 && px[0] > 0),
            "glyphs produced visible coverage"
        );
    }
}
