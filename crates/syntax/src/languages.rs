//! Language definitions and mappings for syntax highlighting.

use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct Language {
    pub name: &'static str,
    pub lumis_lang: lumis::languages::Language,
    pub extensions: &'static [&'static str],
}

impl Language {
    pub const fn new(
        name: &'static str,
        lumis_lang: lumis::languages::Language,
        extensions: &'static [&'static str],
    ) -> Self {
        Self {
            name,
            lumis_lang,
            extensions,
        }
    }
}

// Built-in languages mapping to Lumis language definitions
pub const PLAIN_TEXT: Language = Language::new("plain_text", lumis::languages::Language::PlainText, &["txt", "text"]);
pub const RUST: Language = Language::new("rust", lumis::languages::Language::Rust, &["rs"]);
pub const TOML: Language = Language::new("toml", lumis::languages::Language::Toml, &["toml"]);
pub const JSON: Language = Language::new("json", lumis::languages::Language::JSON, &["json"]);
pub const JSON5: Language = Language::new("json5", lumis::languages::Language::JSON5, &["json5"]);
pub const MARKDOWN: Language = Language::new("markdown", lumis::languages::Language::Markdown, &["md", "markdown"]);
pub const JAVASCRIPT: Language = Language::new("javascript", lumis::languages::Language::JavaScript, &["js", "mjs", "cjs"]);
pub const TYPESCRIPT: Language = Language::new("typescript", lumis::languages::Language::TypeScript, &["ts", "mts", "cts"]);
pub const TSX: Language = Language::new("tsx", lumis::languages::Language::Tsx, &["tsx"]);
pub const HTML: Language = Language::new("html", lumis::languages::Language::HTML, &["html", "htm"]);
pub const CSS: Language = Language::new("css", lumis::languages::Language::CSS, &["css"]);
pub const SCSS: Language = Language::new("scss", lumis::languages::Language::SCSS, &["scss"]);
pub const PYTHON: Language = Language::new("python", lumis::languages::Language::Python, &["py", "pyw"]);
pub const GO: Language = Language::new("go", lumis::languages::Language::Go, &["go"]);
pub const C: Language = Language::new("c", lumis::languages::Language::C, &["c", "h"]);
pub const CPP: Language = Language::new("cpp", lumis::languages::Language::CPlusPlus, &["cpp", "cc", "cxx", "hpp", "hh", "hxx"]);
pub const CSHARP: Language = Language::new("csharp", lumis::languages::Language::CSharp, &["cs"]);
pub const JAVA: Language = Language::new("java", lumis::languages::Language::Java, &["java"]);
pub const KOTLIN: Language = Language::new("kotlin", lumis::languages::Language::Kotlin, &["kt", "kts"]);
pub const SWIFT: Language = Language::new("swift", lumis::languages::Language::Swift, &["swift"]);
pub const PHP: Language = Language::new("php", lumis::languages::Language::Php, &["php", "phtml"]);
pub const RUBY: Language = Language::new("ruby", lumis::languages::Language::Ruby, &["rb", "rake", "gemspec"]);
pub const BASH: Language = Language::new("bash", lumis::languages::Language::Bash, &["sh", "bash"]);
pub const ZSH: Language = Language::new("zsh", lumis::languages::Language::Zsh, &["zsh"]);
pub const YAML: Language = Language::new("yaml", lumis::languages::Language::YAML, &["yaml", "yml"]);
pub const SQL: Language = Language::new("sql", lumis::languages::Language::SQL, &["sql"]);
pub const ZIG: Language = Language::new("zig", lumis::languages::Language::Zig, &["zig"]);
pub const LUA: Language = Language::new("lua", lumis::languages::Language::Lua, &["lua"]);
pub const DOCKERFILE: Language = Language::new("dockerfile", lumis::languages::Language::Dockerfile, &["dockerfile", "Dockerfile"]);
pub const ELIXIR: Language = Language::new("elixir", lumis::languages::Language::Elixir, &["ex", "exs"]);
pub const ERLANG: Language = Language::new("erlang", lumis::languages::Language::Erlang, &["erl", "hrl"]);
pub const HASKELL: Language = Language::new("haskell", lumis::languages::Language::Haskell, &["hs", "lhs"]);
pub const OCAML: Language = Language::new("ocaml", lumis::languages::Language::OCaml, &["ml", "mli"]);
pub const SCALA: Language = Language::new("scala", lumis::languages::Language::Scala, &["scala", "sc"]);
pub const CLOJURE: Language = Language::new("clojure", lumis::languages::Language::Clojure, &["clj", "cljs", "cljc", "edn"]);
pub const DART: Language = Language::new("dart", lumis::languages::Language::Dart, &["dart"]);
pub const GLSL: Language = Language::new("glsl", lumis::languages::Language::Glsl, &["glsl", "vert", "frag", "geom", "comp"]);
pub const MAKEFILE: Language = Language::new("makefile", lumis::languages::Language::Make, &["mk", "Makefile", "makefile"]);
pub const CMAKE: Language = Language::new("cmake", lumis::languages::Language::CMake, &["cmake", "CMakeLists.txt"]);
pub const NIX: Language = Language::new("nix", lumis::languages::Language::Nix, &["nix"]);
pub const SOLIDITY: Language = Language::new("solidity", lumis::languages::Language::Solidity, &["sol"]);
pub const GRAPHQL: Language = Language::new("graphql", lumis::languages::Language::GraphQL, &["graphql", "gql"]);
pub const PROTOBUF: Language = Language::new("protobuf", lumis::languages::Language::ProtoBuf, &["proto"]);
pub const ASTRO: Language = Language::new("astro", lumis::languages::Language::Astro, &["astro"]);
pub const SVELTE: Language = Language::new("svelte", lumis::languages::Language::Svelte, &["svelte"]);
pub const VUE: Language = Language::new("vue", lumis::languages::Language::Vue, &["vue"]);
pub const XML: Language = Language::new("xml", lumis::languages::Language::XML, &["xml", "svg"]);
pub const INI: Language = Language::new("ini", lumis::languages::Language::INI, &["ini"]);
pub const DIFF: Language = Language::new("diff", lumis::languages::Language::Diff, &["diff", "patch"]);

