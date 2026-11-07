//! Fast, zero-dependency syntax highlighting and editor line-gutter helpers
//! tailored to Tessel's seven themes (`theme_palettes.rs`).
//!
//! Guarantees:
//! - `build_layout_job` preserves the exact input `&str` byte-for-byte and emits contiguous,
//!   UTF-8-valid `LayoutSection` ranges covering `0..text.len()`, required by `egui::TextEdit`.
//! - Files larger than [`SYNTAX_MAX_BYTES`] (`512 KiB`) or unknown extensions fall back to O(1)
//!   plain monospace rendering so 60fps typing never stalls on multi-megabyte buffers.
//! - [`SyntaxCache`] memoizes the laid-out `Arc<Galley>` across frames until text content, font
//!   size, wrap width, word-wrap mode, or theme changes.

use std::hash::{Hash, Hasher};
use std::path::Path;
use std::sync::Arc;

use eframe::egui::{self, Color32, FontId, Galley, TextFormat};

use crate::theme_palettes::ThemeId;

fn mix_color(a: Color32, b: Color32, t: f32) -> Color32 {
    let inv = 1.0_f32 - t;
    Color32::from_rgb(
        ((a.r() as f32) * inv + (b.r() as f32) * t).round() as u8,
        ((a.g() as f32) * inv + (b.g() as f32) * t).round() as u8,
        ((a.b() as f32) * inv + (b.b() as f32) * t).round() as u8,
    )
}

/// Maximum UTF-8 byte length tokenized by [`build_layout_job`]; larger buffers render plain.
pub const SYNTAX_MAX_BYTES: usize = 512 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SyntaxLang {
    Plain,
    Rust,
    TypeScript,
    Python,
    Shell,
    CLike,
    Json,
    Toml,
    Yaml,
    Markup,
    Css,
    Sql,
    Markdown,
}

impl SyntaxLang {
    #[must_use]
    pub fn from_path(path: &Path) -> Self {
        let file_name = path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        if matches!(
            file_name.as_str(),
            "dockerfile" | "makefile" | ".bashrc" | ".zshrc" | ".profile"
        ) {
            return Self::Shell;
        }
        if file_name.starts_with(".env") || file_name == ".gitignore" {
            return Self::Toml;
        }
        let ext = path
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        match ext.as_str() {
            "rs" => Self::Rust,
            "ts" | "tsx" | "mts" | "cts" | "js" | "jsx" | "mjs" | "cjs" => Self::TypeScript,
            "py" | "pyi" | "rb" => Self::Python,
            "sh" | "bash" | "zsh" | "ps1" | "psm1" | "bat" | "cmd" => Self::Shell,
            "c" | "h" | "cpp" | "hpp" | "cc" | "cxx" | "cs" | "go" | "java" | "kt" | "swift"
            | "zig" => Self::CLike,
            "json" | "jsonc" | "jsonl" => Self::Json,
            "toml" | "ini" | "cfg" | "conf" => Self::Toml,
            "yaml" | "yml" => Self::Yaml,
            "html" | "htm" | "vue" | "svelte" | "xml" | "svg" => Self::Markup,
            "css" | "scss" | "sass" | "less" => Self::Css,
            "sql" => Self::Sql,
            "md" | "markdown" => Self::Markdown,
            _ => Self::Plain,
        }
    }

