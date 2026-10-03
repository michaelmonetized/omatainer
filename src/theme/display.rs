//! Display preferences change paint and layout only, never the renderer graph.
use super::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Contrast {
    #[default]
    Theme,
    Dark,
    Light,
}
impl Theme {
    /// Apply display choices to a fresh theme before drawing it.
    /// `appearance` supplies contrast, scale and motion; audio state stays outside this value.
    pub(crate) fn configure_display(&mut self, appearance: &crate::preferences::Appearance) {
        self.scale = appearance.scale;
        self.contrast = appearance.contrast;
        self.reduced_motion = appearance.reduced_motion;
        self.waveform_contrast = appearance.waveform_contrast;
        self.level_contrast = appearance.level_contrast;
        match self.contrast {
            Contrast::Theme => {}
            Contrast::Dark => {
                self.bg = rgb(8, 8, 8);
                self.bg_dark = rgb(16, 16, 16);
                self.bg_darker = rgb(0, 0, 0);
                self.bg_light = rgb(40, 40, 40);
                self.selection = rgb(55, 55, 55);
                self.fg = Color32::WHITE;
                self.fg_bright = Color32::WHITE;
                self.fg_dim = rgb(220, 220, 220);
                self.muted = rgb(175, 175, 175);
                self.accent = rgb(130, 215, 255);
                self.red = rgb(255, 175, 190);
                self.green = rgb(150, 245, 180);
                self.yellow = rgb(255, 230, 135);
                self.blue = self.accent;
                self.magenta = rgb(230, 190, 255);
                self.cyan = rgb(140, 240, 235);
                self.orange = rgb(255, 205, 145);
            }
            Contrast::Light => {
                self.bg = rgb(250, 250, 250);
                self.bg_dark = rgb(238, 238, 238);
                self.bg_darker = Color32::WHITE;
                self.bg_light = rgb(225, 225, 225);
                self.selection = rgb(210, 210, 210);
                self.fg = rgb(10, 10, 10);
                self.fg_bright = Color32::BLACK;
                self.fg_dim = rgb(40, 40, 40);
                self.muted = rgb(85, 85, 85);
                self.accent = rgb(0, 55, 150);
                self.red = rgb(145, 0, 30);
                self.green = rgb(0, 90, 35);
                self.yellow = rgb(105, 70, 0);
                self.blue = self.accent;
                self.magenta = rgb(90, 0, 125);
                self.cyan = rgb(0, 75, 85);
                self.orange = rgb(130, 50, 0);
            }
        }
    }
    /// Size readable custom text in the same units as egui's standard text.
    /// `preferred` is the original size; returns a font scaled by text choice with an unzoomed 11-pixel floor.
    pub(crate) fn text_size(&self, preferred: f32) -> f32 {
        (preferred * self.font_size / 12.0).max(11.0 / self.scale)
    }
    /// Leave room for a readable window title and a scrollable body.
    /// `ctx` supplies the logical viewport; returns the available body height at this font size.
    pub(crate) fn window_height(&self, ctx: &egui::Context) -> f32 {
        (ctx.screen_rect().height() - self.text_size(18.0) * 1.3 - 32.0).max(self.target_size(32.0))
    }
    /// Keep a control reachable while the user changes UI or text scale.
    /// `preferred` is its original size; returns at least 24 unzoomed pixels and room for text.
    pub(crate) fn target_size(&self, preferred: f32) -> f32 {
        preferred
            .max(24.0 / self.scale)
            .max(self.text_size(11.0) + 8.0)
    }
    /// Keep high-contrast selection backgrounds readable without depending on hue.
    /// `color` and `strength` retain theme tinting in normal mode; contrast presets return an opaque panel fill.
    pub(crate) fn tint(&self, color: Color32, strength: f32) -> Color32 {
        if self.contrast == Contrast::Theme {
            color.gamma_multiply(strength)
        } else {
            self.bg_light
        }
    }
    /// Choose a visible marker against its actual background.
    /// `color` is the desired hue; returns it when its contrast is sufficient, otherwise the theme foreground.
    pub(crate) fn marker(&self, color: Color32, background: Color32) -> Color32 {
        if self.contrast == Contrast::Theme || contrast_ratio(color, background) >= 3.0 {
            color
        } else {
            self.fg
        }
    }
    /// Apply waveform paint strength while retaining the original default theme tint.
    /// `color` and `strength` describe the original trace; returns its configured visual color.
    pub(crate) fn waveform(&self, color: Color32, strength: f32) -> Color32 {
        let base = if self.contrast == Contrast::Theme {
            color.gamma_multiply(strength)
        } else {
            color
        };
        self.trace(base, self.waveform_contrast)
    }
    /// Raise visual trace contrast without changing the measured amplitude.
    /// `color` is a visible trace and `amount` is 1–3; returns a blend toward the foreground.
    pub(crate) fn trace(&self, color: Color32, amount: f32) -> Color32 {
        let fraction = ((amount - 1.0) / 2.0).clamp(0.0, 1.0);
        if fraction == 0.0 {
            return color;
        }
        let blend = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * fraction).round() as u8;
        Color32::from_rgba_premultiplied(
            blend(color.r(), self.fg.r()),
            blend(color.g(), self.fg.g()),
            blend(color.b(), self.fg.b()),
            blend(color.a(), 255),
        )
    }
}
/// Compare two opaque sRGB paint colors using relative luminance.
/// `first` and `second` supply channels; returns the unrounded light/dark contrast ratio.
pub(crate) fn contrast_ratio(first: Color32, second: Color32) -> f64 {
    let luminance = |color: Color32| {
        let linear = |value: u8| {
            let value = f64::from(value) / 255.0;
            if value <= 0.04045 {
                value / 12.92
            } else {
                ((value + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * linear(color.r()) + 0.7152 * linear(color.g()) + 0.0722 * linear(color.b())
    };
    let first = luminance(first);
    let second = luminance(second);
    (first.max(second) + 0.05) / (first.min(second) + 0.05)
}
