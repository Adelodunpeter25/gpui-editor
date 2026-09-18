//! `syntax`: language registry + highlight spans using tree-sitter via `lumis`.
//!
//! Exposes `highlight()` and `LanguageRegistry` without any GPUI dependency.

pub mod languages;
pub mod theme;

pub use languages::{
    Language, LanguageRegistry, ALL_LANGUAGES, ASTRO, BASH, C, CLOJURE, CMAKE, CPP, CSHARP, CSS,
    DART, DIFF, DOCKERFILE, ELIXIR, ERLANG, GLSL, GO, GRAPHQL, HASKELL, HTML, INI, JAVA,
    JAVASCRIPT, JSON, JSON5, KOTLIN, LUA, MAKEFILE, MARKDOWN, NIX, OCAML, PHP, PLAIN_TEXT,
    PROTOBUF, PYTHON, RUBY, RUST, SCALA, SCSS, SOLIDITY, SQL, SVELTE, SWIFT, TOML, TSX,
    TYPESCRIPT, VUE, XML, YAML, ZIG, ZSH,
};
pub use theme::{get_theme, ThemePreset};

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

/// Map a tree-sitter scope name to our `Capture` enum.
fn scope_to_capture(scope: &str) -> Capture {
    let lower = scope.to_ascii_lowercase();
    if lower.contains("keyword")
        || lower.contains("conditional")
        || lower.contains("repeat")
        || lower.contains("statement")
        || lower.contains("operator")
        || lower.contains("exception")
    {
        Capture::Keyword
    } else if lower.contains("string") || lower.contains("character") {
        Capture::String
    } else if lower.contains("comment") || lower.contains("doc") {
        Capture::Comment
    } else if lower.contains("number")
        || lower.contains("float")
        || lower.contains("integer")
        || lower.contains("boolean")
        || lower.contains("constant")
    {
        Capture::Number
    } else if lower.contains("function")
        || lower.contains("method")
        || lower.contains("constructor")
        || lower.contains("call")
    {
        Capture::Function
    } else if lower.contains("type")
        || lower.contains("struct")
        || lower.contains("enum")
        || lower.contains("interface")
        || lower.contains("class")
        || lower.contains("trait")
    {
        Capture::Type
    } else {
        Capture::Plain
    }
}

/// Highlight input text with `lumis` syntax engine.
/// Converts byte ranges to char offsets for editor compatibility.
pub fn highlight(text: &str, language: Option<&Language>, buffer_version: u64) -> HighlightedVersion {
    let mut spans = Vec::new();

    let lang = language.unwrap_or(&PLAIN_TEXT);

    // Fast ASCII check: if all ASCII, byte offset == char offset (zero extra allocations)
    let is_ascii = text.is_ascii();
    let char_offset_table = if !is_ascii {
        let mut table = Vec::with_capacity(text.len() + 1);
        let mut char_count = 0usize;
        for (byte_idx, _) in text.char_indices() {
            while table.len() < byte_idx {
                table.push(char_count.saturating_sub(1));
            }
            table.push(char_count);
            char_count += 1;
        }
        while table.len() <= text.len() {
            table.push(char_count);
        }
        Some(table)
    } else {
        None
    };

    let byte_to_char = |byte_idx: usize| -> usize {
        if let Some(ref table) = char_offset_table {
            if byte_idx < table.len() {
                table[byte_idx]
            } else {
                *table.last().unwrap_or(&0)
            }
        } else {
            byte_idx
        }
    };

    let default_theme = lumis::themes::get("dracula").ok();

    let _ = lumis::highlight::highlight_iter(
        text,
        lang.lumis_lang,
        default_theme,
        |_text, _language, range, scope, _style| {
            let capture = scope_to_capture(scope);
            let start = byte_to_char(range.start);
            let end = byte_to_char(range.end);
            if start < end {
                spans.push(HighlightSpan {
                    start,
                    end,
                    capture,
                });
            }
            Ok::<_, std::io::Error>(())
        },
    );

    HighlightedVersion {
        buffer_version,
        spans,
    }
}
