use super::*;
use crate::chat::selection::{
    MAX_CODE_LINE_CHARS as MAX_LINE_CHARS, MAX_CODE_LINES as MAX_LINES, Segment, TAB_WIDTH,
};
use crate::review::highlight::Span;
use std::sync::Arc;
use unicode_width::UnicodeWidthChar;

impl HerdrWindow {
    pub(super) fn rich_code(
        &self,
        id: VisualId,
        language: &str,
        source: &Arc<str>,
        block: usize,
        context: &mut RichContext<'_, '_, '_>,
    ) -> Div {
        let spans = context.visuals.code(id);
        let lines: Vec<_> = source
            .lines()
            .take(MAX_LINES)
            .enumerate()
            .map(|(index, raw)| {
                let spans = spans
                    .and_then(|spans| spans.get(index))
                    .map_or(&[][..], Vec::as_slice);
                match context.selection.segment(block, index, 0) {
                    Some(segment) => prepared_code_line(raw, spans, segment),
                    None => code_line(raw, spans),
                }
            })
            .collect();
        let truncated =
            source.lines().count() > MAX_LINES || lines.iter().any(|line| line.truncated);
        div()
            .debug_selector(|| "chat-code-block".into())
            .min_w_0()
            .rounded(px(6.))
            .border_1()
            .border_color(rgb(self.theme.active))
            .bg(rgb(self.theme.surface))
            .text_color(rgb(self.theme.foreground))
            .overflow_hidden()
            .child(
                div()
                    .flex()
                    .justify_between()
                    .items_center()
                    .px(px(14.))
                    .py(px(6.))
                    .border_b_1()
                    .border_color(rgb(self.theme.active))
                    .text_size(px(self.chat_text_size() - 2.))
                    .text_color(rgb(self.theme.muted))
                    .child(if language.is_empty() { "Code" } else { language }.to_owned())
                    .child(self.rich_copy("Copy code", Arc::clone(source))),
            )
            .child(
                div()
                    .id("code-scroll")
                    .debug_selector(|| "chat-code-scroll".into())
                    .min_w_0()
                    .overflow_x_scroll()
                    .flex()
                    .flex_col()
                    .items_start()
                    .p(px(14.))
                    .text_size(px(self.config.terminal.size))
                    .line_height(relative(1.5))
                    .text_font(&self.config.terminal)
                    .children(lines.into_iter().enumerate().map(|(index, line)| {
                        let marks = line.spans.into_iter().map(|span| {
                            (span.start..span.end, HighlightStyle {
                                color: Some(rgb(token_color(&self.theme, span.token)).into()),
                                ..Default::default()
                            })
                        });
                        let fragment = context.selection.fragment(block, index, 0);
                        let styled = match &fragment {
                            Some(fragment) => fragment.styled(line.text, marks, &self.theme),
                            None => StyledText::new(line.text).with_highlights(marks),
                        };
                        let layout = styled.layout().clone();
                        let view = div()
                            .debug_selector(move || format!("chat-code-line-{index}"))
                            .flex_none()
                            .whitespace_nowrap()
                            .min_h(px(self.config.terminal.size * 1.5))
                            .child(styled);
                        match fragment {
                            Some(fragment) => fragment.wrap(view, layout, context.cx),
                            None => view,
                        }
                    })),
            )
            .when(truncated, |view| {
                view.child(
                    div()
                        .debug_selector(|| "chat-code-truncated".into())
                        .px(px(14.))
                        .pb(px(6.))
                        .text_color(rgb(self.theme.muted))
                        .child("Preview limited to 512 lines and 4,000 characters per line; Copy code includes the full block."),
                )
            })
    }
}

struct CodeLine {
    text: String,
    spans: Vec<Span>,
    truncated: bool,
}

/// Prepared source offsets are also used for copy: rendering and selecting
/// expanded tabs cannot drift apart, and the mapping is not rebuilt per frame.
fn prepared_code_line(raw: &str, spans: &[Span], segment: &Segment) -> CodeLine {
    let text = segment.text();
    let end = segment.source_offset(text.len());
    let spans = spans
        .iter()
        .filter_map(|span| {
            let start = segment.display_offset(span.start)?;
            let end = segment.display_offset(span.end.min(end))?;
            (start < end).then_some(Span {
                start,
                end,
                token: span.token,
            })
        })
        .collect();
    CodeLine {
        text: text.to_owned(),
        spans,
        truncated: end < raw.len(),
    }
}

/// Tabs are layout, not unsafe controls. Map grammar byte offsets through tab
/// expansion and control removal; the original source remains the copy payload.
fn code_line(raw: &str, spans: &[Span]) -> CodeLine {
    let mut text = String::new();
    let mut offsets = Vec::new();
    let (mut column, mut end) = (0, 0);
    for (at, ch) in raw.char_indices().take(MAX_LINE_CHARS) {
        offsets.push((at, text.len()));
        if ch == '\t' {
            let spaces = TAB_WIDTH - column % TAB_WIDTH;
            text.extend(std::iter::repeat_n(' ', spaces));
            column += spaces;
        } else if !crate::notifications::unsafe_char(ch) {
            text.push(ch);
            column += ch.width().unwrap_or(0);
        }
        end = at + ch.len_utf8();
    }
    offsets.push((end, text.len()));
    let position = |at| {
        offsets
            .binary_search_by_key(&at, |(source, _)| *source)
            .ok()
            .map(|i| offsets[i].1)
    };
    let spans = spans
        .iter()
        .filter_map(|span| {
            let start = position(span.start)?;
            let end = position(span.end.min(end))?;
            (start < end).then_some(Span {
                start,
                end,
                token: span.token,
            })
        })
        .collect();
    CodeLine {
        text,
        spans,
        truncated: end < raw.len(),
    }
}

#[cfg(test)]
mod tests;
