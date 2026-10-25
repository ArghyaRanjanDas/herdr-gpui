use super::{Block, Inline, Style, formula};
use crate::{
    chat::visual::{Source, VisualId, Visuals},
    release_notes::Kind,
};
use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag, TagEnd};
use std::sync::Arc;

/// Every bounded message uses the same Markdown semantics. Ordinary prose is
/// laid out as shaped text runs; only mixed math needs individual inline atoms.
pub(in crate::chat) fn parse(text: &str, visuals: &mut Visuals) -> Option<Vec<Block>> {
    // Inline layout creates native word elements. Oversized rich fragments
    // use the existing bounded source renderer instead of building a huge tree.
    if text.split_whitespace().take(4_097).count() > 4_096 {
        return None;
    }
    let normalized = normalize_math(text);
    let display = super::math_input::protect_display(&normalized);
    let text = display.markdown.as_str();
    let options = Options::ENABLE_TABLES
        | Options::ENABLE_MATH
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS;
    let events: Vec<_> = Parser::new_ext(text, options)
        .into_offset_iter()
        .take(16_385)
        .collect();
    if events.len() > 16_384 {
        return None;
    }
    let mut blocks = Vec::new();
    let mut content = Vec::new();
    let mut kind = Kind::Paragraph;
    let mut heading = 0;
    let mut style = Style::default();
    let (mut bold, mut italic, mut strike) = (0usize, 0usize, 0usize);
    let mut links = Vec::new();
    let mut lists = Vec::new();
    let mut quote = 0usize;
    let mut item = None;
    let mut table = None;
    let mut rows = Vec::new();
    let mut row = Vec::new();
    let mut code: Option<(String, String, bool)> = None;
    let mut image_depth = 0usize;
    let (mut maths, mut codes) = (0, 0);
    for (event, range) in events {
        if image_depth > 0 {
            match event {
                Event::Start(Tag::Image { .. }) => image_depth += 1,
                Event::End(TagEnd::Image) => image_depth -= 1,
                _ => {}
            }
            continue;
        }
        if let Some((language, source, closed)) = &mut code {
            if matches!(event, Event::End(TagEnd::CodeBlock)) {
                let language = std::mem::take(language);
                let source: Arc<str> = std::mem::take(source).into();
                let id = VisualId::of(("code", &language, &source));
                if *closed && source.len() <= super::MAX_CODE_BYTES && codes < 32 {
                    visuals.want(
                        id,
                        Source::Code {
                            language: language.clone(),
                            source: source.clone(),
                        },
                    );
                    codes += 1;
                }
                blocks.push(Block::Code {
                    id,
                    language,
                    source,
                });
                code = None;
            } else if let Event::Text(text) = event {
                source.push_str(&text);
            }
            continue;
        }
        match event {
            // Standalone supported pictures are handled by the message layer.
            // Remaining inline/unsupported images stay exact visible source;
            // rendering must neither discard their path nor load it implicitly.
            Event::Start(Tag::Image { .. }) => {
                push_text(&mut content, &text[range], &style);
                image_depth = 1;
            }
            Event::Start(Tag::CodeBlock(info)) => {
                flush(
                    &mut blocks,
                    &mut content,
                    kind,
                    heading,
                    lists.len().saturating_sub(1),
                );
                let closed = match &info {
                    CodeBlockKind::Indented => true,
                    CodeBlockKind::Fenced(_) => closed_fence(&text[range.clone()]),
                };
                let language = match info {
                    CodeBlockKind::Fenced(info) => info
                        .split_whitespace()
                        .next()
                        .unwrap_or("")
                        .chars()
                        .take(64)
                        .collect::<String>()
                        .to_ascii_lowercase(),
                    CodeBlockKind::Indented => String::new(),
                };
                code = Some((language, String::new(), closed));
            }
            Event::Start(Tag::Paragraph) => {
                kind = item.unwrap_or(if quote > 0 {
                    Kind::Quote
                } else {
                    Kind::Paragraph
                });
            }
            Event::Start(Tag::Heading { level, .. }) => {
                flush(
                    &mut blocks,
                    &mut content,
                    kind,
                    heading,
                    lists.len().saturating_sub(1),
                );
                kind = Kind::Heading;
                heading = level as u8;
            }
            Event::End(TagEnd::Paragraph | TagEnd::Heading(_)) => {
                flush(
                    &mut blocks,
                    &mut content,
                    kind,
                    heading,
                    lists.len().saturating_sub(1),
                );
                item = None;
                kind = Kind::Paragraph;
                heading = 0;
            }
            Event::Start(Tag::BlockQuote(_)) => {
                quote += 1;
                kind = Kind::Quote;
            }
            Event::End(TagEnd::BlockQuote(_)) => {
                flush(
                    &mut blocks,
                    &mut content,
                    kind,
                    heading,
                    lists.len().saturating_sub(1),
                );
                quote = quote.saturating_sub(1);
            }
            Event::Start(Tag::List(start)) => {
                flush(
                    &mut blocks,
                    &mut content,
                    kind,
                    heading,
                    lists.len().saturating_sub(1),
                );
                lists.push(start);
            }
            Event::End(TagEnd::List(_)) => {
                lists.pop();
            }
            Event::Start(Tag::Item) => {
                flush(
                    &mut blocks,
                    &mut content,
                    kind,
                    heading,
                    lists.len().saturating_sub(1),
                );
                kind = match lists.last_mut() {
                    Some(Some(number)) => {
                        let n = *number;
                        *number = number.saturating_add(1);
                        Kind::Numbered(n.min(u32::MAX as u64) as u32)
                    }
                    _ => Kind::Bullet,
                };
                item = Some(kind);
            }
            Event::End(TagEnd::Item) => {
                flush(
                    &mut blocks,
                    &mut content,
                    kind,
                    heading,
                    lists.len().saturating_sub(1),
                );
                item = None;
                kind = Kind::Paragraph;
                heading = 0;
            }
            Event::Start(Tag::Strong) => {
                bold += 1;
                style.bold = true;
            }
            Event::End(TagEnd::Strong) => {
                bold = bold.saturating_sub(1);
                style.bold = bold > 0;
            }
            Event::Start(Tag::Emphasis) => {
                italic += 1;
                style.italic = true;
            }
            Event::End(TagEnd::Emphasis) => {
                italic = italic.saturating_sub(1);
                style.italic = italic > 0;
            }
            Event::Start(Tag::Strikethrough) => {
                strike += 1;
                style.strike = true;
            }
            Event::End(TagEnd::Strikethrough) => {
                strike = strike.saturating_sub(1);
                style.strike = strike > 0;
            }
            Event::Start(Tag::Link { dest_url, .. }) => {
                let previous = style.link.take();
                style.link = crate::browser::WebUrl::try_from(dest_url.as_ref())
                    .ok()
                    .map(|url| url.as_str().to_owned())
                    .or_else(|| {
                        crate::terminal::file_target(&dest_url).map(|_| dest_url.to_string())
                    });
                // Unsupported schemes remain visible, never actionable.
                let literal = style.link.is_none().then(|| dest_url.into_string());
                if literal.is_some() {
                    push_text(&mut content, "[", &style);
                }
                links.push((previous, literal));
            }
            Event::End(TagEnd::Link) => {
                if let Some((previous, literal)) = links.pop() {
                    if let Some(target) = literal {
                        push_text(&mut content, &format!("]({target})"), &style);
                    }
                    style.link = previous;
                }
            }
            Event::Start(Tag::Table(align)) => {
                flush(
                    &mut blocks,
                    &mut content,
                    kind,
                    heading,
                    lists.len().saturating_sub(1),
                );
                table = Some(align);
            }
            Event::Start(Tag::TableHead | Tag::TableRow) => row.clear(),
            Event::End(TagEnd::TableCell) => row.push(std::mem::take(&mut content)),
            Event::End(TagEnd::TableHead | TagEnd::TableRow) => rows.push(std::mem::take(&mut row)),
            Event::End(TagEnd::Table) => {
                if rows.len() > 128 || rows.iter().any(|row| row.len() > 16) {
                    return None;
                }
                if let Some(align) = table.take() {
                    blocks.push(Block::Table {
                        align,
                        rows: std::mem::take(&mut rows),
                    });
                }
            }
            Event::InlineMath(source) => {
                if let Some((id, source)) = formula(&source, false, &mut maths, visuals) {
                    content.push(Inline::Math { id, source });
                } else {
                    push_text(&mut content, &format!("${source}$"), &style);
                }
            }
            Event::DisplayMath(source) => {
                let source = display
                    .sources
                    .get(&range.start)
                    .map_or(source.as_ref(), String::as_str);
                if let Some((id, source)) = formula(source, true, &mut maths, visuals) {
                    if table.is_some() {
                        content.push(Inline::Math { id, source });
                    } else {
                        flush(
                            &mut blocks,
                            &mut content,
                            kind,
                            heading,
                            lists.len().saturating_sub(1),
                        );
                        blocks.push(Block::Math { id, source });
                    }
                } else {
                    push_text(&mut content, &format!("$${source}$$"), &style);
                }
            }
            Event::Text(text) => push_linked_text(&mut content, &text, &style),
            Event::Html(text) | Event::InlineHtml(text) => push_text(&mut content, &text, &style),
            Event::Code(text) => {
                let mut code_style = style.clone();
                code_style.code = true;
                push_text(
                    &mut content,
                    &text.replace('\t', &" ".repeat(super::TAB_WIDTH)),
                    &code_style,
                );
            }
            Event::SoftBreak => push_text(&mut content, " ", &style),
            Event::HardBreak => content.push(Inline::Break),
            Event::TaskListMarker(done) => {
                push_text(&mut content, if done { "☑ " } else { "☐ " }, &style)
            }
            Event::Rule => {
                flush(
                    &mut blocks,
                    &mut content,
                    kind,
                    heading,
                    lists.len().saturating_sub(1),
                );
                blocks.push(Block::Paragraph {
                    kind: Kind::Rule,
                    heading: 0,
                    indent: 0,
                    content: Vec::new(),
                });
            }
            _ => {}
        }
    }
    flush(
        &mut blocks,
        &mut content,
        kind,
        heading,
        lists.len().saturating_sub(1),
    );
    Some(blocks)
}

