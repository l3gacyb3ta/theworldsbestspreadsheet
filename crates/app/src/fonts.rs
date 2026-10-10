//! Fallback fonts from the OS for scripts egui's built-in fonts don't cover (Japanese, Chinese, Korean, Arabic,
//! Hebrew, Devanagari…), so text typed with an IME doesn't show as boxes. Nothing is bundled, and nothing is read
//! until text that needs them shows up (they're tens of MB): then the first font found in each group is added after
//! egui's own fonts. Arabic and Hebrew get glyphs but not right-to-left layout (egui has no bidi, see #51).

use eframe::egui::{self, epaint::text::{FontInsert, FontPriority, InsertFontFamily}, FontData, FontFamily};
use std::cell::Cell;

/// Candidates in order of preference; one font is taken from each group.
#[cfg(target_os = "macos")]
const GROUPS: &[&[(&str, u32)]] = &[
    // Japanese first, as the kana and kanji forms most macOS users expect
    &[("/System/Library/Fonts/ヒラギノ角ゴシック W3.ttc", 0), ("/System/Library/Fonts/Hiragino Sans GB.ttc", 0)],
    // everything else: Hangul, Arabic, Hebrew, Devanagari, Thai…
    &[("/System/Library/Fonts/Supplemental/Arial Unicode.ttf", 0), ("/Library/Fonts/Arial Unicode.ttf", 0)],
];

#[cfg(target_os = "windows")]
const GROUPS: &[&[(&str, u32)]] = &[
    &[("C:\\Windows\\Fonts\\YuGothR.ttc", 0), ("C:\\Windows\\Fonts\\meiryo.ttc", 0), ("C:\\Windows\\Fonts\\msgothic.ttc", 0)],
    &[("C:\\Windows\\Fonts\\malgun.ttf", 0)],
    &[("C:\\Windows\\Fonts\\msyh.ttc", 0), ("C:\\Windows\\Fonts\\simsun.ttc", 0)],
    &[("C:\\Windows\\Fonts\\segoeui.ttf", 0)],
    &[("C:\\Windows\\Fonts\\Nirmala.ttf", 0)],
];

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
const GROUPS: &[&[(&str, u32)]] = &[
    &[
        ("/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc", 0),
        ("/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc", 0),
        ("/usr/share/fonts/google-noto-cjk/NotoSansCJK-Regular.ttc", 0),
        ("/usr/share/fonts/truetype/droid/DroidSansFallbackFull.ttf", 0),
        ("/usr/share/fonts/wenquanyi/wqy-microhei/wqy-microhei.ttc", 0),
    ],
    &[
        ("/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf", 0),
        ("/usr/share/fonts/dejavu/DejaVuSans.ttf", 0),
        ("/usr/share/fonts/TTF/DejaVuSans.ttf", 0),
    ],
];

/// The fonts found on this system: the first existing file of each group. Missing fonts are skipped.
pub fn system() -> Vec<FontData> {
    GROUPS
        .iter()
        .filter_map(|group| group.iter().find_map(|(p, ix)| std::fs::read(p).ok().map(|b| FontData { index: *ix, ..FontData::from_owned(b) })))
        .collect()
}

/// Characters egui's built-in fonts have no glyphs for, that a system font is likely to: CJK, kana, Hangul, Arabic,
/// Hebrew, Indic, Thai, and the arrows in key names (⇧, ↑).
pub fn needs_fallback(c: char) -> bool {
    matches!(c as u32,
        0x2190..=0x21FF        // arrows: ⇧ ↑ ↓ ← →
        | 0x0590..=0x08FF      // Hebrew, Arabic, Syriac, Thaana…
        | 0x0900..=0x0E7F      // Indic scripts, Sinhala, Thai
        | 0x1100..=0x11FF      // Hangul Jamo
        | 0x2E80..=0x9FFF      // CJK radicals, punctuation, kana, Hangul compatibility, CJK ideographs
        | 0xA960..=0xA97F | 0xAC00..=0xD7FF  // Hangul
        | 0xF900..=0xFAFF      // CJK compatibility ideographs
        | 0xFB1D..=0xFDFF | 0xFE70..=0xFEFF  // Hebrew and Arabic presentation forms
        | 0xFF00..=0xFFEF      // full- and half-width forms
        | 0x20000..=0x3FFFF)   // CJK extensions
}

/// Loads the fallbacks the first time text that needs them is seen (`note`), at the end of that frame (`load_if_wanted`).
pub struct Fallbacks {
    source: Option<Box<dyn FnOnce() -> Vec<FontData>>>,
    wanted: Cell<bool>,
}

impl Default for Fallbacks {
    fn default() -> Self {
        Fallbacks::from_source(system)
    }
}

impl Fallbacks {
    pub fn from_source(source: impl FnOnce() -> Vec<FontData> + 'static) -> Self {
        Fallbacks { source: Some(Box::new(source)), wanted: Cell::new(false) }
    }

    pub fn loaded(&self) -> bool {
        self.source.is_none()
    }

    /// Text about to be shown or edited.
    pub fn note(&self, s: &str) {
        if !s.is_ascii() && self.source.is_some() && !self.wanted.get() && s.chars().any(needs_fallback) {
            self.wanted.set(true);
        }
    }

    /// Adds the fallbacks to both font families if something needed them. egui uses them from the next pass, so this
    /// frame's boxes are redrawn right away.
    pub fn load_if_wanted(&mut self, ctx: &egui::Context) {
        if !self.wanted.get() {
            return;
        }
        let Some(source) = self.source.take() else { return };
        for (i, data) in source().into_iter().enumerate() {
            let families = [FontFamily::Proportional, FontFamily::Monospace]
                .into_iter()
                .map(|family| InsertFontFamily { family, priority: FontPriority::Lowest })
                .collect();
            ctx.add_font(FontInsert { name: format!("system fallback {i}"), data, families });
        }
        ctx.request_repaint();
    }
}
