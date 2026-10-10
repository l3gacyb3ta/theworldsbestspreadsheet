//! Fallback fonts from the OS for scripts egui's built-in fonts don't cover (Japanese, Chinese, Korean, Arabic,
//! Hebrew, Devanagari…), so text typed with an IME doesn't show as boxes. Nothing is bundled: the first font found
//! in each group is read at startup and added after egui's own fonts. Arabic and Hebrew get glyphs but not shaping
//! or right-to-left layout (egui has neither).

use eframe::egui::{self, epaint::text::{FontInsert, FontPriority, InsertFontFamily}, FontData, FontFamily};

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

/// Adds the fallbacks found to both font families. Missing fonts are skipped.
pub fn install(ctx: &egui::Context) {
    for (i, group) in GROUPS.iter().enumerate() {
        let Some((bytes, index)) = group.iter().find_map(|(p, ix)| std::fs::read(p).ok().map(|b| (b, *ix))) else { continue };
        let data = FontData { index, ..FontData::from_owned(bytes) };
        let families = [FontFamily::Proportional, FontFamily::Monospace]
            .into_iter()
            .map(|family| InsertFontFamily { family, priority: FontPriority::Lowest })
            .collect();
        ctx.add_font(FontInsert { name: format!("system fallback {i}"), data, families });
    }
}