pub const ALL_LANGUAGES: &[&Language] = &[
    &PLAIN_TEXT,
    &RUST,
    &TOML,
    &JSON,
    &JSON5,
    &MARKDOWN,
    &JAVASCRIPT,
    &TYPESCRIPT,
    &TSX,
    &HTML,
    &CSS,
    &SCSS,
    &PYTHON,
    &GO,
    &C,
    &CPP,
    &CSHARP,
    &JAVA,
    &KOTLIN,
    &SWIFT,
    &PHP,
    &RUBY,
    &BASH,
    &ZSH,
    &YAML,
    &SQL,
    &ZIG,
    &LUA,
    &DOCKERFILE,
    &ELIXIR,
    &ERLANG,
    &HASKELL,
    &OCAML,
    &SCALA,
    &CLOJURE,
    &DART,
    &GLSL,
    &MAKEFILE,
    &CMAKE,
    &NIX,
    &SOLIDITY,
    &GRAPHQL,
    &PROTOBUF,
    &ASTRO,
    &SVELTE,
    &VUE,
    &XML,
    &INI,
    &DIFF,
];

/// Registry for fast language lookup by extension or name.
#[derive(Debug, Clone, Default)]
pub struct LanguageRegistry {
    by_ext: HashMap<&'static str, &'static Language>,
    by_name: HashMap<&'static str, &'static Language>,
}

impl LanguageRegistry {
    pub fn builtin() -> Self {
        let mut by_ext = HashMap::new();
        let mut by_name = HashMap::new();
        for &lang in ALL_LANGUAGES {
            by_name.insert(lang.name, lang);
            for ext in lang.extensions {
                by_ext.insert(*ext, lang);
            }
        }
        Self { by_ext, by_name }
    }

    pub fn for_extension(&self, ext: &str) -> Option<&'static Language> {
        self.by_ext.get(ext).copied()
    }

    pub fn for_name(&self, name: &str) -> Option<&'static Language> {
        self.by_name.get(name).copied()
    }

    pub fn for_path(path: &std::path::Path) -> Option<&'static Language> {
        if let Some(ext) = path.extension().and_then(|s| s.to_str()) {
            if let Some(lang) = ALL_LANGUAGES.iter().find(|l| l.extensions.contains(&ext)) {
                return Some(lang);
            }
        }
        if let Some(file_name) = path.file_name().and_then(|s| s.to_str()) {
            if let Some(lang) = ALL_LANGUAGES.iter().find(|l| l.extensions.contains(&file_name)) {
                return Some(lang);
            }
        }
        None
    }

    pub fn guess(path_or_ext: Option<&str>, content: &str) -> &'static Language {
        let lumis_lang = lumis::languages::Language::guess(path_or_ext, content);
        ALL_LANGUAGES
            .iter()
            .find(|l| l.lumis_lang == lumis_lang)
            .copied()
            .unwrap_or(&PLAIN_TEXT)
    }
}
