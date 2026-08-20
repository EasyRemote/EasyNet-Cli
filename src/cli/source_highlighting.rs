//! AST-backed source rendering for interactive CLI output.
//!
//! Markdown parsing, language grammars, semantic token projection, and the
//! terminal theme are deliberately separate. Grammar-specific capture names
//! never escape this module: callers render repository-owned semantic kinds.

use std::cell::RefCell;
use std::collections::HashMap;
use std::io::{self, Write};
use std::ops::Range;
use std::sync::OnceLock;

use termimad::crossterm::style::{
    Attribute, Color, SetAttribute, SetBackgroundColor, SetForegroundColor,
};
use termimad::crossterm::QueueableCommand;
use termimad::MadSkin;
use tree_sitter::{Language, Parser};
use tree_sitter_highlight::{Highlight, HighlightConfiguration, HighlightEvent, Highlighter};

const MAX_HIGHLIGHT_BYTES: usize = 1024 * 1024;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
enum SourceLanguage {
    Bash,
    Css,
    Go,
    Html,
    Java,
    JavaScript,
    Json,
    Python,
    Rust,
    Swift,
    Toml,
    TypeScript,
    Tsx,
    Yaml,
}

impl SourceLanguage {
    fn from_fence_info(info: &str) -> Option<Self> {
        let first = info.split_whitespace().next()?;
        let normalized = first
            .strip_prefix("{.")
            .and_then(|value| value.strip_suffix('}'))
            .unwrap_or(first);
        let normalized = normalized
            .strip_prefix("language-")
            .unwrap_or(normalized)
            .trim_start_matches('.')
            .to_ascii_lowercase();

        match normalized.as_str() {
            "bash" | "sh" | "shell" => Some(Self::Bash),
            "css" => Some(Self::Css),
            "go" | "golang" => Some(Self::Go),
            "html" | "htm" => Some(Self::Html),
            "java" => Some(Self::Java),
            "javascript" | "js" | "jsx" | "mjs" | "node" => Some(Self::JavaScript),
            "json" => Some(Self::Json),
            "py" | "python" => Some(Self::Python),
            "rs" | "rust" => Some(Self::Rust),
            "swift" => Some(Self::Swift),
            "toml" => Some(Self::Toml),
            "ts" | "typescript" => Some(Self::TypeScript),
            "tsx" => Some(Self::Tsx),
            "yaml" | "yml" => Some(Self::Yaml),
            _ => None,
        }
    }

