//! Design tokens from docs/design-tokens.md as RGB values, plus the
//! high-contrast switch. Platform-independent so it can be unit tested.

/// 0xRRGGBB.
pub type Rgb = u32;

pub const BG: Rgb = 0x0E0E10;
pub const SURFACE: Rgb = 0x17171A;
pub const SURFACE_2: Rgb = 0x1F1F23;
pub const BORDER: Rgb = 0x2E2E34;
/// Input outlines (text boxes, check boxes); used from M1 task dialogs.
#[allow(dead_code)]
pub const BORDER_CONTROL: Rgb = 0x7A7A84;
pub const TEXT: Rgb = 0xFFFFFF;
pub const TEXT_MUTED: Rgb = 0xB8B8C0;
pub const BTN: Rgb = 0x6B0700;
pub const BTN_HOVER: Rgb = 0x861008;
pub const BTN_PRESSED: Rgb = 0x540500;
pub const ACCENT: Rgb = 0xE0313A;

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
    pub border: Rgb,
    pub text: Rgb,
    pub text_muted: Rgb,
    pub btn: Rgb,
    pub btn_hover: Rgb,
    pub btn_pressed: Rgb,
    pub btn_text: Rgb,
    pub accent: Rgb,
    pub input_bg: Rgb,
}

impl Palette {
    pub const DARK: Palette = Palette {
        bg: BG,
        surface: SURFACE,
        border: BORDER,
        text: TEXT,
        text_muted: TEXT_MUTED,
        btn: BTN,
        btn_hover: BTN_HOVER,
        btn_pressed: BTN_PRESSED,
        btn_text: TEXT,
        accent: ACCENT,
        input_bg: SURFACE_2,
    };

    /// Builds the palette from system colors (`window`, `window_text`,
    /// `button_face`, `button_text`, `highlight`) for contrast themes.
    pub fn high_contrast(window: Rgb, window_text: Rgb, button_face: Rgb, button_text: Rgb, highlight: Rgb) -> Palette {
        Palette {
            bg: window,
            surface: window,
            border: window_text,
            text: window_text,
            text_muted: window_text,
            btn: button_face,
            btn_hover: highlight,
            btn_pressed: highlight,
            btn_text: button_text,
            accent: highlight,
            input_bg: window,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colorref_swaps_red_and_blue() {
        assert_eq!(colorref(0x112233), 0x332211);
        assert_eq!(colorref(BTN), 0x00076B);
    }

    #[test]
    fn token_contrast_matches_docs() {
        // Values published in docs/design-tokens.md; keep them in sync.
        let r = |a, b| (contrast(a, b) * 100.0).round() / 100.0;
        assert_eq!(r(TEXT, BTN), 12.68);
        assert_eq!(r(TEXT, BTN_HOVER), 10.04);
        assert_eq!(r(TEXT, SURFACE), 17.89);
        assert_eq!(r(TEXT_MUTED, SURFACE), 9.08);
        assert_eq!(r(ACCENT, SURFACE), 3.98);
        assert_eq!(r(BORDER_CONTROL, SURFACE_2), 3.87);
    }

    #[test]
    fn text_meets_wcag_aa_on_every_button_state() {
        for b in [BTN, BTN_HOVER, BTN_PRESSED] {
            assert!(contrast(TEXT, b) >= 4.5);
        }
        // Focus ring is a UI component: 3:1 against the surface it sits on.
        assert!(contrast(ACCENT, SURFACE) >= 3.0);
    }
}
