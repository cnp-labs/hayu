use chrono::{NaiveTime, Timelike};
use cosmic_text::{Attrs, Buffer, Color, FontSystem, Metrics, Shaping, SwashCache};
use tiny_skia::{Color as SkColor, Paint, Pixmap, Rect, Transform};

use crate::config;
use crate::hyprland::Workspace;

#[derive(Debug, Clone)]
pub struct Theme {
    pub bg: [u8; 3],
    pub fg: [u8; 3],
    pub muted: [u8; 3],
    pub accent: [u8; 3],
}

impl Theme {
    pub fn from_config(mode: &str, accent: &str) -> Self {
        let (bg, fg, muted) = if mode == "light" {
            ([0xF5, 0xF5, 0xF5], [0x1E, 0x1E, 0x2E], [0x6C, 0x6F, 0x85])
        } else {
            ([0x1E, 0x1E, 0x2E], [0xCD, 0xD6, 0xF4], [0x6C, 0x70, 0x86])
        };
        Self {
            bg,
            fg,
            muted,
            accent: config::parse_color(accent),
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

pub fn draw(scene: &Scene<'_>, fonts: &mut FontSystem, cache: &mut SwashCache) -> Option<Pixmap> {
    if scene.width == 0 || scene.height == 0 {
        return None;
    }
    let mut pixmap = Pixmap::new(scene.width, scene.height)?;
    let bg = scene.theme.bg;
    pixmap.fill(SkColor::from_rgba8(bg[0], bg[1], bg[2], 255));

    if fonts.db().is_empty() {
        // No system fonts: keep the bar usable (solid background) instead of panicking.
        return Some(pixmap);
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
            let accent = scene.theme.accent;
            fill(
                &mut pixmap,
                x,
                4.0 * scale,
                pill,
                scene.height as f32 - 8.0 * scale,
                accent,
            );
        }
        let color = if active { scene.theme.bg } else { fg };
        draw_text(
            &mut pixmap,
            fonts,
            cache,
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
    draw_text(
        &mut pixmap,
        fonts,
        cache,
        scene.clock,
        clock_x,
        text_y,
        font_size,
        fg,
    );

    let title_x = clock_x + clock_w + 12.0 * scale;
    let title_max = (scene.width as f32 - 8.0 * scale) - title_x;
    if !scene.title.is_empty() && title_max > 16.0 * scale {
        let title = ellipsize(scene.title, title_max, |s| text_width(fonts, s, font_size));
        draw_text(
            &mut pixmap,
            fonts,
            cache,
            &title,
            title_x,
            text_y,
            font_size,
            scene.theme.muted,
        );
    }

    Some(pixmap)
}

fn fill(pixmap: &mut Pixmap, x: f32, y: f32, w: f32, h: f32, rgb: [u8; 3]) {
    let Some(rect) = Rect::from_xywh(x, y, w, h) else {
        return;
    };
    let mut paint = Paint::default();
    paint.set_color_rgba8(rgb[0], rgb[1], rgb[2], 255);
    paint.anti_alias = true;
    pixmap.fill_rect(rect, &paint, Transform::identity(), None);
}

fn text_width(fonts: &mut FontSystem, text: &str, size: f32) -> f32 {
    let mut buffer = Buffer::new(fonts, Metrics::new(size, size * 1.3));
    buffer.set_text(text, &Attrs::new(), Shaping::Advanced, None);
    buffer.shape_until_scroll(fonts, false);
    buffer
        .layout_runs()
        .map(|run| run.line_w)
        .fold(0.0, f32::max)
}

#[allow(clippy::too_many_arguments)]
fn draw_text(
    pixmap: &mut Pixmap,
    fonts: &mut FontSystem,
    cache: &mut SwashCache,
    text: &str,
    x: f32,
    y: f32,
    size: f32,
    rgb: [u8; 3],
) {
    if text.is_empty() {
        return;
    }
    let mut buffer = Buffer::new(fonts, Metrics::new(size, size * 1.3));
    buffer.set_text(text, &Attrs::new(), Shaping::Advanced, None);
    buffer.set_size(None, Some(size * 1.3));
    let width = pixmap.width() as i32;
    let height = pixmap.height() as i32;
    let data = pixmap.data_mut();
    buffer.draw(
        fonts,
        cache,
        Color::rgb(rgb[0], rgb[1], rgb[2]),
        |gx, gy, w, h, color| {
            let (r, g, b, a) = color.as_rgba_tuple();
            if a == 0 {
                return;
            }
            for dy in 0..h as i32 {
                for dx in 0..w as i32 {
                    let px = gx + dx + x as i32;
                    let py = gy + dy + y as i32;
                    if px < 0 || py < 0 || px >= width || py >= height {
                        continue;
                    }
                    let index = ((py * width + px) * 4) as usize;
                    let alpha = a as u32;
                    let inverse = 255 - alpha;
                    data[index] = ((r as u32 * alpha + data[index] as u32 * inverse) / 255) as u8;
                    data[index + 1] =
                        ((g as u32 * alpha + data[index + 1] as u32 * inverse) / 255) as u8;
                    data[index + 2] =
                        ((b as u32 * alpha + data[index + 2] as u32 * inverse) / 255) as u8;
                    data[index + 3] =
                        (alpha + data[index + 3] as u32 * inverse / 255).min(255) as u8;
                }
            }
        },
    );
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

pub fn copy_to_shm(pixmap: &Pixmap, canvas: &mut [u8]) {
    let src = pixmap.data();
    let len = src.len().min(canvas.len());
    for (s, d) in src[..len]
        .chunks_exact(4)
        .zip(canvas[..len].chunks_exact_mut(4))
    {
        d[0] = s[2];
        d[1] = s[1];
        d[2] = s[0];
        d[3] = s[3];
    }
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
}