fn flush(
    blocks: &mut Vec<Block>,
    content: &mut Vec<Inline>,
    kind: Kind,
    heading: u8,
    indent: usize,
) {
    if !content.is_empty() {
        blocks.push(Block::Paragraph {
            kind,
            heading,
            indent,
            content: std::mem::take(content),
        });
    }
}

fn push_text(content: &mut Vec<Inline>, text: &str, style: &Style) {
    // The fragment is already bounded by message(). Keep line breaks in
    // literal image/HTML source; stripping them can concatenate its tokens.
    let text: String = text
        .chars()
        .filter(|ch| *ch == '\n' || !crate::notifications::unsafe_char(*ch))
        .collect();
    if let Some(Inline::Text(previous, previous_style)) = content.last_mut()
        && previous_style == style
    {
        previous.push_str(&text);
    } else {
        content.push(Inline::Text(text, style.clone()));
    }
}

/// Preserve the plain renderer's bare web links without reinterpreting code
/// or explicit link labels. Inspect each whitespace-delimited token once;
/// candidates passed to the shared detector are bounded by its URL limit.
fn push_linked_text(content: &mut Vec<Inline>, text: &str, style: &Style) {
    if style.code || style.link.is_some() {
        push_text(content, text, style);
        return;
    }
    for token in text.split_inclusive(char::is_whitespace) {
        let end = token
            .trim_end_matches(char::is_whitespace)
            .trim_end_matches(['.', ',', ';', ':', '!', '?', ')', '\'', '"', '>'])
            .len();
        let found = token[..end].match_indices("http").find_map(|(start, _)| {
            let candidate = &token[start..end];
            (candidate.len() <= crate::release_notes::MAX_URL)
                .then(|| crate::release_notes::bare_url(candidate))
                .flatten()
                .map(|url| (start, url))
        });
        if let Some((start, url)) = found {
            push_text(content, &token[..start], style);
            let mut linked = style.clone();
            linked.link = Some(url.to_owned());
            push_text(content, url, &linked);
            push_text(content, &token[start + url.len()..], style);
        } else {
            push_text(content, token, style);
        }
    }
}