    const fn canonical_name(self) -> &'static str {
        match self {
            Self::Bash => "bash",
            Self::Css => "css",
            Self::Go => "go",
            Self::Html => "html",
            Self::Java => "java",
            Self::JavaScript => "javascript",
            Self::Json => "json",
            Self::Python => "python",
            Self::Rust => "rust",
            Self::Swift => "swift",
            Self::Toml => "toml",
            Self::TypeScript => "typescript",
            Self::Tsx => "tsx",
            Self::Yaml => "yaml",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SemanticTokenKind {
    Default,
    Attribute,
    Comment,
    Constant,
    Constructor,
    Embedded,
    Escape,
    Function,
    Keyword,
    Label,
    Number,
    Operator,
    Property,
    Punctuation,
    String,
    Tag,
    Type,
    Variable,
}

struct HighlightClass {
    name: &'static str,
    kind: SemanticTokenKind,
}

const HIGHLIGHT_CLASSES: &[HighlightClass] = &[
    HighlightClass {
        name: "attribute",
        kind: SemanticTokenKind::Attribute,
    },
    HighlightClass {
        name: "comment.documentation",
        kind: SemanticTokenKind::Comment,
    },
    HighlightClass {
        name: "comment",
        kind: SemanticTokenKind::Comment,
    },
    HighlightClass {
        name: "constant.builtin",
        kind: SemanticTokenKind::Constant,
    },
    HighlightClass {
        name: "constant.macro",
        kind: SemanticTokenKind::Constant,
    },
    HighlightClass {
        name: "constant",
        kind: SemanticTokenKind::Constant,
    },
    HighlightClass {
        name: "boolean",
        kind: SemanticTokenKind::Constant,
    },
    HighlightClass {
        name: "constructor",
        kind: SemanticTokenKind::Constructor,
    },
    HighlightClass {
        name: "embedded",
        kind: SemanticTokenKind::Embedded,
    },
    HighlightClass {
        name: "escape",
        kind: SemanticTokenKind::Escape,
    },
    HighlightClass {
        name: "character.special",
        kind: SemanticTokenKind::Escape,
    },
    HighlightClass {
        name: "function.builtin",
        kind: SemanticTokenKind::Function,
    },
    HighlightClass {
        name: "function.macro",
        kind: SemanticTokenKind::Function,
    },
    HighlightClass {
        name: "function.method",
        kind: SemanticTokenKind::Function,
    },
    HighlightClass {
        name: "function.call",
        kind: SemanticTokenKind::Function,
    },
    HighlightClass {
        name: "function",
        kind: SemanticTokenKind::Function,
    },
    HighlightClass {
        name: "keyword",
        kind: SemanticTokenKind::Keyword,
    },
    HighlightClass {
        name: "charset",
        kind: SemanticTokenKind::Keyword,
    },
    HighlightClass {
        name: "import",
        kind: SemanticTokenKind::Keyword,
    },
    HighlightClass {
        name: "keyframes",
        kind: SemanticTokenKind::Keyword,
    },
    HighlightClass {
        name: "media",
        kind: SemanticTokenKind::Keyword,
    },
    HighlightClass {
        name: "namespace",
        kind: SemanticTokenKind::Keyword,
    },
    HighlightClass {
        name: "supports",
        kind: SemanticTokenKind::Keyword,
    },
    HighlightClass {
        name: "label",
        kind: SemanticTokenKind::Label,
    },
    HighlightClass {
        name: "number",
        kind: SemanticTokenKind::Number,
    },
    HighlightClass {
        name: "operator",
        kind: SemanticTokenKind::Operator,
    },
    HighlightClass {
        name: "property",
        kind: SemanticTokenKind::Property,
    },
    HighlightClass {
        name: "punctuation.bracket",
        kind: SemanticTokenKind::Punctuation,
    },
    HighlightClass {
        name: "punctuation.delimiter",
        kind: SemanticTokenKind::Punctuation,
    },
    HighlightClass {
        name: "punctuation.special",
        kind: SemanticTokenKind::Punctuation,
    },
    HighlightClass {
        name: "string.escape",
        kind: SemanticTokenKind::Escape,
    },
    HighlightClass {
        name: "string.regexp",
        kind: SemanticTokenKind::String,
    },
    HighlightClass {
        name: "string.special",
        kind: SemanticTokenKind::String,
    },
    HighlightClass {
        name: "string",
        kind: SemanticTokenKind::String,
    },
    HighlightClass {
        name: "tag",
        kind: SemanticTokenKind::Tag,
    },
    HighlightClass {
        name: "type.builtin",
        kind: SemanticTokenKind::Type,
    },
    HighlightClass {
        name: "type",
        kind: SemanticTokenKind::Type,
    },
    HighlightClass {
        name: "variable.builtin",
        kind: SemanticTokenKind::Variable,
    },
    HighlightClass {
        name: "variable.parameter",
        kind: SemanticTokenKind::Variable,
    },
    HighlightClass {
        name: "variable.member",
        kind: SemanticTokenKind::Variable,
    },
    HighlightClass {
        name: "variable",
        kind: SemanticTokenKind::Variable,
    },
];

struct LanguageProfile {
    parser_language: Language,
    configuration: Result<HighlightConfiguration, String>,
}

struct LanguageRegistry {
    profiles: HashMap<SourceLanguage, LanguageProfile>,
}

impl LanguageRegistry {
    fn new() -> Self {
        let javascript_highlights = format!(
            "{}\n{}",
            tree_sitter_javascript::HIGHLIGHT_QUERY,
            tree_sitter_javascript::JSX_HIGHLIGHT_QUERY
        );
        let typescript_highlights = format!(
            "{}\n{}",
            tree_sitter_javascript::HIGHLIGHT_QUERY,
            tree_sitter_typescript::HIGHLIGHTS_QUERY
        );
        let tsx_highlights = format!(
            "{}\n{}",
            javascript_highlights,
            tree_sitter_typescript::HIGHLIGHTS_QUERY
        );

        let definitions = [
            profile(
                SourceLanguage::Bash,
                tree_sitter_bash::LANGUAGE.into(),
                tree_sitter_bash::HIGHLIGHT_QUERY.to_owned(),
                "",
                "",
            ),
            profile(
                SourceLanguage::Css,
                tree_sitter_css::LANGUAGE.into(),
                tree_sitter_css::HIGHLIGHTS_QUERY.to_owned(),
                "",
                "",
            ),
            profile(
                SourceLanguage::Go,
                tree_sitter_go::LANGUAGE.into(),
                tree_sitter_go::HIGHLIGHTS_QUERY.to_owned(),
                "",
                "",
            ),
            profile(
                SourceLanguage::Html,
                tree_sitter_html::LANGUAGE.into(),
                tree_sitter_html::HIGHLIGHTS_QUERY.to_owned(),
                tree_sitter_html::INJECTIONS_QUERY,
                "",
            ),
            profile(
                SourceLanguage::Java,
                tree_sitter_java::LANGUAGE.into(),
                tree_sitter_java::HIGHLIGHTS_QUERY.to_owned(),
                "",
                "",
            ),
            profile(
                SourceLanguage::JavaScript,
                tree_sitter_javascript::LANGUAGE.into(),
                javascript_highlights.clone(),
                tree_sitter_javascript::INJECTIONS_QUERY,
                tree_sitter_javascript::LOCALS_QUERY,
            ),
            profile(
                SourceLanguage::Json,
                tree_sitter_json::LANGUAGE.into(),
                tree_sitter_json::HIGHLIGHTS_QUERY.to_owned(),
                "",
                "",
            ),
            profile(
                SourceLanguage::Python,
                tree_sitter_python::LANGUAGE.into(),
                tree_sitter_python::HIGHLIGHTS_QUERY.to_owned(),
                "",
                "",
            ),
            profile(
                SourceLanguage::Rust,
                tree_sitter_rust::LANGUAGE.into(),
                tree_sitter_rust::HIGHLIGHTS_QUERY.to_owned(),
                tree_sitter_rust::INJECTIONS_QUERY,
                "",
            ),
            profile(
                SourceLanguage::Swift,
                tree_sitter_swift::LANGUAGE.into(),
                tree_sitter_swift::HIGHLIGHTS_QUERY.to_owned(),
                tree_sitter_swift::INJECTIONS_QUERY,
                tree_sitter_swift::LOCALS_QUERY,
            ),
            profile(
                SourceLanguage::Toml,
                tree_sitter_toml_ng::LANGUAGE.into(),
                tree_sitter_toml_ng::HIGHLIGHTS_QUERY.to_owned(),
                "",
                "",
            ),
            profile(
                SourceLanguage::TypeScript,
                tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
                typescript_highlights.clone(),
                tree_sitter_javascript::INJECTIONS_QUERY,
                tree_sitter_typescript::LOCALS_QUERY,
            ),
            profile(
                SourceLanguage::Tsx,
                tree_sitter_typescript::LANGUAGE_TSX.into(),
                tsx_highlights,
                tree_sitter_javascript::INJECTIONS_QUERY,
                tree_sitter_typescript::LOCALS_QUERY,
            ),
            profile(
                SourceLanguage::Yaml,
                tree_sitter_yaml::LANGUAGE.into(),
                tree_sitter_yaml::HIGHLIGHTS_QUERY.to_owned(),
                "",
                "",
            ),
        ];

        Self {
            profiles: definitions.into_iter().collect(),
        }
    }

    fn profile(&self, language: SourceLanguage) -> Option<&LanguageProfile> {
        self.profiles.get(&language)
    }

    fn injection_configuration(&self, name: &str) -> Option<&HighlightConfiguration> {
        let language = SourceLanguage::from_fence_info(name)?;
        self.profile(language)?.configuration.as_ref().ok()
    }
}

fn profile(
    language: SourceLanguage,
    parser_language: Language,
    highlights_query: String,
    injections_query: &str,
    locals_query: &str,
) -> (SourceLanguage, LanguageProfile) {
    let configuration = HighlightConfiguration::new(
        parser_language.clone(),
        language.canonical_name(),
        &highlights_query,
        injections_query,
        locals_query,
    )
    .map(|mut configuration| {
        let names = HIGHLIGHT_CLASSES
            .iter()
            .map(|class| class.name)
            .collect::<Vec<_>>();
        configuration.configure(&names);
        configuration
    })
    .map_err(|error| error.to_string());

    (
        language,
        LanguageProfile {
            parser_language,
            configuration,
        },
    )
}

fn language_registry() -> &'static LanguageRegistry {
    static REGISTRY: OnceLock<LanguageRegistry> = OnceLock::new();
    REGISTRY.get_or_init(LanguageRegistry::new)
}

thread_local! {
    static AST_HIGHLIGHTER: RefCell<Highlighter> = RefCell::new(Highlighter::new());
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum HighlightQuality {
    Complete,
    PartialSyntax,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct SemanticSpan {
    range: Range<usize>,
    kind: SemanticTokenKind,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct HighlightedSource {
    quality: HighlightQuality,
    spans: Vec<SemanticSpan>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum PlainReason {
    MissingOrUnknownLanguage,
    SourceTooLarge { bytes: usize, limit: usize },
    ConfigurationUnavailable(String),
    ParserUnavailable(String),
    HighlightFailed(String),
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum SourceHighlight {
    Ast(HighlightedSource),
    Plain(PlainReason),
}

fn highlight_source(language: Option<SourceLanguage>, source: &str) -> SourceHighlight {
    let Some(language) = language else {
        return SourceHighlight::Plain(PlainReason::MissingOrUnknownLanguage);
    };
    if source.len() > MAX_HIGHLIGHT_BYTES {
        return SourceHighlight::Plain(PlainReason::SourceTooLarge {
            bytes: source.len(),
            limit: MAX_HIGHLIGHT_BYTES,
        });
    }

    let registry = language_registry();
    let Some(profile) = registry.profile(language) else {
        return SourceHighlight::Plain(PlainReason::ConfigurationUnavailable(format!(
            "{} grammar is not registered",
            language.canonical_name()
        )));
    };
    let configuration = match &profile.configuration {
        Ok(configuration) => configuration,
        Err(error) => {
            return SourceHighlight::Plain(PlainReason::ConfigurationUnavailable(error.clone()))
        }
    };

    let quality = match syntax_quality(&profile.parser_language, source) {
        Ok(quality) => quality,
        Err(error) => return SourceHighlight::Plain(PlainReason::ParserUnavailable(error)),
    };

    let result = AST_HIGHLIGHTER.with(|slot| {
        let mut highlighter = slot.borrow_mut();
        let events = highlighter
            .highlight(configuration, source.as_bytes(), None, |name| {
                registry.injection_configuration(name)
            })
            .map_err(|error| error.to_string())?;
        collect_semantic_spans(events, source)
    });

    match result {
        Ok(spans) => SourceHighlight::Ast(HighlightedSource { quality, spans }),
        Err(error) => SourceHighlight::Plain(PlainReason::HighlightFailed(error)),
    }
}

fn syntax_quality(language: &Language, source: &str) -> Result<HighlightQuality, String> {
    let mut parser = Parser::new();
    parser
        .set_language(language)
        .map_err(|error| error.to_string())?;
    let tree = parser
        .parse(source, None)
        .ok_or_else(|| "Tree-sitter returned no syntax tree".to_owned())?;
    if tree.root_node().has_error() {
        Ok(HighlightQuality::PartialSyntax)
    } else {
        Ok(HighlightQuality::Complete)
    }
}

fn collect_semantic_spans<I>(events: I, source: &str) -> Result<Vec<SemanticSpan>, String>
where
    I: IntoIterator<Item = Result<HighlightEvent, tree_sitter_highlight::Error>>,
{
    let mut spans = Vec::new();
    let mut stack = Vec::new();
    let mut cursor = 0;

    for event in events {
        match event.map_err(|error| error.to_string())? {
            HighlightEvent::HighlightStart(Highlight(index)) => {
                let kind = HIGHLIGHT_CLASSES
                    .get(index)
                    .map_or(SemanticTokenKind::Default, |class| class.kind);
                stack.push(kind);
            }
            HighlightEvent::HighlightEnd => {
                stack
                    .pop()
                    .ok_or_else(|| "unbalanced Tree-sitter highlight end event".to_owned())?;
            }
            HighlightEvent::Source { start, end } => {
                if start < cursor
                    || end < start
                    || end > source.len()
                    || !source.is_char_boundary(start)
                    || !source.is_char_boundary(end)
                {
                    return Err(format!(
                        "invalid Tree-sitter source range {start}..{end} at cursor {cursor}"
                    ));
                }
                if start > cursor {
                    push_span(&mut spans, cursor..start, SemanticTokenKind::Default);
                }
                push_span(
                    &mut spans,
                    start..end,
                    stack.last().copied().unwrap_or(SemanticTokenKind::Default),
                );
                cursor = end;
            }
        }
    }

    if !stack.is_empty() {
        return Err("unbalanced Tree-sitter highlight start event".to_owned());
    }
    if cursor < source.len() {
        push_span(&mut spans, cursor..source.len(), SemanticTokenKind::Default);
    }
    Ok(spans)
}

fn push_span(spans: &mut Vec<SemanticSpan>, range: Range<usize>, kind: SemanticTokenKind) {
    if range.is_empty() {
        return;
    }
    if let Some(previous) = spans.last_mut() {
        if previous.kind == kind && previous.range.end == range.start {
            previous.range.end = range.end;
            return;
        }
    }
    spans.push(SemanticSpan { range, kind });
}

#[derive(Clone, Copy)]
struct TerminalStyle {
    foreground: Color,
    background: Color,
    bold: bool,
    italic: bool,
}

struct SourceTheme {
    background: Color,
    default_foreground: Color,
}

impl SourceTheme {
    fn from_skin(skin: &MadSkin) -> Self {
        Self {
            background: skin
                .code_block
                .compound_style
                .get_bg()
                .unwrap_or(Color::Rgb {
                    r: 30,
                    g: 30,
                    b: 40,
                }),
            default_foreground: skin
                .code_block
                .compound_style
                .get_fg()
                .unwrap_or(Color::Rgb {
                    r: 220,
                    g: 220,
                    b: 220,
                }),
        }
    }

    fn style(&self, kind: SemanticTokenKind) -> TerminalStyle {
        let (foreground, bold, italic) = match kind {
            SemanticTokenKind::Default | SemanticTokenKind::Embedded => {
                (self.default_foreground, false, false)
            }
            SemanticTokenKind::Attribute => (rgb(255, 203, 107), false, false),
            SemanticTokenKind::Comment => (rgb(112, 126, 140), false, true),
            SemanticTokenKind::Constant => (rgb(247, 140, 108), false, false),
            SemanticTokenKind::Constructor | SemanticTokenKind::Type => {
                (rgb(255, 203, 107), false, false)
            }
            SemanticTokenKind::Escape => (rgb(255, 125, 171), false, false),
            SemanticTokenKind::Function => (rgb(130, 170, 255), false, false),
            SemanticTokenKind::Keyword => (rgb(199, 146, 234), true, false),
            SemanticTokenKind::Label => (rgb(86, 182, 194), false, false),
            SemanticTokenKind::Number => (rgb(247, 140, 108), false, false),
            SemanticTokenKind::Operator => (rgb(199, 146, 234), false, false),
            SemanticTokenKind::Property => (rgb(86, 182, 194), false, false),
            SemanticTokenKind::Punctuation => (rgb(156, 170, 184), false, false),
            SemanticTokenKind::String => (rgb(195, 232, 141), false, false),
            SemanticTokenKind::Tag => (rgb(255, 97, 136), false, false),
            SemanticTokenKind::Variable => (rgb(224, 230, 237), false, false),
        };
        TerminalStyle {
            foreground,
            background: self.background,
            bold,
            italic,
        }
    }
}

const fn rgb(r: u8, g: u8, b: u8) -> Color {
    Color::Rgb { r, g, b }
}

#[derive(Clone, Copy)]
struct Fence<'a> {
    marker: u8,
    length: usize,
    info: &'a str,
}

enum MarkdownChunk<'a> {
    Markdown(&'a str),
    Source { info: &'a str, source: &'a str },
}

/// Collapse repeated blank lines in prose without changing fenced source.
pub(crate) fn compact_markdown(source: &str) -> String {
    let mut output = String::with_capacity(source.len());
    let mut offset = 0;
    let mut active_fence: Option<(u8, usize)> = None;
    let mut previous_prose_line_blank = true;

    while offset < source.len() {
        let (line, next) = line_at(source, offset);
        let content = line_content(line);
        if let Some((marker, length)) = active_fence {
            output.push_str(line);
            if is_closing_fence(content, marker, length) {
                active_fence = None;
                previous_prose_line_blank = false;
            }
        } else if let Some(fence) = opening_fence(content) {
            output.push_str(line);
            active_fence = Some((fence.marker, fence.length));
            previous_prose_line_blank = false;
        } else if content.trim().is_empty() {
            if !previous_prose_line_blank {
                output.push('\n');
                previous_prose_line_blank = true;
            }
        } else {
            output.push_str(content);
            output.push('\n');
            previous_prose_line_blank = false;
        }
        offset = next;
    }

    if active_fence.is_none() {
        while output.ends_with("\n\n") {
            output.pop();
        }
    }
    output
}

/// Render Markdown while replacing supported fenced blocks with AST styling.
pub(crate) fn write_markdown<W: Write>(
    writer: &mut W,
    skin: &MadSkin,
    markdown: &str,
) -> io::Result<()> {
    let theme = SourceTheme::from_skin(skin);
    for chunk in markdown_chunks(markdown) {
        match chunk {
            MarkdownChunk::Markdown(text) => {
                skin.write_text_on(writer, text).map_err(io::Error::other)?
            }
            MarkdownChunk::Source { info, source } => {
                write_source(
                    writer,
                    &theme,
                    SourceLanguage::from_fence_info(info),
                    source,
                )?;
            }
        }
    }
    writer.flush()
}

fn write_source<W: Write>(
    writer: &mut W,
    theme: &SourceTheme,
    language: Option<SourceLanguage>,
    source: &str,
) -> io::Result<()> {
    match highlight_source(language, source) {
        SourceHighlight::Ast(highlighted) => {
            for span in highlighted.spans {
                write_styled_fragment(writer, theme.style(span.kind), &source[span.range])?;
            }
        }
        SourceHighlight::Plain(_) => {
            write_styled_fragment(writer, theme.style(SemanticTokenKind::Default), source)?;
        }
    }
    writer.queue(SetAttribute(Attribute::Reset))?;
    writer.queue(SetForegroundColor(Color::Reset))?;
    writer.queue(SetBackgroundColor(Color::Reset))?;
    Ok(())
}

fn write_styled_fragment<W: Write>(
    writer: &mut W,
    style: TerminalStyle,
    fragment: &str,
) -> io::Result<()> {
    for part in fragment.split_inclusive('\n') {
        let (text, has_newline) = part
            .strip_suffix('\n')
            .map_or((part, false), |text| (text, true));
        apply_style(writer, style)?;
        write_terminal_safe(writer, text)?;
        if has_newline {
            reset_style(writer)?;
            writer.write_all(b"\n")?;
        }
    }
    Ok(())
}

fn apply_style<W: Write>(writer: &mut W, style: TerminalStyle) -> io::Result<()> {
    writer.queue(SetAttribute(Attribute::Reset))?;
    writer.queue(SetForegroundColor(style.foreground))?;
    writer.queue(SetBackgroundColor(style.background))?;
    if style.bold {
        writer.queue(SetAttribute(Attribute::Bold))?;
    }
    if style.italic {
        writer.queue(SetAttribute(Attribute::Italic))?;
    }
    Ok(())
}

fn reset_style<W: Write>(writer: &mut W) -> io::Result<()> {
    writer.queue(SetAttribute(Attribute::Reset))?;
    writer.queue(SetForegroundColor(Color::Reset))?;
    writer.queue(SetBackgroundColor(Color::Reset))?;
    Ok(())
}

fn write_terminal_safe<W: Write>(writer: &mut W, source: &str) -> io::Result<()> {
    for character in source.chars() {
        match character {
            '\t' => writer.write_all(b"\t")?,
            '\r' => writer.write_all("␍".as_bytes())?,
            '\u{1b}' => writer.write_all("␛".as_bytes())?,
            value if value.is_control() => write!(writer, "\\u{{{:x}}}", value as u32)?,
            value => write!(writer, "{value}")?,
        }
    }
    Ok(())
}

fn markdown_chunks(source: &str) -> Vec<MarkdownChunk<'_>> {
    let mut chunks = Vec::new();
    let mut prose_start = 0;
    let mut offset = 0;

    while offset < source.len() {
        let (line, next) = line_at(source, offset);
        let Some(fence) = opening_fence(line_content(line)) else {
            offset = next;
            continue;
        };

        if prose_start < offset {
            chunks.push(MarkdownChunk::Markdown(&source[prose_start..offset]));
        }
        let code_start = next;
        let mut scan = next;
        let mut code_end = source.len();
        let mut after_fence = source.len();
        while scan < source.len() {
            let (candidate, candidate_next) = line_at(source, scan);
            if is_closing_fence(line_content(candidate), fence.marker, fence.length) {
                code_end = scan;
                after_fence = candidate_next;
                break;
            }
            scan = candidate_next;
        }
        chunks.push(MarkdownChunk::Source {
            info: fence.info,
            source: &source[code_start..code_end],
        });
        offset = after_fence;
        prose_start = after_fence;
    }

    if prose_start < source.len() {
        chunks.push(MarkdownChunk::Markdown(&source[prose_start..]));
    }
    chunks
}

fn opening_fence(line: &str) -> Option<Fence<'_>> {
    let bytes = line.as_bytes();
    let indentation = bytes.iter().take_while(|byte| **byte == b' ').count();
    if indentation > 3 || indentation == bytes.len() {
        return None;
    }
    let marker = bytes[indentation];
    if marker != b'`' && marker != b'~' {
        return None;
    }
    let length = bytes[indentation..]
        .iter()
        .take_while(|byte| **byte == marker)
        .count();
    if length < 3 {
        return None;
    }
    let info = line[indentation + length..].trim();
    if marker == b'`' && info.contains('`') {
        return None;
    }
    Some(Fence {
        marker,
        length,
        info,
    })
}

fn is_closing_fence(line: &str, marker: u8, minimum_length: usize) -> bool {
    let bytes = line.as_bytes();
    let indentation = bytes.iter().take_while(|byte| **byte == b' ').count();
    if indentation > 3 || indentation == bytes.len() || bytes[indentation] != marker {
        return false;
    }
    let length = bytes[indentation..]
        .iter()
        .take_while(|byte| **byte == marker)
        .count();
    length >= minimum_length && line[indentation + length..].trim().is_empty()
}

fn line_at(source: &str, offset: usize) -> (&str, usize) {
    match source[offset..].find('\n') {
        Some(relative) => {
            let next = offset + relative + 1;
            (&source[offset..next], next)
        }
        None => (&source[offset..], source.len()),
    }
}

fn line_content(line: &str) -> &str {
    line.strip_suffix('\n')
        .unwrap_or(line)
        .strip_suffix('\r')
        .unwrap_or_else(|| line.strip_suffix('\n').unwrap_or(line))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ast(language: SourceLanguage, source: &str) -> HighlightedSource {
        match highlight_source(Some(language), source) {
            SourceHighlight::Ast(highlighted) => highlighted,
            SourceHighlight::Plain(reason) => panic!("expected AST highlighting, got {reason:?}"),
        }
    }

    fn source_for_spans<'a>(source: &'a str, spans: &[SemanticSpan]) -> String {
        spans
            .iter()
            .map(|span| &source[span.range.clone()])
            .collect()
    }

    fn has_non_default_span(highlighted: &HighlightedSource) -> bool {
        highlighted
            .spans
            .iter()
            .any(|span| span.kind != SemanticTokenKind::Default)
    }

    #[test]
    fn fence_aliases_resolve_to_explicit_languages() {
        let cases = [
            ("sh", SourceLanguage::Bash),
            ("css", SourceLanguage::Css),
            ("golang", SourceLanguage::Go),
            ("html", SourceLanguage::Html),
            ("java", SourceLanguage::Java),
            ("node", SourceLanguage::JavaScript),
            ("json", SourceLanguage::Json),
            ("py", SourceLanguage::Python),
            ("rs", SourceLanguage::Rust),
            ("swift", SourceLanguage::Swift),
            ("toml", SourceLanguage::Toml),
            ("language-typescript", SourceLanguage::TypeScript),
            ("{.tsx}", SourceLanguage::Tsx),
            ("yml", SourceLanguage::Yaml),
        ];
        for (alias, expected) in cases {
            assert_eq!(SourceLanguage::from_fence_info(alias), Some(expected));
        }
        assert_eq!(SourceLanguage::from_fence_info("unknown"), None);
        assert_eq!(SourceLanguage::from_fence_info("jsonc"), None);
        assert_eq!(SourceLanguage::from_fence_info("console"), None);
    }

    #[test]
    fn every_registered_language_produces_ast_semantic_spans() {
        let cases = [
            (SourceLanguage::Bash, "echo \"hello\"\n"),
            (SourceLanguage::Css, ".card { color: red; }\n"),
            (SourceLanguage::Go, "package main\nfunc main() {}\n"),
            (SourceLanguage::Html, "<main class=\"app\">hello</main>\n"),
            (SourceLanguage::Java, "class App { void run() {} }\n"),
            (SourceLanguage::JavaScript, "const answer = () => 42;\n"),
            (SourceLanguage::Json, "{\"ready\": true, \"count\": 2}\n"),
            (
                SourceLanguage::Python,
                "def greet(name):\n    return f\"hi {name}\"\n",
            ),
            (SourceLanguage::Rust, "fn main() { let value: u32 = 7; }\n"),
            (
                SourceLanguage::Swift,
                "func greet(name: String) -> String { name }\n",
            ),
            (SourceLanguage::Toml, "name = \"easynet\"\n"),
            (SourceLanguage::TypeScript, "const value: number = 42;\n"),
            (
                SourceLanguage::Tsx,
                "const view = <main className=\"app\">Hi</main>;\n",
            ),
            (SourceLanguage::Yaml, "name: easynet\nenabled: true\n"),
        ];

        for (language, source) in cases {
            let highlighted = ast(language, source);
            assert_eq!(
                highlighted.quality,
                HighlightQuality::Complete,
                "{language:?}"
            );
            assert!(has_non_default_span(&highlighted), "{language:?}");
            assert_eq!(source_for_spans(source, &highlighted.spans), source);
        }
    }

    #[test]
    fn ast_captures_project_to_distinct_semantic_kinds() {
        let source = "fn greet(name: &str) -> String { format!(\"hello {name}\") }";
        let highlighted = ast(SourceLanguage::Rust, source);
        let kinds = highlighted
            .spans
            .iter()
            .map(|span| span.kind)
            .collect::<Vec<_>>();
        assert!(kinds.contains(&SemanticTokenKind::Keyword));
        assert!(kinds.contains(&SemanticTokenKind::Function));
        assert!(kinds.contains(&SemanticTokenKind::Type));
        assert!(kinds.contains(&SemanticTokenKind::String));
    }

    #[test]
    fn html_injections_use_the_registered_javascript_ast() {
        let source = "<script>const answer = () => 42;</script>\n";
        let highlighted = ast(SourceLanguage::Html, source);
        let keyword_text = highlighted
            .spans
            .iter()
            .filter(|span| span.kind == SemanticTokenKind::Keyword)
            .map(|span| &source[span.range.clone()])
            .collect::<String>();
        assert!(keyword_text.contains("const"));
    }

    #[test]
    fn unicode_ranges_are_valid_and_reconstruct_the_source() {
        let source = "fn 问候() { let 消息 = \"你好，EasyNet\"; }\n";
        let highlighted = ast(SourceLanguage::Rust, source);
        for span in &highlighted.spans {
            assert!(source.is_char_boundary(span.range.start));
            assert!(source.is_char_boundary(span.range.end));
        }
        assert_eq!(source_for_spans(source, &highlighted.spans), source);
    }

    #[test]
    fn malformed_source_reports_partial_ast_quality_without_losing_text() {
        let source = "fn broken( { let value = \"still highlighted\";\n";
        let highlighted = ast(SourceLanguage::Rust, source);
        assert_eq!(highlighted.quality, HighlightQuality::PartialSyntax);
        assert!(has_non_default_span(&highlighted));
        assert_eq!(source_for_spans(source, &highlighted.spans), source);
    }

    #[test]
    fn absent_language_and_oversized_source_have_explicit_plain_reasons() {
        assert_eq!(
            highlight_source(None, "let value = 1;"),
            SourceHighlight::Plain(PlainReason::MissingOrUnknownLanguage)
        );
        let oversized = "x".repeat(MAX_HIGHLIGHT_BYTES + 1);
        assert_eq!(
            highlight_source(Some(SourceLanguage::Rust), &oversized),
            SourceHighlight::Plain(PlainReason::SourceTooLarge {
                bytes: oversized.len(),
                limit: MAX_HIGHLIGHT_BYTES,
            })
        );
    }

    #[test]
    fn compact_markdown_preserves_blank_lines_inside_source_fences() {
        let markdown =
            "\n\nBefore\n\n\n```rust\nfn first() {}\n\n\nfn second() {}\n```\n\n\nAfter\n\n";
        let compact = compact_markdown(markdown);
        assert_eq!(
            compact,
            "Before\n\n```rust\nfn first() {}\n\n\nfn second() {}\n```\n\nAfter\n"
        );
    }

    #[test]
    fn markdown_chunking_supports_tildes_long_fences_and_unclosed_blocks() {
        let markdown = "intro\n~~~~python\nprint(\"```\")\n~~~~\noutro\n```rust\nfn open() {}\n";
        let chunks = markdown_chunks(markdown);
        assert_eq!(chunks.len(), 4);
        match &chunks[1] {
            MarkdownChunk::Source { info, source } => {
                assert_eq!(*info, "python");
                assert_eq!(*source, "print(\"```\")\n");
            }
            MarkdownChunk::Markdown(_) => panic!("expected source chunk"),
        }
        match &chunks[3] {
            MarkdownChunk::Source { info, source } => {
                assert_eq!(*info, "rust");
                assert_eq!(*source, "fn open() {}\n");
            }
            MarkdownChunk::Markdown(_) => panic!("expected unclosed source chunk"),
        }
    }

    #[test]
    fn terminal_rendering_sanitizes_source_escape_sequences() {
        let skin = MadSkin::default();
        let mut output = Vec::new();
        write_markdown(
            &mut output,
            &skin,
            "```bash\nprintf '\u{1b}[31munsafe'\n```\n",
        )
        .expect("render markdown");
        let rendered = String::from_utf8(output).expect("UTF-8 terminal output");
        assert!(rendered.contains("␛[31munsafe"));
        assert!(!rendered.contains("'\u{1b}[31munsafe'"));
    }

    #[test]
    fn semantic_theme_assigns_different_styles_to_language_roles() {
        let theme = SourceTheme::from_skin(&MadSkin::default());
        let keyword = theme.style(SemanticTokenKind::Keyword);
        let string = theme.style(SemanticTokenKind::String);
        let comment = theme.style(SemanticTokenKind::Comment);
        assert_ne!(keyword.foreground, string.foreground);
        assert_ne!(string.foreground, comment.foreground);
        assert!(keyword.bold);
        assert!(comment.italic);
    }
}
