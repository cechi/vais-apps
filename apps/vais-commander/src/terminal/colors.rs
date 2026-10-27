//! Terminal colours: the 16 ANSI colours from the theme, the 256-colour
//! cube and the greys, and the cell colour resolution.

use alacritty_terminal::term::color::Colors;
use alacritty_terminal::vte::ansi::{Color, NamedColor, Rgb};
use gpui_kit::component::theme::Theme;
use gpui_kit::*;

/// The colours a terminal draws with when the program sets none of its own.
#[derive(Clone)]
pub struct Palette {
    pub ansi: [Hsla; 16],
    pub foreground: Hsla,
    pub background: Hsla,
    pub cursor: Hsla,
    pub selection: Hsla,
}

impl Palette {
    pub fn from_theme(theme: &Theme) -> Self {
        let dark = theme.mode.is_dark();
        let black = if dark {
            hsla(0., 0., 0.12, 1.)
        } else {
            hsla(0., 0., 0.2, 1.)
        };
        let bright_black = theme.muted_foreground;
        let white = if dark {
            hsla(0., 0., 0.8, 1.)
        } else {
            hsla(0., 0., 0.55, 1.)
        };
        let bright_white = if dark {
            hsla(0., 0., 0.98, 1.)
        } else {
            hsla(0., 0., 0.3, 1.)
        };
        Self {
            ansi: [
                black,
                theme.red,
                theme.green,
                theme.yellow,
                theme.blue,
                theme.magenta,
                theme.cyan,
                white,
                bright_black,
                theme.red_light,
                theme.green_light,
                theme.yellow_light,
                theme.blue_light,
                theme.magenta_light,
                theme.cyan_light,
                bright_white,
            ],
            foreground: theme.foreground,
            background: theme.background,
            cursor: theme.foreground,
            selection: theme.selection,
        }
    }

    /// A cell colour as the theme and the program's own overrides make it.
    pub fn resolve(&self, color: Color, overrides: &Colors) -> Hsla {
        match color {
            Color::Spec(rgb) => rgb_to_hsla(rgb),
            Color::Indexed(ix) => self.indexed(ix as usize, overrides),
            Color::Named(named) => self.named(named, overrides),
        }
    }

    fn named(&self, named: NamedColor, overrides: &Colors) -> Hsla {
        if let Some(rgb) = overrides[named] {
            return rgb_to_hsla(rgb);
        }
        match named {
            NamedColor::Foreground | NamedColor::BrightForeground => self.foreground,
            NamedColor::DimForeground => self.foreground.alpha(0.6),
            NamedColor::Background => self.background,
            NamedColor::Cursor => self.cursor,
            NamedColor::DimBlack
            | NamedColor::DimRed
            | NamedColor::DimGreen
            | NamedColor::DimYellow
            | NamedColor::DimBlue
            | NamedColor::DimMagenta
            | NamedColor::DimCyan
            | NamedColor::DimWhite => {
                let base = named as usize - NamedColor::DimBlack as usize;
                self.ansi[base].alpha(0.6)
            }
            other => self.ansi[(other as usize) & 0xf],
        }
    }

    /// Colour `ix` of the 256 as the theme defines it (no overrides).
    pub fn indexed_default(&self, ix: usize) -> Hsla {
        if ix < 16 {
            return self.ansi[ix];
        }
        if ix < 232 {
            let ix = ix - 16;
            let (r, g, b) = (ix / 36, (ix / 6) % 6, ix % 6);
            let level = |v: usize| if v == 0 { 0 } else { 55 + v * 40 };
            return rgb_to_hsla(Rgb {
                r: level(r) as u8,
                g: level(g) as u8,
                b: level(b) as u8,
            });
        }
        if ix < 256 {
            let v = (8 + (ix - 232) * 10) as u8;
            return rgb_to_hsla(Rgb { r: v, g: v, b: v });
        }
        self.foreground
    }

    fn indexed(&self, ix: usize, overrides: &Colors) -> Hsla {
        if let Some(rgb) = overrides[ix] {
            return rgb_to_hsla(rgb);
        }
        if ix < 16 {
            return self.ansi[ix];
        }
        if ix < 232 {
            let ix = ix - 16;
            let (r, g, b) = (ix / 36, (ix / 6) % 6, ix % 6);
            let level = |v: usize| if v == 0 { 0 } else { 55 + v * 40 };
            return rgb_to_hsla(Rgb {
                r: level(r) as u8,
                g: level(g) as u8,
                b: level(b) as u8,
            });
        }
        if ix < 256 {
            let v = (8 + (ix - 232) * 10) as u8;
            return rgb_to_hsla(Rgb { r: v, g: v, b: v });
        }
        self.foreground
    }
}

pub fn rgb_to_hsla(rgb: Rgb) -> Hsla {
    Hsla::from(Rgba {
        r: rgb.r as f32 / 255.,
        g: rgb.g as f32 / 255.,
        b: rgb.b as f32 / 255.,
        a: 1.,
    })
}

#[cfg(test)]
mod tests {
    use super::rgb_to_hsla;
    use alacritty_terminal::vte::ansi::Rgb;

    #[test]
    fn converts_rgb() {
        let white = rgb_to_hsla(Rgb {
            r: 255,
            g: 255,
            b: 255,
        });
        assert!((white.l - 1.0).abs() < 0.01);
        let red = rgb_to_hsla(Rgb { r: 255, g: 0, b: 0 });
        assert!((red.s - 1.0).abs() < 0.01 && red.h.abs() < 0.01);
    }
}