/// Accept the other common TeX delimiters, without rewriting code or HTML.
/// Only complete pairs are converted, so unfinished streamed formulas remain source.
pub(super) fn normalize_math(text: &str) -> String {
    let protected = super::math_input::protected_ranges(text);
    let mut result = String::with_capacity(text.len());
    let mut cursor = 0;
    let mut ranges = protected.iter().cloned().peekable();
    // Index closers once. Searching each remaining suffix for an unmatched
    // opener is quadratic on adversarial (or unfinished streamed) prose.
    let (mut inline, mut display) = (Vec::new(), Vec::new());
    let mut chars = text.char_indices();
    while let Some((at, ch)) = chars.next() {
        if ch == '\\' {
            let inside = protected.partition_point(|region| region.start <= at);
            let protected_closer = inside
                .checked_sub(1)
                .is_some_and(|index| at < protected[index].end);
            match chars.next() {
                Some((_, ')')) if !protected_closer => inline.push(at),
                Some((_, ']')) if !protected_closer => display.push(at),
                _ => {}
            }
        }
    }
    let mut inline = inline.into_iter().peekable();
    let mut display = display.into_iter().peekable();
    while cursor < text.len() {
        if let Some(range) = ranges.peek()
            && range.start <= cursor
        {
            let end = range.end.max(cursor);
            result.push_str(&text[cursor..end]);
            cursor = end;
            ranges.next();
            continue;
        }
        let rest = &text[cursor..];
        if rest.starts_with("\\\\") {
            result.push_str("\\\\");
            cursor += 2;
            continue;
        }
        let pair = if rest.starts_with("\\(") {
            Some((&mut inline, "$"))
        } else if rest.starts_with("\\[") {
            Some((&mut display, "$$"))
        } else {
            None
        };
        if let Some((closers, marker)) = pair {
            while closers.peek().is_some_and(|at| *at < cursor + 2) {
                closers.next();
            }
            // pulldown-cmark opens inline `$` only before a non-space and closes
            // it only after one, so `\( x \)` must lose its inner padding.
            // Display math keeps its exact source.
            if let Some(&end) = closers.peek()
                && (marker == "$$" || ranges.peek().is_none_or(|range| end < range.start))
                && let body = match marker {
                    "$" => text[cursor + 2..end].trim(),
                    _ => &text[cursor + 2..end],
                }
                && !body.trim().is_empty()
            {
                result.push_str(marker);
                result.push_str(body);
                result.push_str(marker);
                cursor = end + 2;
                continue;
            }
        }
        let Some(ch) = rest.chars().next() else {
            break;
        };
        result.push(ch);
        cursor += ch.len_utf8();
    }
    result
}

fn closed_fence(source: &str) -> bool {
    let mut lines = source.lines();
    let Some(open) = lines.next().map(str::trim_start) else {
        return false;
    };
    let Some(marker) = open.chars().next() else {
        return false;
    };
    let count = open.chars().take_while(|ch| *ch == marker).count();
    count >= 3
        && lines.last().is_some_and(|close| {
            let close = close.trim();
            close.len() >= count && close.chars().all(|ch| ch == marker)
        })
}
