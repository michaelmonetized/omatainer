use egui::{Color32, CornerRadius, Stroke, Style, Visuals};
#[cfg(test)]
use std::fs;
use std::path::{Path, PathBuf};
mod color;
pub(crate) mod reload;
#[cfg(test)]
mod tests;

#[derive(Clone, Debug, PartialEq)]
pub struct Theme {
    pub bg: Color32,
    pub bg_dark: Color32,
    pub bg_darker: Color32,
    pub bg_light: Color32,
    pub fg: Color32,
    pub fg_dim: Color32,
    pub fg_bright: Color32,
    pub accent: Color32,
    pub red: Color32,
    pub green: Color32,
    pub yellow: Color32,
    pub blue: Color32,
    pub magenta: Color32,
    pub cyan: Color32,
    pub orange: Color32,
    pub selection: Color32,
    pub muted: Color32,
    pub font: String,
    pub font_size: f32,
    pub rounding: f32,
    pub path: PathBuf,
}

impl Default for Theme {
    fn default() -> Self {
        Self {
            bg: rgb(0x1e, 0x1e, 0x2e),
            bg_dark: rgb(0x16, 0x16, 0x22),
            bg_darker: rgb(0x10, 0x10, 0x19),
            bg_light: rgb(0x31, 0x32, 0x44),
            fg: rgb(0xcd, 0xd6, 0xf4),
            fg_dim: rgb(0x6c, 0x70, 0x86),
            fg_bright: rgb(0xcd, 0xd6, 0xf4),
            accent: rgb(0x89, 0xb4, 0xfa),
            red: rgb(0xf3, 0x8b, 0xa8),
            green: rgb(0xa6, 0xe3, 0xa1),
            yellow: rgb(0xf9, 0xe2, 0xaf),
            blue: rgb(0x89, 0xb4, 0xfa),
            magenta: rgb(0xf5, 0xc2, 0xe7),
            cyan: rgb(0x94, 0xe2, 0xd5),
            orange: rgb(0xf6, 0xb6, 0xab),
            selection: rgb(0x45, 0x47, 0x5a),
            muted: rgb(0x58, 0x5b, 0x70),
            font: "bundled default".into(),
            font_size: 12.0,
            rounding: 6.0,
            path: theme_dir().join("colors.toml"),
        }
    }
}

impl Theme {
    #[cfg(test)]
    fn reload_colors(&mut self, path: &Path) -> Vec<ColorDiagnostic> {
        fs::read_to_string(path)
            .ok()
            .and_then(|raw| self.apply_colors(&raw).ok())
            .unwrap_or_default()
    }

    fn apply_colors(&mut self, raw: &str) -> Result<Vec<ColorDiagnostic>, toml::de::Error> {
        let v = raw.parse::<toml::Value>()?;
        let mut diagnostics = Vec::new();
        self.bg = hex_of(&v, "background", self.bg, &mut diagnostics);
        self.bg_dark = hex_of(&v, "dark_background", self.bg_dark, &mut diagnostics);
        self.bg_darker = hex_of(&v, "darker_background", self.bg_darker, &mut diagnostics);
        self.bg_light = hex_of(&v, "lighter_background", self.bg_light, &mut diagnostics);
        self.fg = hex_of(&v, "foreground", self.fg, &mut diagnostics);
        self.fg_dim = hex_of(&v, "dark_foreground", self.fg_dim, &mut diagnostics);
        self.fg_bright = hex_of(&v, "bright_foreground", self.fg_bright, &mut diagnostics);
        self.accent = hex_of(&v, "accent", self.accent, &mut diagnostics);
        self.red = hex_of(&v, "red", self.red, &mut diagnostics);
        self.green = hex_of(&v, "green", self.green, &mut diagnostics);
        self.yellow = hex_of(&v, "yellow", self.yellow, &mut diagnostics);
        self.blue = hex_of(&v, "blue", self.blue, &mut diagnostics);
        self.magenta = hex_of(&v, "magenta", self.magenta, &mut diagnostics);
        self.cyan = hex_of(&v, "cyan", self.cyan, &mut diagnostics);
        self.orange = hex_of(&v, "orange", self.orange, &mut diagnostics);
        self.selection = hex_of(&v, "selection", self.selection, &mut diagnostics);
        self.muted = hex_of(&v, "muted", self.muted, &mut diagnostics);
        Ok(diagnostics)
    }

