//! `syntax`: language registry + highlight spans.
//!
//! M0: naive keyword scanner (no tree-sitter) so `cargo run` stays fast.
//! M1 swaps the backend to tree-sitter incremental parsing behind the same
//! `highlight()` return type. No GPUI dependency.

use std::collections::HashMap;
use std::sync::Arc;

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

/// Char-offset span with a capture kind. Packed-friendly (u32 triple).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HighlightSpan {
    pub start: usize,
    pub end: usize,
    pub capture: Capture,
}

#[derive(Debug, Clone)]
pub struct HighlightedVersion {
    pub buffer_version: u64,
    pub spans: Vec<HighlightSpan>,
}

#[derive(Debug, Clone)]
pub struct Language {
    pub name: &'static str,
    pub extensions: &'static [&'static str],
    pub keywords: &'static [&'static str],
}

impl Language {
    const fn new(
        name: &'static str,
        extensions: &'static [&'static str],
        keywords: &'static [&'static str],
    ) -> Self {
        Self {
            name,
            extensions,
            keywords,
        }
    }
}

pub const RUST: Language = Language::new(
    "rust",
    &["rs"],
    &[
        "fn", "let", "mut", "struct", "enum", "impl", "for", "in", "if", "else", "match",
        "return", "use", "mod", "pub", "crate", "where", "loop", "while", "const", "static",
        "trait", "type", "dyn", "async", "await", "move", "ref", "self", "Self", "super",
    ],
);

pub const MARKDOWN: Language = Language::new("markdown", &["md", "markdown"], &[]);

/// Registry: extension -> language. Single owner, cheap Arc clones.
#[derive(Debug, Clone, Default)]
pub struct LanguageRegistry {
    map: HashMap<&'static str, &'static Language>,
}

impl LanguageRegistry {
    pub fn builtin() -> Self {
        let mut map = HashMap::new();
        for lang in [&RUST, &MARKDOWN] {
            for ext in lang.extensions {
                map.insert(*ext, lang);
            }
        }
        Self { map }
    }

    pub fn for_extension(&self, ext: &str) -> Option<&'static Language> {
        self.map.get(ext).copied()
    }

    pub fn for_name(name: &str) -> Option<&'static Language> {
        match name {
            "rust" => Some(&RUST),
            "markdown" => Some(&MARKDOWN),
            _ => None,
        }
    }
}

/// Naive highlighter: comments, strings, numbers, keywords.
/// Operates on `&str` snapshot; caller passes `rope.to_string()` once.
/// M1 replaces internals with tree-sitter; signature stays.
pub fn highlight(text: &str, language: Option<&Language>, buffer_version: u64) -> HighlightedVersion {
    let mut spans = Vec::new();
    if let Some(lang) = language {
        if lang.name == "markdown" {
            highlight_markdown(text, &mut spans);
            return HighlightedVersion {
                buffer_version,
                spans,
            };
        }
    }
    highlight_code(text, language, &mut spans);
    HighlightedVersion {
        buffer_version,
        spans,
    }
}

fn highlight_code(text: &str, language: Option<&Language>, out: &mut Vec<HighlightSpan>) {
    let bytes = text.as_bytes();
    let mut ix = 0usize; // byte index
    let mut char_ix = 0usize; // char index
    let keywords: &[&str] = language.map(|l| l.keywords).unwrap_or(&[]);

    while ix < bytes.len() {
        let c = bytes[ix] as char;
        // Line comment
        if c == '/' && ix + 1 < bytes.len() && bytes[ix + 1] == b'/' {
            let start_c = char_ix;
            while ix < bytes.len() && bytes[ix] != b'\n' {
                ix += 1;
                char_ix += 1;
            }
            out.push(HighlightSpan {
                start: start_c,
                end: char_ix,
                capture: Capture::Comment,
            });
            continue;
        }
        // String
        if c == '"' {
            let start_c = char_ix;
            ix += 1;
            char_ix += 1;
            while ix < bytes.len() && bytes[ix] != b'"' && bytes[ix] != b'\n' {
                if bytes[ix] == b'\\' {
                    ix += 1;
                    char_ix += 1;
                }
                ix += 1;
                char_ix += 1;
            }
            if ix < bytes.len() && bytes[ix] == b'"' {
                ix += 1;
                char_ix += 1;
            }
            out.push(HighlightSpan {
                start: start_c,
                end: char_ix,
                capture: Capture::String,
            });
            continue;
        }
        // Number
        if c.is_ascii_digit() {
            let start_c = char_ix;
            while ix < bytes.len() && (bytes[ix] as char).is_ascii_alphanumeric() {
                ix += 1;
                char_ix += 1;
            }
            out.push(HighlightSpan {
                start: start_c,
                end: char_ix,
                capture: Capture::Number,
            });
            continue;
        }
        // Word (keyword check)
        if c.is_ascii_alphabetic() || c == '_' {
            let start_c = char_ix;
            let start_b = ix;
            while ix < bytes.len() && ((bytes[ix] as char).is_ascii_alphanumeric() || bytes[ix] == b'_') {
                ix += 1;
                char_ix += 1;
            }
            let word = &text[start_b..ix];
            if keywords.contains(&word) {
                out.push(HighlightSpan {
                    start: start_c,
                    end: char_ix,
                    capture: Capture::Keyword,
                });
            }
            continue;
        }
        ix += 1;
        char_ix += 1;
    }
}

fn highlight_markdown(text: &str, out: &mut Vec<HighlightSpan>) {
    // Headings: lines starting with # -> Keyword; code spans -> String.
    let mut char_ix = 0usize;
    for line in text.split_inclusive('\n') {
        let trimmed = line.trim_start();
        if trimmed.starts_with('#') {
            out.push(HighlightSpan {
                start: char_ix,
                end: char_ix + line.chars().count(),
                capture: Capture::Keyword,
            });
        }
        char_ix += line.chars().count();
    }
    let _ = Arc::new(());
}
