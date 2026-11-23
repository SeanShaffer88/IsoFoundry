//! FluentGruv design tokens as RGB values, plus Windows high-contrast
//! overrides. Platform-independent so palettes and contrast can be tested.

/// 0xRRGGBB.
pub type Rgb = u32;

pub const BG_CANVAS: Rgb = 0x1D2021;
pub const BG_SURFACE: Rgb = 0x282828;
pub const BG_ELEVATED: Rgb = 0x32302F;
pub const BG_SUNKEN: Rgb = 0x171819;
pub const BORDER_SUBTLE: Rgb = 0x3C3836;
pub const BORDER_STRONG: Rgb = 0x504945;
pub const FG_PRIMARY: Rgb = 0xEBDBB2;
pub const FG_SECONDARY: Rgb = 0xD5C4A1;
pub const FG_MUTED: Rgb = 0xA89984;
pub const FG_INVERSE: Rgb = 0x1D2021;
pub const ACCENT_PRIMARY: Rgb = 0xFB4934;
pub const ACCENT_SECONDARY: Rgb = 0xCC241D;
pub const ACCENT_SUBTLE: Rgb = 0x3B1D1B;
pub const STATUS_SUCCESS: Rgb = 0xB8BB26;
pub const STATUS_WARNING: Rgb = 0xFABD2F;
pub const STATUS_DANGER: Rgb = 0xFB4934;

pub const LIGHT_BG_CANVAS: Rgb = 0xF9F5D7;
pub const LIGHT_BG_SURFACE: Rgb = 0xFBF1C7;
pub const LIGHT_BG_ELEVATED: Rgb = 0xEBDBB2;
pub const LIGHT_BG_SUNKEN: Rgb = 0xF2E5BC;
pub const LIGHT_BORDER_SUBTLE: Rgb = 0xD5C4A1;
pub const LIGHT_BORDER_STRONG: Rgb = 0xBDAE93;
pub const LIGHT_FG_PRIMARY: Rgb = 0x282828;
pub const LIGHT_FG_SECONDARY: Rgb = 0x3C3836;
pub const LIGHT_FG_MUTED: Rgb = 0x665C54;
pub const LIGHT_FG_INVERSE: Rgb = 0xFBF1C7;
pub const LIGHT_ACCENT_PRIMARY: Rgb = 0xCC241D;
pub const LIGHT_ACCENT_SECONDARY: Rgb = 0x9D0006;
pub const LIGHT_ACCENT_SUBTLE: Rgb = 0xF4D2AE;
pub const LIGHT_STATUS_SUCCESS: Rgb = 0x79740E;
pub const LIGHT_STATUS_WARNING: Rgb = 0xAF3A03;
pub const LIGHT_STATUS_DANGER: Rgb = 0xCC241D;

/// Win32 COLORREF (0x00BBGGRR) from 0xRRGGBB.
pub const fn colorref(rgb: Rgb) -> u32 {
    ((rgb & 0xFF) << 16) | (rgb & 0xFF00) | ((rgb >> 16) & 0xFF)
}

/// WCAG relative luminance.
#[cfg(test)]
pub fn luminance(rgb: Rgb) -> f64 {
    let ch = |v: u32| {
        let c = v as f64 / 255.0;
        if c <= 0.03928 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * ch((rgb >> 16) & 0xFF) + 0.7152 * ch((rgb >> 8) & 0xFF) + 0.0722 * ch(rgb & 0xFF)
}

#[cfg(test)]
pub fn contrast(a: Rgb, b: Rgb) -> f64 {
    let (x, y) = (luminance(a), luminance(b));
    let (hi, lo) = if x > y { (x, y) } else { (y, x) };
    (hi + 0.05) / (lo + 0.05)
}

/// Colors actually used for painting. In a Windows contrast theme every
/// token maps to a system color instead (docs/design-tokens.md).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Palette {
    pub bg: Rgb,
    pub surface: Rgb,
    pub elevated: Rgb,
    pub sunken: Rgb,
    pub border: Rgb,
    pub border_strong: Rgb,
    pub text: Rgb,
    pub text_secondary: Rgb,
    pub text_muted: Rgb,
    pub inverse: Rgb,
    pub btn: Rgb,
    pub btn_hover: Rgb,
    pub btn_pressed: Rgb,
    pub btn_text: Rgb,
    pub accent: Rgb,
    pub accent_secondary: Rgb,
    pub accent_subtle: Rgb,
    pub success: Rgb,
    pub warning: Rgb,
    pub danger: Rgb,
    pub input_bg: Rgb,
    pub high_contrast: bool,
    pub dark_theme: bool,
}