    #[must_use]
    pub const fn display_name(self) -> &'static str {
        match self {
            Self::Plain => "Plain Text",
            Self::Rust => "Rust",
            Self::TypeScript => "TypeScript / JS",
            Self::Python => "Python",
            Self::Shell => "Shell",
            Self::CLike => "C / Go / Java",
            Self::Json => "JSON",
            Self::Toml => "TOML / Config",
            Self::Yaml => "YAML",
            Self::Markup => "HTML / Vue / XML",
            Self::Css => "CSS",
            Self::Sql => "SQL",
            Self::Markdown => "Markdown",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TokenKind {
    Plain,
    Comment,
    String,
    Number,
    Keyword,
    Type,
    Function,
    Attribute,
    Punctuation,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SyntaxPalette {
    pub plain: Color32,
    pub comment: Color32,
    pub string: Color32,
    pub number: Color32,
    pub keyword: Color32,
    pub type_name: Color32,
    pub function: Color32,
    pub attribute: Color32,
    pub punctuation: Color32,
}

impl SyntaxPalette {
    #[must_use]
    pub fn for_theme(theme: ThemeId) -> Self {
        Self::for_theme_mode(theme, true)
    }

    #[must_use]
    pub fn for_theme_mode(theme: ThemeId, dark: bool) -> Self {
        if !dark && theme == ThemeId::Classic {
            return Self {
                plain: Color32::from_rgb(0x24, 0x29, 0x2f),
                comment: Color32::from_rgb(0x6e, 0x77, 0x81),
                string: Color32::from_rgb(0x11, 0x63, 0x29),
                number: Color32::from_rgb(0x95, 0x38, 0x00),
                keyword: Color32::from_rgb(0xcf, 0x22, 0x2e),
                type_name: Color32::from_rgb(0x82, 0x50, 0xdf),
                function: Color32::from_rgb(0x05, 0x50, 0xae),
                attribute: Color32::from_rgb(0x09, 0x69, 0xda),
                punctuation: Color32::from_rgb(0x57, 0x60, 0x6a),
            };
        }
        let spec = theme.spec();
        if let Some(ansi) = &spec.term.ansi {
            Self {
                plain: spec.ui.text,
                comment: spec.ui.text_dim,
                string: ansi.green,
                number: ansi.bright_magenta,
                keyword: ansi.magenta,
                type_name: ansi.yellow,
                function: ansi.blue,
                attribute: ansi.cyan,
                punctuation: mix_color(spec.ui.text, spec.ui.text_dim, 0.35),
            }
        } else {
            match theme {
                ThemeId::Warp => Self {
                    plain: spec.ui.text,
                    comment: Color32::from_rgb(0x76, 0x83, 0x90),
                    string: Color32::from_rgb(0xa1, 0xc8, 0xa7),
                    number: Color32::from_rgb(0xe2, 0xb8, 0x6b),
                    keyword: Color32::from_rgb(0xc7, 0x92, 0xea),
                    type_name: Color32::from_rgb(0x82, 0xaa, 0xff),
                    function: Color32::from_rgb(0x7f, 0xdb, 0xca),
                    attribute: Color32::from_rgb(0xf7, 0x8c, 0x6c),
                    punctuation: Color32::from_rgb(0xa8, 0xb2, 0xc1),
                },
                _ => Self {
                    plain: spec.ui.text,
                    comment: Color32::from_rgb(0x7f, 0x84, 0x8e),
                    string: Color32::from_rgb(0x98, 0xc3, 0x79),
                    number: Color32::from_rgb(0xd1, 0x9a, 0x66),
                    keyword: Color32::from_rgb(0xc6, 0x78, 0xdd),
                    type_name: Color32::from_rgb(0xe5, 0xc0, 0x7b),
                    function: Color32::from_rgb(0x61, 0xaf, 0xef),
                    attribute: Color32::from_rgb(0x56, 0xb6, 0xc2),
                    punctuation: Color32::from_rgb(0xab, 0xb2, 0xbf),
                },
            }
        }
    }

    #[must_use]
    pub const fn color(self, kind: TokenKind) -> Color32 {
        match kind {
            TokenKind::Plain => self.plain,
            TokenKind::Comment => self.comment,
            TokenKind::String => self.string,
            TokenKind::Number => self.number,
            TokenKind::Keyword => self.keyword,
            TokenKind::Type => self.type_name,
            TokenKind::Function => self.function,
            TokenKind::Attribute => self.attribute,
            TokenKind::Punctuation => self.punctuation,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct SyntaxCache {
    text_hash: u64,
    text_len: usize,
    lang: Option<SyntaxLang>,
    theme: Option<ThemeId>,
    dark: bool,
    font_bits: u32,
    wrap_bits: u32,
    word_wrap: bool,
    enabled: bool,
    galley: Option<Arc<Galley>>,
}

impl SyntaxCache {
    pub fn invalidate(&mut self) {
        self.galley = None;
    }

    #[allow(clippy::too_many_arguments)]
    pub fn layout(
        &mut self,
        ui: &egui::Ui,
        text: &str,
        lang: SyntaxLang,
        theme: ThemeId,
        dark: bool,
        font: &FontId,
        wrap_width: f32,
        word_wrap: bool,
        enabled: bool,
    ) -> Arc<Galley> {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        text.hash(&mut hasher);
        let h = hasher.finish();
        let fb = font.size.to_bits();
        let wb = if word_wrap { wrap_width.to_bits() } else { 0 };
        if let Some(existing) = &self.galley
            && self.text_hash == h
            && self.text_len == text.len()
            && self.lang == Some(lang)
            && self.theme == Some(theme)
            && self.dark == dark
            && self.font_bits == fb
            && self.wrap_bits == wb
            && self.word_wrap == word_wrap
            && self.enabled == enabled
        {
            return Arc::clone(existing);
        }
        let job = build_layout_job_with_mode(
            text,
            if enabled { lang } else { SyntaxLang::Plain },
            theme,
            dark,
            font.clone(),
            wrap_width,
            word_wrap,
        );
        let galley = ui.fonts_mut(|f| f.layout_job(job));
        self.text_hash = h;
        self.text_len = text.len();
        self.lang = Some(lang);
        self.theme = Some(theme);
        self.dark = dark;
        self.font_bits = fb;
        self.wrap_bits = wb;
        self.word_wrap = word_wrap;
        self.enabled = enabled;
        self.galley = Some(Arc::clone(&galley));
        galley
    }
}

#[must_use]
pub fn build_layout_job(
    text: &str,
    lang: SyntaxLang,
    theme: ThemeId,
    font: FontId,
    wrap_width: f32,
    word_wrap: bool,
) -> egui::text::LayoutJob {
    build_layout_job_with_mode(text, lang, theme, true, font, wrap_width, word_wrap)
}

/// Returns non-overlapping `(start_byte, end_byte, TokenKind)` syntax spans for `text`
/// when `lang != SyntaxLang::Plain` and `text.len() <= SYNTAX_MAX_BYTES`.
#[must_use]
pub fn tokenize_spans(text: &str, lang: SyntaxLang) -> Vec<(usize, usize, TokenKind)> {
    if text.is_empty() || lang == SyntaxLang::Plain || text.len() > SYNTAX_MAX_BYTES {
        return Vec::new();
    }
    tokenize(text, lang)
}

#[must_use]
pub fn build_layout_job_with_mode(
    text: &str,
    lang: SyntaxLang,
    theme: ThemeId,
    dark: bool,
    font: FontId,
    wrap_width: f32,
    word_wrap: bool,
) -> egui::text::LayoutJob {
    let palette = SyntaxPalette::for_theme_mode(theme, dark);
    if text.is_empty() || lang == SyntaxLang::Plain || text.len() > SYNTAX_MAX_BYTES {
        let mut job =
            egui::text::LayoutJob::simple(text.to_owned(), font, palette.plain, wrap_width);
        job.wrap.break_anywhere = word_wrap;
        return job;
    }

    let spans = tokenize(text, lang);
    let mut sections = Vec::with_capacity(spans.len().max(1));
    let mut cursor = 0usize;
    for (start, end, kind) in spans {
        if start > cursor {
            sections.push(egui::text::LayoutSection {
                leading_space: 0.0,
                byte_range: cursor..start,
                format: TextFormat {
                    font_id: font.clone(),
                    color: palette.plain,
                    ..Default::default()
                },
            });
        }
        if end > start {
            sections.push(egui::text::LayoutSection {
                leading_space: 0.0,
                byte_range: start..end,
                format: TextFormat {
                    font_id: font.clone(),
                    color: palette.color(kind),
                    italics: kind == TokenKind::Comment,
                    ..Default::default()
                },
            });
            cursor = end;
        }
    }
    if cursor < text.len() {
        sections.push(egui::text::LayoutSection {
            leading_space: 0.0,
            byte_range: cursor..text.len(),
            format: TextFormat {
                font_id: font,
                color: palette.plain,
                ..Default::default()
            },
        });
    }

    let mut job = egui::text::LayoutJob {
        text: text.to_owned(),
        sections,
        ..Default::default()
    };
    job.wrap.max_width = wrap_width;
    job.wrap.break_anywhere = word_wrap;
    job
}

fn is_ident_start(b: u8) -> bool {
    b.is_ascii_alphabetic() || b == b'_' || b == b'$'
}

fn is_ident_continue(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'$' || b == b'-'
}

fn is_keyword(lang: SyntaxLang, word: &str) -> bool {
    match lang {
        SyntaxLang::Rust => matches!(
            word,
            "as" | "async"
                | "await"
                | "break"
                | "const"
                | "continue"
                | "crate"
                | "dyn"
                | "else"
                | "enum"
                | "extern"
                | "false"
                | "fn"
                | "for"
                | "if"
                | "impl"
                | "in"
                | "let"
                | "loop"
                | "match"
                | "mod"
                | "move"
                | "mut"
                | "pub"
                | "ref"
                | "return"
                | "self"
                | "Self"
                | "static"
                | "struct"
                | "super"
                | "trait"
                | "true"
                | "type"
                | "unsafe"
                | "use"
                | "where"
                | "while"
        ),
        SyntaxLang::TypeScript => matches!(
            word,
            "async"
                | "await"
                | "break"
                | "case"
                | "catch"
                | "class"
                | "const"
                | "continue"
                | "debugger"
                | "default"
                | "delete"
                | "do"
                | "else"
                | "enum"
                | "export"
                | "extends"
                | "false"
                | "finally"
                | "for"
                | "from"
                | "function"
                | "if"
                | "implements"
                | "import"
                | "in"
                | "instanceof"
                | "interface"
                | "let"
                | "new"
                | "null"
                | "of"
                | "private"
                | "protected"
                | "public"
                | "readonly"
                | "return"
                | "static"
                | "super"
                | "switch"
                | "this"
                | "throw"
                | "true"
                | "try"
                | "type"
                | "typeof"
                | "undefined"
                | "var"
                | "void"
                | "while"
                | "yield"
        ),
        SyntaxLang::Python => matches!(
            word,
            "and"
                | "as"
                | "assert"
                | "async"
                | "await"
                | "break"
                | "class"
                | "continue"
                | "def"
                | "del"
                | "elif"
                | "else"
                | "except"
                | "False"
                | "finally"
                | "for"
                | "from"
                | "global"
                | "if"
                | "import"
                | "in"
                | "is"
                | "lambda"
                | "None"
                | "nonlocal"
                | "not"
                | "or"
                | "pass"
                | "raise"
                | "return"
                | "self"
                | "True"
                | "try"
                | "while"
                | "with"
                | "yield"
        ),
        SyntaxLang::Shell => matches!(
            word,
            "if" | "then"
                | "else"
                | "elif"
                | "fi"
                | "for"
                | "in"
                | "do"
                | "done"
                | "while"
                | "case"
                | "esac"
                | "function"
                | "return"
                | "exit"
                | "export"
                | "local"
                | "set"
                | "unset"
                | "param"
                | "foreach"
                | "switch"
                | "try"
                | "catch"
                | "finally"
        ),
        SyntaxLang::CLike => matches!(
            word,
            "auto"
                | "break"
                | "case"
                | "catch"
                | "class"
                | "const"
                | "continue"
                | "default"
                | "defer"
                | "do"
                | "else"
                | "enum"
                | "extern"
                | "false"
                | "for"
                | "func"
                | "go"
                | "goto"
                | "if"
                | "import"
                | "interface"
                | "map"
                | "namespace"
                | "new"
                | "nil"
                | "null"
                | "package"
                | "private"
                | "public"
                | "range"
                | "return"
                | "select"
                | "sizeof"
                | "static"
                | "struct"
                | "switch"
                | "template"
                | "this"
                | "throw"
                | "true"
                | "try"
                | "typedef"
                | "typename"
                | "using"
                | "var"
                | "virtual"
                | "void"
                | "while"
        ),
        SyntaxLang::Json | SyntaxLang::Toml | SyntaxLang::Yaml => {
            matches!(word, "true" | "false" | "null" | "yes" | "no")
        }
        SyntaxLang::Css => matches!(
            word,
            "important" | "media" | "import" | "keyframes" | "from" | "to"
        ),
        SyntaxLang::Sql => {
            let up = word.to_ascii_uppercase();
            matches!(
                up.as_str(),
                "SELECT"
                    | "FROM"
                    | "WHERE"
                    | "INSERT"
                    | "INTO"
                    | "VALUES"
                    | "UPDATE"
                    | "SET"
                    | "DELETE"
                    | "CREATE"
                    | "TABLE"
                    | "ALTER"
                    | "DROP"
                    | "INDEX"
                    | "JOIN"
                    | "LEFT"
                    | "RIGHT"
                    | "INNER"
                    | "OUTER"
                    | "ON"
                    | "GROUP"
                    | "BY"
                    | "ORDER"
                    | "ASC"
                    | "DESC"
                    | "LIMIT"
                    | "AND"
                    | "OR"
                    | "NOT"
                    | "NULL"
                    | "IS"
                    | "AS"
                    | "PRIMARY"
                    | "KEY"
                    | "FOREIGN"
                    | "REFERENCES"
                    | "BEGIN"
                    | "COMMIT"
                    | "ROLLBACK"
            )
        }
        _ => false,
    }
}

fn is_builtin_type(lang: SyntaxLang, word: &str) -> bool {
    if word.bytes().next().is_some_and(|b| b.is_ascii_uppercase())
        && word.bytes().any(|b| b.is_ascii_lowercase())
    {
        return true;
    }
    match lang {
        SyntaxLang::Rust => matches!(
            word,
            "bool"
                | "char"
                | "str"
                | "u8"
                | "u16"
                | "u32"
                | "u64"
                | "u128"
                | "usize"
                | "i8"
                | "i16"
                | "i32"
                | "i64"
                | "i128"
                | "isize"
                | "f32"
                | "f64"
        ),
        SyntaxLang::TypeScript => matches!(
            word,
            "string"
                | "number"
                | "boolean"
                | "any"
                | "unknown"
                | "never"
                | "bigint"
                | "symbol"
                | "object"
        ),
        SyntaxLang::CLike => matches!(
            word,
            "int"
                | "long"
                | "short"
                | "char"
                | "float"
                | "double"
                | "bool"
                | "byte"
                | "rune"
                | "string"
                | "error"
                | "size_t"
                | "uint8_t"
                | "uint16_t"
                | "uint32_t"
                | "uint64_t"
                | "int32_t"
                | "int64_t"
        ),
        _ => false,
    }
}

pub fn tokenize(text: &str, lang: SyntaxLang) -> Vec<(usize, usize, TokenKind)> {
    let bytes = text.as_bytes();
    let len = bytes.len();
    let mut out = Vec::new();
    let mut i = 0usize;

    if lang == SyntaxLang::Markdown {
        for line in text.split_inclusive('\n') {
            let start = line.as_ptr() as usize - text.as_ptr() as usize;
            let end = start + line.len();
            let trimmed = line.trim_start();
            if trimmed.starts_with('#') {
                out.push((start, end, TokenKind::Keyword));
            } else if trimmed.starts_with("```") {
                out.push((start, end, TokenKind::Attribute));
            } else if trimmed.starts_with("> ") {
                out.push((start, end, TokenKind::Comment));
            } else if trimmed.starts_with("- ")
                || trimmed.starts_with("* ")
                || trimmed.starts_with("+ ")
            {
                let lead = line.len() - trimmed.len();
                out.push((start + lead, start + lead + 2, TokenKind::Keyword));
            }
        }
        return out;
    }

    let mut line_start = true;
    while i < len {
        let b = bytes[i];
        if b == b'\n' {
            line_start = true;
            i += 1;
            continue;
        }
        if b.is_ascii_whitespace() {
            i += 1;
            continue;
        }

        // HTML / XML / Vue comments `<!-- ... -->`
        if lang == SyntaxLang::Markup && bytes[i..].starts_with(b"<!--") {
            let start = i;
            i += 4;
            while i < len && !bytes[i..].starts_with(b"-->") {
                i += 1;
            }
            i = (i + 3).min(len);
            out.push((start, i, TokenKind::Comment));
            line_start = false;
            continue;
        }

        // Line comments (`//`, `--` in SQL, `#` in Python/Shell/TOML/YAML)
        let slash_comment = matches!(
            lang,
            SyntaxLang::Rust
                | SyntaxLang::TypeScript
                | SyntaxLang::CLike
                | SyntaxLang::Json
                | SyntaxLang::Css
        ) && bytes[i..].starts_with(b"//");
        let hash_comment = matches!(
            lang,
            SyntaxLang::Python | SyntaxLang::Shell | SyntaxLang::Toml | SyntaxLang::Yaml
        ) && b == b'#';
        let sql_comment = lang == SyntaxLang::Sql && bytes[i..].starts_with(b"--");
        if slash_comment || hash_comment || sql_comment {
            let start = i;
            while i < len && bytes[i] != b'\n' {
                i += 1;
            }
            out.push((start, i, TokenKind::Comment));
            continue;
        }

        // Block comments `/* ... */`
        if matches!(
            lang,
            SyntaxLang::Rust
                | SyntaxLang::TypeScript
                | SyntaxLang::CLike
                | SyntaxLang::Css
                | SyntaxLang::Sql
        ) && bytes[i..].starts_with(b"/*")
        {
            let start = i;
            i += 2;
            while i < len && !bytes[i..].starts_with(b"*/") {
                i += 1;
            }
            i = (i + 2).min(len);
            out.push((start, i, TokenKind::Comment));
            line_start = false;
            continue;
        }

        // Rust attributes `#[...]` / `#![...]` or decorators `@...`
        if (lang == SyntaxLang::Rust
            && b == b'#'
            && (bytes.get(i + 1) == Some(&b'[') || bytes.get(i + 1) == Some(&b'!')))
            || (matches!(
                lang,
                SyntaxLang::Python | SyntaxLang::TypeScript | SyntaxLang::CLike | SyntaxLang::Css
            ) && b == b'@')
        {
            let start = i;
            i += 1;
            if b == b'#' {
                let mut depth = 0i32;
                while i < len {
                    if bytes[i] == b'[' {
                        depth += 1;
                    } else if bytes[i] == b']' {
                        depth -= 1;
                        if depth <= 0 {
                            i += 1;
                            break;
                        }
                    } else if bytes[i] == b'\n' && depth == 0 {
                        break;
                    }
                    i += 1;
                }
            } else {
                while i < len && is_ident_continue(bytes[i]) {
                    i += 1;
                }
            }
            out.push((start, i, TokenKind::Attribute));
            line_start = false;
            continue;
        }

        // TOML section headers `[section]` at start of line
        if lang == SyntaxLang::Toml && line_start && b == b'[' {
            let start = i;
            while i < len && bytes[i] != b'\n' {
                i += 1;
            }
            out.push((start, i, TokenKind::Type));
            continue;
        }

        // Markup tags `<tag ...>` / `</tag>`
        if lang == SyntaxLang::Markup && b == b'<' {
            let start = i;
            i += 1;
            if i < len && (bytes[i] == b'/' || bytes[i] == b'!' || bytes[i] == b'?') {
                i += 1;
            }
            let name_start = i;
            while i < len && is_ident_continue(bytes[i]) {
                i += 1;
            }
            if i > name_start {
                out.push((start, i, TokenKind::Keyword));
            } else {
                out.push((start, i, TokenKind::Punctuation));
            }
            line_start = false;
            continue;
        }

        // Strings (`"..."`, `'...'`, `` `...` ``)
        if b == b'"'
            || b == b'\''
            || (matches!(
                lang,
                SyntaxLang::TypeScript | SyntaxLang::CLike | SyntaxLang::Markdown
            ) && b == b'`')
        {
            // Skip Rust lifetime `'a` so `'static` highlights as attribute instead of runaway string
            if lang == SyntaxLang::Rust
                && b == b'\''
                && bytes.get(i + 1).is_some_and(|&c| is_ident_start(c))
                && bytes.get(i + 2) != Some(&b'\'')
            {
                let start = i;
                i += 2;
                while i < len && is_ident_continue(bytes[i]) {
                    i += 1;
                }
                out.push((start, i, TokenKind::Attribute));
                line_start = false;
                continue;
            }
            let quote = b;
            let start = i;
            i += 1;
            while i < len {
                if bytes[i] == b'\\' {
                    // Advance safely over escaped UTF-8 char
                    i += 1;
                    if i < len {
                        let ch_len = text[i..].chars().next().map_or(1, char::len_utf8);
                        i += ch_len;
                    }
                    continue;
                }
                if bytes[i] == quote {
                    i += 1;
                    break;
                }
                if bytes[i] == b'\n' && quote != b'`' {
                    break;
                }
                let ch_len = text[i..].chars().next().map_or(1, char::len_utf8);
                i += ch_len;
            }
            // In JSON, `"key":` highlights as Attribute (property key)
            let mut look = i;
            while look < len && (bytes[look] == b' ' || bytes[look] == b'\t') {
                look += 1;
            }
            let kind = if lang == SyntaxLang::Json && bytes.get(look) == Some(&b':') {
                TokenKind::Attribute
            } else {
                TokenKind::String
            };
            out.push((start, i, kind));
            line_start = false;
            continue;
        }

        // Numbers
        if b.is_ascii_digit() {
            let start = i;
            i += 1;
            while i < len
                && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_' || bytes[i] == b'.')
            {
                i += 1;
            }
            out.push((start, i, TokenKind::Number));
            line_start = false;
            continue;
        }

        // Shell / PHP / PowerShell variables `$foo`
        if matches!(lang, SyntaxLang::Shell | SyntaxLang::Yaml) && b == b'$' {
            let start = i;
            i += 1;
            while i < len && (is_ident_continue(bytes[i]) || bytes[i] == b'{' || bytes[i] == b'}') {
                i += 1;
            }
            out.push((start, i, TokenKind::Attribute));
            line_start = false;
            continue;
        }

        // Identifiers, keywords, types, function calls, YAML/TOML keys
        if is_ident_start(b) {
            let start = i;
            i += 1;
            while i < len
                && (bytes[i].is_ascii_alphanumeric()
                    || bytes[i] == b'_'
                    || bytes[i] == b'$'
                    || (matches!(
                        lang,
                        SyntaxLang::Toml | SyntaxLang::Yaml | SyntaxLang::Css | SyntaxLang::Markup
                    ) && bytes[i] == b'-'))
            {
                i += 1;
            }
            let word = &text[start..i];
            let mut look = i;
            while look < len && (bytes[look] == b' ' || bytes[look] == b'\t') {
                look += 1;
            }
            let kind = if matches!(lang, SyntaxLang::Yaml | SyntaxLang::Toml)
                && line_start
                && matches!(bytes.get(look), Some(&b':' | &b'='))
            {
                TokenKind::Attribute
            } else if lang == SyntaxLang::Rust && bytes.get(i) == Some(&b'!') {
                i += 1;
                TokenKind::Attribute
            } else if is_keyword(lang, word) {
                TokenKind::Keyword
            } else if is_builtin_type(lang, word) {
                TokenKind::Type
            } else if bytes.get(look) == Some(&b'(') {
                TokenKind::Function
            } else if lang == SyntaxLang::Markup && bytes.get(look) == Some(&b'=') {
                TokenKind::Attribute
            } else {
                TokenKind::Plain
            };
            if kind != TokenKind::Plain {
                out.push((start, i, kind));
            }
            line_start = false;
            continue;
        }

        // Non-ASCII UTF-8 sequence: advance cleanly by scalar length
        if !b.is_ascii() {
            let ch_len = text[i..].chars().next().map_or(1, char::len_utf8);
            i += ch_len;
            line_start = false;
            continue;
        }

        // Punctuation
        let start = i;
        i += 1;
        out.push((start, i, TokenKind::Punctuation));
        line_start = false;
    }

    out
}

/// Returns a `Vec<bool>` indexed by 0-based line in `current` marking lines changed from `original`.
#[must_use]
pub fn changed_line_flags(original: &str, current: &str) -> Vec<bool> {
    let curr_lines: Vec<&str> = current.split('\n').collect();
    if original == current {
        return vec![false; curr_lines.len()];
    }
    let orig_lines: Vec<&str> = original.split('\n').collect();
    let mut flags = vec![true; curr_lines.len()];
    let mut prefix = 0usize;
    while prefix < orig_lines.len()
        && prefix < curr_lines.len()
        && orig_lines[prefix] == curr_lines[prefix]
    {
        flags[prefix] = false;
        prefix += 1;
    }
    let mut o_end = orig_lines.len();
    let mut c_end = curr_lines.len();
    while o_end > prefix && c_end > prefix && orig_lines[o_end - 1] == curr_lines[c_end - 1] {
        flags[c_end - 1] = false;
        o_end -= 1;
        c_end -= 1;
    }
    flags
}

/// Computes the pixel width of the left editor line-number + modified-line gutter.
#[must_use]
pub fn editor_gutter_width(ui: &egui::Ui, font: &FontId, line_count: usize) -> f32 {
    let digits = (line_count.max(99).ilog10() as usize) + 1;
    let digit_w = ui.fonts_mut(|f| f.glyph_width(font, '8')).max(7.0_f32);
    (digits as f32) * digit_w + 18.0_f32
}

/// Paints 1-based line numbers (at logical line starts, skipping wrapped sub-rows) and a `2.5px`
/// changed-line accent bar aligned with the exact `Galley` rows of `egui::TextEdit`.
/// Returns the number of logical line numbers painted.
#[allow(clippy::too_many_arguments)]
pub fn paint_editor_gutter(
    painter: &egui::Painter,
    gutter_x_min: f32,
    gutter_x_max: f32,
    galley_pos: egui::Pos2,
    galley: &Galley,
    font: &FontId,
    changed_flags: &[bool],
    show_line_numbers: bool,
    text_dim: Color32,
    border_color: Color32,
    changed_bar_color: Color32,
) -> usize {
    let mut logical_line = 0usize;
    let mut at_line_start = true;
    let mut painted_numbers = 0usize;
    let _ = gutter_x_min;
    for placed_row in &galley.rows {
        let row_rect = placed_row.rect();
        let y_min = galley_pos.y + row_rect.min.y;
        let y_max = galley_pos.y + row_rect.max.y.max(row_rect.min.y + 12.0_f32);
        if changed_flags.get(logical_line).copied().unwrap_or(false) {
            let bar = egui::Rect::from_min_max(
                egui::pos2(gutter_x_max - 4.0_f32, y_min),
                egui::pos2(gutter_x_max - 1.5_f32, y_max),
            );
            painter.rect_filled(bar, 1.0_f32, changed_bar_color);
        }
        if show_line_numbers && at_line_start {
            painter.text(
                egui::pos2(gutter_x_max - 8.0_f32, y_min),
                egui::Align2::RIGHT_TOP,
                (logical_line + 1).to_string(),
                font.clone(),
                text_dim,
            );
            painted_numbers += 1;
        }
        at_line_start = placed_row.ends_with_newline;
        if placed_row.ends_with_newline {
            logical_line += 1;
        }
    }
    let y_end = galley_pos.y + galley.size().y.max(20.0_f32);
    painter.line_segment(
        [
            egui::pos2(gutter_x_max, galley_pos.y),
            egui::pos2(gutter_x_max, y_end),
        ],
        egui::Stroke::new(1.0_f32, border_color),
    );
    painted_numbers
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme_palettes::THEMES;
    use std::path::PathBuf;

    #[test]
    fn language_detection_covers_code_config_markup_and_dotfiles() {
        assert_eq!(
            SyntaxLang::from_path(&PathBuf::from("src/app.rs")),
            SyntaxLang::Rust
        );
        assert_eq!(
            SyntaxLang::from_path(&PathBuf::from("web/View.tsx")),
            SyntaxLang::TypeScript
        );
        assert_eq!(
            SyntaxLang::from_path(&PathBuf::from("App.vue")),
            SyntaxLang::Markup
        );
        assert_eq!(
            SyntaxLang::from_path(&PathBuf::from(".env.local")),
            SyntaxLang::Toml
        );
        assert_eq!(
            SyntaxLang::from_path(&PathBuf::from("Cargo.toml")),
            SyntaxLang::Toml
        );
        assert_eq!(
            SyntaxLang::from_path(&PathBuf::from("notes.unknown")),
            SyntaxLang::Plain
        );
    }

    #[test]
    fn layout_job_preserves_exact_utf8_bytes_and_valid_char_boundaries_across_all_themes() {
        let sample = concat!(
            "// Commentaire accentué éàü 🚀\n",
            "#[derive(Debug, Clone)]\n",
            "pub struct Config<'static> {\n",
            "    pub count: usize,\n",
            "}\n",
            "fn compute(x: usize) -> &'static str {\n",
            "    println!(\"résultat: {}\", x + 42);\n",
            "    \"ok\"\n",
            "}\n"
        );
        for theme in THEMES {
            let job = build_layout_job(
                sample,
                SyntaxLang::Rust,
                theme,
                FontId::monospace(13.0),
                f32::INFINITY,
                false,
            );
            assert_eq!(job.text, sample);
            let mut expected_cursor = 0usize;
            for sec in &job.sections {
                assert_eq!(sec.byte_range.start, expected_cursor);
                assert!(sec.byte_range.end > sec.byte_range.start);
                assert!(sample.is_char_boundary(sec.byte_range.start));
                assert!(sample.is_char_boundary(sec.byte_range.end));
                expected_cursor = sec.byte_range.end;
            }
            assert_eq!(expected_cursor, sample.len());
            assert!(
                job.sections.len() > 10,
                "Rust tokens must produce distinct sections"
            );
        }
    }

    #[test]
    fn oversized_buffer_falls_back_to_single_plain_section() {
        let huge = "let x = 1;\n".repeat((SYNTAX_MAX_BYTES / 10) + 50);
        assert!(huge.len() > SYNTAX_MAX_BYTES);
        let job = build_layout_job(
            &huge,
            SyntaxLang::Rust,
            ThemeId::Dracula,
            FontId::monospace(13.0),
            f32::INFINITY,
            false,
        );
        assert_eq!(job.sections.len(), 1);
        assert_eq!(job.sections[0].byte_range, 0..huge.len());
    }

    #[test]
    fn changed_line_flags_marks_only_modified_middle_lines() {
        let orig = "a\nb\nc\nd\n";
        let curr = "a\nb_edited\ninserted\nc\nd\n";
        assert_eq!(
            changed_line_flags(orig, curr),
            vec![false, true, true, false, false, false]
        );
    }
}
