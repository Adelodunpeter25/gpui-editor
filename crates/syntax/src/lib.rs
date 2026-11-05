//! `syntax`: language registry + highlight spans using tree-sitter via `lumis`.
//!
//! Exposes `highlight()` and `LanguageRegistry` without any GPUI dependency.

pub mod languages;
pub mod theme;
mod types;

pub use languages::{
    Language, LanguageRegistry, ALL_LANGUAGES, ASTRO, BASH, C, CLOJURE, CMAKE, CPP, CSHARP, CSS,
    DART, DIFF, DOCKERFILE, ELIXIR, ERLANG, GLSL, GO, GRAPHQL, HASKELL, HTML, INI, JAVA,
    JAVASCRIPT, JSON, JSON5, KOTLIN, LUA, MAKEFILE, MARKDOWN, NIX, OCAML, PHP, PLAIN_TEXT,
    PROTOBUF, PYTHON, RUBY, RUST, SCALA, SCSS, SOLIDITY, SQL, SVELTE, SWIFT, TOML, TSX, TYPESCRIPT,
    VUE, XML, YAML, ZIG, ZSH,
};
pub use theme::{get_theme, ThemePreset};
pub use types::{Capture, HighlightSpan, HighlightedVersion};

/// Parse a lumis theme hex color ("#rrggbb") into RGB bytes.
fn parse_hex_color(hex: &str) -> Option<(u8, u8, u8)> {
    let hex = hex.strip_prefix('#')?;
    if hex.len() != 6 {
        return None;
    }
    let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
    let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
    let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
    Some((r, g, b))
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

/// Highlight input text with `lumis` syntax engine using the given theme
/// preset (defaults to `GitHubDark` if `None`).
/// Converts byte ranges to char offsets for editor compatibility.
pub fn highlight(
    text: &str,
    language: Option<&Language>,
    buffer_version: u64,
) -> HighlightedVersion {
    highlight_themed(text, language, buffer_version, None)
}

/// Forward byte -> char offset converter for span boundaries. Highlight
/// ranges arrive in ascending order, so one advancing walk serves every
/// query; a query below the cursor (rare) just restarts the walk.
struct ByteCharCursor<'a> {
    text: &'a str,
    byte: usize,
    chars: usize,
}

impl<'a> ByteCharCursor<'a> {
    fn new(text: &'a str) -> Self {
        Self { text, byte: 0, chars: 0 }
    }

    /// Number of chars whose start byte is strictly below `byte_idx`
    /// (clamped to the total char count for `byte_idx` past the end).
    fn convert(&mut self, byte_idx: usize) -> usize {
        if byte_idx < self.byte {
            self.byte = 0;
            self.chars = 0;
        }
        while self.byte < byte_idx {
            let Some(ch) = self.text[self.byte..].chars().next() else {
                break;
            };
            self.byte += ch.len_utf8();
            self.chars += 1;
        }
        self.chars
    }
}

/// Same as [`highlight`], but with an explicit theme preset driving span colors.
pub fn highlight_themed(
    text: &str,
    language: Option<&Language>,
    buffer_version: u64,
    theme: Option<ThemePreset>,
) -> HighlightedVersion {
    let mut spans = Vec::new();

    let lang = language.unwrap_or(&PLAIN_TEXT);

    // ASCII needs no conversion (byte offset == char offset). Non-ASCII
    // converts span boundaries with a forward cursor: highlight ranges
    // arrive in ascending order, so one advancing walk covers them all —
    // no byte-per-entry offset table (4 transient bytes per source byte,
    // rebuilt on every keystroke).
    let is_ascii = text.is_ascii();
    let mut cursor = ByteCharCursor::new(text);
    let mut byte_to_char = |byte_idx: usize| -> usize {
        if is_ascii {
            byte_idx
        } else {
            cursor.convert(byte_idx)
        }
    };

    let theme = theme.unwrap_or(ThemePreset::GitHubDark).to_theme();

    let _ = lumis::highlight::highlight_iter(
        text,
        lang.lumis_lang,
        theme,
        |_text, _language, range, scope, style| {
            let capture = scope_to_capture(scope);
            let color = style.fg.as_deref().and_then(parse_hex_color);
            let start = byte_to_char(range.start);
            let end = byte_to_char(range.end);
            if start < end {
                spans.push(HighlightSpan {
                    start,
                    end,
                    capture,
                    color,
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