impl Palette {
    pub const DARK: Palette = Palette {
        bg: BG_CANVAS,
        surface: BG_SURFACE,
        elevated: BG_ELEVATED,
        sunken: BG_SUNKEN,
        border: BORDER_SUBTLE,
        border_strong: BORDER_STRONG,
        text: FG_PRIMARY,
        text_secondary: FG_SECONDARY,
        text_muted: FG_MUTED,
        inverse: FG_INVERSE,
        btn: BG_SURFACE,
        btn_hover: BG_ELEVATED,
        btn_pressed: ACCENT_SUBTLE,
        btn_text: FG_PRIMARY,
        accent: ACCENT_PRIMARY,
        accent_secondary: ACCENT_SECONDARY,
        accent_subtle: ACCENT_SUBTLE,
        success: STATUS_SUCCESS,
        warning: STATUS_WARNING,
        danger: STATUS_DANGER,
        input_bg: BG_SUNKEN,
        high_contrast: false,
        dark_theme: true,
    };

    pub const LIGHT: Palette = Palette {
        bg: LIGHT_BG_CANVAS,
        surface: LIGHT_BG_SURFACE,
        elevated: LIGHT_BG_ELEVATED,
        sunken: LIGHT_BG_SUNKEN,
        border: LIGHT_BORDER_SUBTLE,
        border_strong: LIGHT_BORDER_STRONG,
        text: LIGHT_FG_PRIMARY,
        text_secondary: LIGHT_FG_SECONDARY,
        text_muted: LIGHT_FG_MUTED,
        inverse: LIGHT_FG_INVERSE,
        btn: LIGHT_BG_SURFACE,
        btn_hover: LIGHT_BG_ELEVATED,
        btn_pressed: LIGHT_ACCENT_SUBTLE,
        btn_text: LIGHT_FG_PRIMARY,
        accent: LIGHT_ACCENT_PRIMARY,
        accent_secondary: LIGHT_ACCENT_SECONDARY,
        accent_subtle: LIGHT_ACCENT_SUBTLE,
        success: LIGHT_STATUS_SUCCESS,
        warning: LIGHT_STATUS_WARNING,
        danger: LIGHT_STATUS_DANGER,
        input_bg: LIGHT_BG_SUNKEN,
        high_contrast: false,
        dark_theme: false,
    };

    /// Builds the palette from system colors (`window`, `window_text`,
    /// `button_face`, `button_text`, `highlight`) for contrast themes.
    pub fn high_contrast(window: Rgb, window_text: Rgb, button_face: Rgb, button_text: Rgb, highlight: Rgb) -> Palette {
        Palette {
            bg: window,
            surface: window,
            elevated: button_face,
            sunken: window,
            border: window_text,
            border_strong: window_text,
            text: window_text,
            text_secondary: window_text,
            text_muted: window_text,
            inverse: button_text,
            btn: button_face,
            btn_hover: highlight,
            btn_pressed: highlight,
            btn_text: button_text,
            accent: highlight,
            accent_secondary: highlight,
            accent_subtle: window,
            success: window_text,
            warning: window_text,
            danger: window_text,
            input_bg: window,
            high_contrast: true,
            dark_theme: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colorref_swaps_red_and_blue() {
        assert_eq!(colorref(0x112233), 0x332211);
        assert_eq!(colorref(ACCENT_PRIMARY), 0x3449FB);
    }

    #[test]
    fn gruvbox_dark_and_light_tokens_meet_fluentgruv_contrast_floors() {
        let r = |a, b| (contrast(a, b) * 100.0).round() / 100.0;
        assert_eq!(r(FG_PRIMARY, BG_CANVAS), 11.95);
        assert_eq!(r(FG_PRIMARY, BG_SURFACE), 10.75);
        assert_eq!(r(FG_MUTED, BG_SURFACE), 5.30);
        assert_eq!(r(FG_INVERSE, ACCENT_PRIMARY), 4.77);
        assert_eq!(r(LIGHT_FG_PRIMARY, LIGHT_BG_CANVAS), 13.39);
        assert_eq!(r(LIGHT_FG_MUTED, LIGHT_BG_SURFACE), 5.74);
        assert_eq!(r(LIGHT_FG_INVERSE, LIGHT_ACCENT_PRIMARY), 4.82);
        assert!(contrast(ACCENT_PRIMARY, BG_CANVAS) >= 3.0);
        assert!(contrast(LIGHT_ACCENT_PRIMARY, LIGHT_BG_CANVAS) >= 3.0);
    }

    #[test]
    fn high_contrast_palette_uses_system_colors_for_every_role() {
        let p = Palette::high_contrast(1, 2, 3, 4, 5);
        assert!(p.high_contrast);
        assert_eq!(p.text, 2);
        assert_eq!(p.btn, 3);
        assert_eq!(p.btn_text, 4);
        assert_eq!(p.accent, 5);
    }
}