    pub fn apply(&self, ctx: &egui::Context) {
        let mut style = Style {
            visuals: Visuals {
                dark_mode: true,
                override_text_color: Some(self.fg),
                panel_fill: self.bg,
                window_fill: self.bg,
                faint_bg_color: self.bg_dark,
                extreme_bg_color: self.bg_darker,
                code_bg_color: self.bg_dark,
                hyperlink_color: self.accent,
                selection: egui::style::Selection {
                    bg_fill: self.accent.gamma_multiply(0.35),
                    stroke: st(1.0, self.accent),
                },
                widgets: egui::style::Widgets {
                    noninteractive: widget(self.bg_dark, self.fg_dim, self.muted),
                    inactive: widget(self.bg_light, self.fg, self.muted),
                    hovered: widget(self.selection, self.fg_bright, self.accent),
                    active: widget(
                        self.accent.gamma_multiply(0.25),
                        self.fg_bright,
                        self.accent,
                    ),
                    open: widget(self.bg_light, self.fg, self.accent),
                },
                window_corner_radius: CornerRadius::same(0),
                window_stroke: st(0.0, self.bg),
                menu_corner_radius: CornerRadius::same(self.rounding as u8),
                ..Visuals::dark()
            },
            ..Style::default()
        };
        // Preserve the default style hierarchy while honoring the shell's
        // base size for every standard text style, including numeric widgets.
        let base = self.font_size;
        for (kind, font) in &mut style.text_styles {
            font.size = match kind {
                egui::TextStyle::Heading => base * 1.5,
                egui::TextStyle::Small => base * (10.0 / 12.0),
                _ => base,
            };
        }
        style.spacing.item_spacing = egui::vec2(6.0, 4.0);
        style.spacing.button_padding = egui::vec2(8.0, 4.0);
        style.spacing.window_margin = egui::Margin::same(8);
        style.visuals.widgets.inactive.corner_radius = CornerRadius::same(self.rounding as u8);
        style.visuals.widgets.hovered.corner_radius = CornerRadius::same(self.rounding as u8);
        style.visuals.widgets.active.corner_radius = CornerRadius::same(self.rounding as u8);
        ctx.set_style(style);
    }

    pub fn track_color(&self, i: usize) -> Color32 {
        const ORDER: [&str; 8] = [
            "accent", "red", "green", "yellow", "magenta", "cyan", "orange", "blue",
        ];
        match ORDER[i % 8] {
            "accent" => self.accent,
            "red" => self.red,
            "green" => self.green,
            "yellow" => self.yellow,
            "magenta" => self.magenta,
            "cyan" => self.cyan,
            "orange" => self.orange,
            _ => self.blue,
        }
    }
}

fn st(width: f32, color: Color32) -> Stroke {
    Stroke { width, color }
}

fn widget(bg: Color32, fg: Color32, stroke: Color32) -> egui::style::WidgetVisuals {
    egui::style::WidgetVisuals {
        bg_fill: bg,
        weak_bg_fill: bg,
        bg_stroke: st(1.0, stroke.gamma_multiply(0.4)),
        fg_stroke: st(1.0, fg),
        corner_radius: CornerRadius::same(6),
        expansion: 0.0,
    }
}

fn rgb(r: u8, g: u8, b: u8) -> Color32 {
    Color32::from_rgb(r, g, b)
}

#[derive(Debug, PartialEq, Eq)]
struct ColorDiagnostic {
    key: &'static str,
}

impl std::fmt::Display for ColorDiagnostic {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "invalid theme color '{}': expected a string of six ASCII hex digits, optionally prefixed with #; keeping the previous color", self.key)
    }
}

fn hex_of(
    v: &toml::Value,
    key: &'static str,
    fallback: Color32,
    diagnostics: &mut Vec<ColorDiagnostic>,
) -> Color32 {
    let Some(value) = v.get(key) else {
        return fallback;
    };
    if let Some(color) = value.as_str().and_then(parse_hex) {
        color
    } else {
        diagnostics.push(ColorDiagnostic { key });
        fallback
    }
}

fn parse_hex(s: &str) -> Option<Color32> {
    let [r, g, b] = color::parse_rgb(s)?;
    Some(Color32::from_rgb(r, g, b))
}

pub fn theme_dir() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
    Path::new(&home).join(".local/state/omarchy/current/theme")
}

pub fn config_dir() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
    Path::new(&home).join(".config/omatainer")
}

pub fn socket_path() -> std::io::Result<PathBuf> {
    crate::runtime::socket_path()
}
