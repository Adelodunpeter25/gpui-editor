//! Data types for `syntax`: highlight output shapes shared with `editor-ui`.
//! (`Language`/`LanguageRegistry` live in `languages.rs`, `ThemePreset` in
//! `theme.rs` — already split into their own files.)

/// Stable capture kinds. Mapped to theme colors once per pass in `editor-ui`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Capture {
    Keyword,
    String,
    Comment,
    Number,
    Function,
    Type,
    Plain,
}

/// Char-offset span with a capture kind and the theme's actual foreground
/// color for that scope (`None` if the theme leaves it unstyled).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HighlightSpan {
    pub start: usize,
    pub end: usize,
    pub capture: Capture,
    pub color: Option<(u8, u8, u8)>,
}

#[derive(Debug, Clone)]
pub struct HighlightedVersion {
    pub buffer_version: u64,
    pub spans: Vec<HighlightSpan>,
}
