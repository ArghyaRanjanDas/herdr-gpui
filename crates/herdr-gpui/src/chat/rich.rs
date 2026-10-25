//! Structured Markdown kept separately from layout. Parsing is bounded by
//! the message limit; math rasterization and grammar parsing use Visuals' workers.

mod code;
mod math;
mod math_input;
mod parse;

#[cfg(test)]
pub(super) use code::MAX_CODE_LINE_CHARS;
pub(super) use code::{CodeText, MAX_CODE_LINES, TAB_WIDTH};
pub(super) use math::draw as draw_math;
pub(super) use parse::parse;

use super::visual::{Source, VisualId, Visuals};
use crate::release_notes::Kind;
use pulldown_cmark::Alignment;
use std::sync::Arc;

pub(super) const MAX_MATH_BYTES: usize = 4_096;
pub(super) const MAX_MATH: usize = 64;
pub(super) const MAX_CODE_BYTES: usize = 32 * 1024;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Style {
    pub bold: bool,
    pub italic: bool,
    pub code: bool,
    pub strike: bool,
    pub link: Option<String>,
}

#[derive(Clone, Debug)]
pub(crate) enum Inline {
    Text(String, Style),
    Math { id: VisualId, source: Arc<str> },
    Break,
}

#[derive(Debug)]
pub(crate) enum Block {
    Paragraph {
        kind: Kind,
        /// Heading level (1–6), otherwise zero; kept for typographic hierarchy.
        heading: u8,
        /// Nested list depth; bounded in the view so deep input stays readable.
        indent: usize,
        content: Vec<Inline>,
    },
    Math {
        id: VisualId,
        source: Arc<str>,
    },
    Code {
        id: VisualId,
        language: String,
        source: Arc<str>,
    },
    Table {
        align: Vec<Alignment>,
        rows: Vec<Vec<Vec<Inline>>>,
    },
}

impl Block {
    pub(super) fn visuals(&self) -> Vec<VisualId> {
        let inline = |content: &[Inline]| {
            content
                .iter()
                .filter_map(|part| match part {
                    Inline::Math { id, .. } => Some(*id),
                    _ => None,
                })
                .collect::<Vec<_>>()
        };
        match self {
            Self::Code { id, .. } | Self::Math { id, .. } => vec![*id],
            Self::Paragraph { content, .. } => inline(content),
            Self::Table { rows, .. } => rows
                .iter()
                .flatten()
                .flat_map(|cell| inline(cell))
                .collect(),
        }
    }
}

fn formula(
    source: &str,
    display: bool,
    count: &mut usize,
    visuals: &mut Visuals,
) -> Option<(VisualId, Arc<str>)> {
    if source.len() > MAX_MATH_BYTES || source.trim().is_empty() || *count >= MAX_MATH {
        return None;
    }
    *count += 1;
    let source: Arc<str> = source.into();
    let id = VisualId::of(("math", display, &source));
    visuals.want(
        id,
        Source::Math {
            source: source.clone(),
            display,
        },
    );
    Some((id, source))
}

/// Reuse the review's syntax grammar cache and semantic token classes.
pub(super) fn highlight(language: &str, source: &str) -> Vec<Vec<crate::review::highlight::Span>> {
    let extension = match language {
        "rust" => "rs",
        "python" | "py" => "py",
        "javascript" | "js" | "jsx" => "js",
        "typescript" | "ts" | "tsx" => "ts",
        "bash" | "sh" | "shell" | "zsh" => "sh",
        "c++" | "cpp" => "cpp",
        "c#" | "csharp" => "cs",
        "ruby" => "rb",
        "yaml" | "yml" => "yaml",
        "markdown" | "md" => "md",
        other => other,
    };
    crate::review::highlight::colour_file(&format!("chat.{extension}"), source, 512)
}

#[cfg(test)]
mod tests;
