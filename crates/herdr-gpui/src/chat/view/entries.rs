//! What one transcript entry shows: a message, a thought, a tool call, a
//! picture, a notice, or how a turn ended. The chat's layout decides the
//! row it sits in.

use super::{
    super::{
        chats::ChatPane,
        prepared::{Body, Part},
        rich::{Block, Inline},
        visual::Visuals,
    },
    Hues,
    layout::{Row, layout_for, side},
    visuals::Frame,
};
use crate::{HerdrWindow, config::corners, fonts::StyledFont, notifications::safe_text};
use gpui::{prelude::*, *};
use herdr_chat_protocol::{Entry, EntryKind, NoticeLevel, Tool, ToolContent, ToolKind, ToolStatus};

mod questions;

/// Characters of a folded thought shown.
const FOLDED: usize = 120;
fn tool_kind(kind: ToolKind) -> &'static str {
    match kind {
        ToolKind::Read => "Read",
        ToolKind::Edit => "Edit",
        ToolKind::Delete => "Delete",
        ToolKind::Move => "Move",
        ToolKind::Search => "Search",
        ToolKind::Execute => "Run",
        ToolKind::Think => "Think",
        ToolKind::Fetch => "Fetch",
        ToolKind::SwitchMode => "Mode",
        ToolKind::Other => "Tool",
    }
}

fn tool_label(title: &str) -> &str {
    match title {
        "request_user_input" | "request_user_input_async" => "Ask user",
        _ => title,
    }
}

/// An overlay copy control may cover the last block or an inline control.
/// Plain prose keeps the layout's compact, floating copy treatment.
fn copy_needs_footer(body: Option<&Body>) -> bool {
    let Some(Body::Message(parts)) = body else {
        return false;
    };
    parts.iter().any(|part| match part {
        Part::Text(_) => false,
        Part::Rich(blocks) => blocks.iter().any(|block| match block {
            Block::Paragraph { content, .. } => content.iter().any(|inline| match inline {
                Inline::Math { .. } => true,
                Inline::Text(_, style) => style.code || style.link.is_some(),
                Inline::Break => false,
            }),
            _ => true,
        }),
        _ => true,
    })
}

impl HerdrWindow {
    pub(super) fn render_chat_entry(
        &self,
        pane_id: &str,
        index: usize,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(chat) = self.chats.pane(pane_id) else {
            return div().into_any_element();
        };
        let entries = &chat.session.transcript().entries;
        let Some(entry) = entries.get(index) else {
            return div().into_any_element();
        };
        let body = chat.prepared.body(entry.id);
        let content = match &entry.kind {
            EntryKind::User { text } | EntryKind::Agent { text } => {
                let message = match body.as_deref() {
                    Some(Body::Message(parts)) => self.render_chat_message(
                        pane_id,
                        entry.id,
                        parts,
                        &chat.prepared.visuals,
                        cx,
                    ),
                    Some(Body::Summary(summary)) => {
                        // Copy takes the whole summary, as its cut note says.
                        let card = div()
                            .group("chat-agent-message")
                            .child(self.render_chat_summary(
                                pane_id,
                                entry,
                                summary,
                                chat.expanded.contains(&entry.id),
                                cx,
                            ))
                            .child(self.render_chat_copy(entry.id.0, text, true, cx));
                        return self.render_chat_entry_row(chat, index, card);
                    }
                    _ => self.render_chat_plain_text(
                        self.chat_text_scope(pane_id, entry.id, 0),
                        (0, 0, 0),
                        text.clone(),
                        cx,
                    ),
                };
                div()
                    .group("chat-agent-message")
                    .child(message)
                    .child(self.render_chat_copy(
                        entry.id.0,
                        text,
                        self.config.chat.layout == crate::config::ChatLayoutMode::IMessage
                            && copy_needs_footer(body.as_deref()),
                        cx,
                    ))
            }
            EntryKind::Turn(summary) => {
                let text = self.render_chat_plain_text(
                    self.chat_text_scope(pane_id, entry.id, 0),
                    (0, 0, 0),
                    summary.describe(),
                    cx,
                );
                layout_for(self.config.chat.layout).turn(text.into_any_element(), &self.chat_look())
            }
            EntryKind::Thought { text } => self.render_chat_thought(
                pane_id,
                entry,
                text,
                chat.expanded.contains(&entry.id),
                cx,
            ),
            EntryKind::Image { author, .. } => match body.as_deref() {
                Some(Body::Image(id)) => {
                    self.render_chat_image(*author, *id, &chat.prepared.visuals)
                }
                _ => div(),
            },
            EntryKind::Tool(tool) => self.render_chat_tool(
                pane_id,
                entry.id,
                tool,
                body.as_deref(),
                &chat.prepared.visuals,
                cx,
            ),
            EntryKind::Notice { .. } => self.render_chat_notice(pane_id, entry, cx),
        };
        self.render_chat_entry_row(chat, index, content)
    }

    /// An entry's row in the list, as the chat's layout draws it.
    fn render_chat_entry_row(&self, chat: &ChatPane, index: usize, content: Div) -> AnyElement {
        let entries = &chat.session.transcript().entries;
        let Some(entry) = entries.get(index) else {
            return div().into_any_element();
        };
        let near = |index: Option<usize>| {
            index
                .and_then(|index| entries.get(index))
                .and_then(|entry| side(&entry.kind))
        };
        let transcript = chat.session.transcript();
        let row = Row {
            entry,
            previous: near(index.checked_sub(1)),
            next: near(index.checked_add(1)),
            agent: chat.session.agent(),
            live: index + 1 == entries.len()
                && transcript.status == herdr_chat_protocol::Status::Working,
            content,
        };
        layout_for(self.config.chat.layout).row(row, &self.chat_look())
    }

    /// A copy button by a message, positioned by the selected layout.
    fn render_chat_copy(&self, id: u64, text: &str, footer: bool, cx: &mut Context<Self>) -> Div {
        let theme = &self.theme;
        let text = text.to_owned();
        let copy = layout_for(self.config.chat.layout).copy(
            div()
                .id(("chat-copy", id))
                .debug_selector(move || format!("chat-copy-{id}"))
                .invisible()
                .group_hover("chat-agent-message", |button| button.visible())
                .flex()
                .items_center()
                .gap(px(4.))
                .px(px(4.))
                .rounded(px(corners::SMALL))
                .cursor_pointer()
                .text_size(px((self.chat_text_size() - 2.).max(10.)))
                .text_color(rgb(theme.muted))
                .hover(|button| button.bg(rgb(theme.active)))
                .child(
                    svg()
                        .path("icons/copy.svg")
                        .size(px(11.))
                        .text_color(rgb(theme.muted)),
                )
                .child("Copy")
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    cx.stop_propagation();
                    cx.write_to_clipboard(ClipboardItem::new_string(text.clone()));
                    this.show_flash(crate::window::Flash::success("Copied the message"), cx);
                })),
            &self.chat_look(),
        );
        copy.when(
            footer && self.config.chat.layout == crate::config::ChatLayoutMode::IMessage,
            |copy| {
                // Keep the native pill, but let its actual height reserve a
                // footer instead of covering the last code/table/visual row.
                copy.debug_selector(move || format!("chat-copy-footer-{id}"))
                    .relative()
                    .flex()
                    .justify_end()
                    .pt(px(4.))
            },
        )
    }

    fn render_chat_thought(
        &self,
        pane_id: &str,
        entry: &Entry,
        text: &str,
        expanded: bool,
        cx: &mut Context<Self>,
    ) -> Div {
        let theme = &self.theme;
        let shown = if expanded {
            text.to_owned()
        } else {
            let line = text
                .lines()
                .find(|line| !line.trim().is_empty())
                .unwrap_or("");
            safe_text(line, FOLDED)
        };
        let pane_id = pane_id.to_owned();
        let text_pane = pane_id.clone();
        let id = entry.id;
        div().child(
            div()
                .id(("chat-thought", id.0))
                .debug_selector(move || format!("chat-thought-{}", id.0))
                .flex()
                .gap(px(6.))
                .cursor_pointer()
                .text_color(rgb(theme.dimmed(theme.foreground)))
                .italic()
                .child(
                    svg()
                        .path(if expanded {
                            "icons/chevron-down.svg"
                        } else {
                            "icons/chevron-right.svg"
                        })
                        .flex_none()
                        .mt(px(3.))
                        .size(px(10.))
                        .text_color(rgb(theme.muted)),
                )
                .child(
                    div()
                        .id(("chat-thought-text", id.0))
                        .min_w_0()
                        .when(!expanded, |row| row.truncate())
                        .when(expanded, |row| row.overflow_x_scroll())
                        .child(self.render_chat_plain_text(
                            self.chat_text_scope(&pane_id, id, 0),
                            (0, 0, 0),
                            shown,
                            cx,
                        )),
                )
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_mouse_up(
                    MouseButton::Left,
                    cx.listener(move |this, event: &MouseUpEvent, _, cx| {
                        if !expanded {
                            this.click_chat_card_text(&text_pane, id, event.position, cx);
                        }
                    }),
                )
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    cx.stop_propagation();
                    if let Some(pane) = this.chats.pane_mut(&pane_id) {
                        if !pane.expanded.remove(&id) {
                            pane.expanded.insert(id);
                        }
                        pane.refresh_text();
                        pane.remeasure(id);
                    }
                    cx.notify();
                })),
        )
    }

    fn render_chat_tool(
        &self,
        pane_id: &str,
        entry: herdr_chat_protocol::EntryId,
        tool: &Tool,
        body: Option<&Body>,
        visuals: &Visuals,
        cx: &mut Context<Self>,
    ) -> Div {
        let theme = &self.theme;
        let hues = Hues::of(theme);
        let (status, color) = match tool.status {
            ToolStatus::Pending => ("pending", theme.muted),
            ToolStatus::InProgress => ("running", hues.yellow),
            ToolStatus::Completed => ("done", hues.green),
            ToolStatus::Failed => ("failed", hues.red),
        };
        let small = px((self.chat_text_size() - 1.).max(10.));
        let summary = match body {
            Some(Body::Tool(summary)) => Some(summary),
            _ => None,
        };
        let expanded = self
            .chats
            .pane(pane_id)
            .is_some_and(|chat| chat.expanded.contains(&entry));
        let toggle_pane = pane_id.to_owned();
        let text_pane = pane_id.to_owned();
        // The gutter's dot carries the state's color; the row stays plain.
        div()
            .flex()
            .flex_col()
            .gap(px(2.))
            .child(
                div()
                    .id(("chat-tool", entry.0))
                    .debug_selector(move || format!("chat-tool-{}", entry.0))
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .cursor_pointer()
                    .child(
                        svg()
                            .path(if expanded {
                                "icons/chevron-down.svg"
                            } else {
                                "icons/chevron-right.svg"
                            })
                            .flex_none()
                            .size(px(10.))
                            .text_color(rgb(theme.muted)),
                    )
                    .child(
                        div()
                            .flex_none()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(hues.blue))
                            .child(tool_kind(tool.kind)),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_font(&self.config.terminal)
                            .text_size(px(self.config.terminal.size - 1.))
                            .text_color(rgb(theme.subtext()))
                            .child(self.render_chat_plain_text(
                                self.chat_text_scope(
                                    pane_id,
                                    entry,
                                    crate::chat::selection::LABEL_PART,
                                ),
                                (0, 0, 0),
                                safe_text(tool_label(&tool.title), 300),
                                cx,
                            )),
                    )
                    .child(
                        div()
                            .flex_none()
                            .text_size(small)
                            .text_color(rgb(color))
                            .child(status),
                    )
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_mouse_up(
                        MouseButton::Left,
                        cx.listener(move |this, event: &MouseUpEvent, _, cx| {
                            this.click_chat_card_text(&text_pane, entry, event.position, cx);
                        }),
                    )
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        cx.stop_propagation();
                        if let Some(pane) = this.chats.pane_mut(&toggle_pane) {
                            if !pane.expanded.remove(&entry) {
                                pane.expanded.insert(entry);
                            }
                            pane.refresh_text();
                            pane.remeasure(entry);
                        }
                        cx.notify();
                    })),
            )
            .children(
                page_written(tool).map(|path| self.render_chat_file_page(pane_id, entry, path, cx)),
            )
            .when(expanded, |card| {
                card.child(
                    div()
                        .id(("chat-tool-details", entry.0))
                        .debug_selector(move || format!("chat-tool-details-{}", entry.0))
                        .min_w_0()
                        .w_full()
                        .overflow_x_scroll()
                        .text_size(small)
                        .text_font(&self.config.terminal)
                        .text_color(rgb(theme.subtext()))
                        .child(self.render_chat_plain_text(
                            self.chat_text_scope(pane_id, entry, 0),
                            (0, 0, 0),
                            tool.title.clone(),
                            cx,
                        ))
                        .children(tool.locations.iter().enumerate().map(|(index, location)| {
                            self.render_chat_plain_text(
                                self.chat_text_scope(pane_id, entry, 1),
                                (0, index, 0),
                                location.clone(),
                                cx,
                            )
                        }))
                        .children(tool.content.iter().enumerate().filter_map(|(index, item)| {
                            tool_text(item).map(|text| {
                                // Keep the full retained output; long unbroken lines
                                // scroll horizontally instead of disappearing.
                                div()
                                    .id(SharedString::from(format!(
                                        "chat-tool-content-{}-{index}",
                                        entry.0
                                    )))
                                    .debug_selector(move || {
                                        format!("chat-tool-content-{}-{index}", entry.0)
                                    })
                                    .w_full()
                                    .overflow_x_scroll()
                                    .py(px(3.))
                                    .child(self.render_chat_plain_text(
                                        self.chat_text_scope(pane_id, entry, 2),
                                        (index, 0, 0),
                                        text,
                                        cx,
                                    ))
                            })
                        }))
                        .children(summary.into_iter().flat_map(|summary| {
                            summary
                                .images
                                .iter()
                                .map(|id| self.render_chat_visual(*id, visuals, Frame::ToolImage))
                        })),
                )
            })
    }

    fn render_chat_notice(&self, pane_id: &str, entry: &Entry, cx: &mut Context<Self>) -> Div {
        let EntryKind::Notice {
            answers,
            details,
            level,
            text,
        } = &entry.kind
        else {
            return div();
        };
        if !answers.is_empty() {
            return self.render_chat_answers(pane_id, entry, cx);
        }
        self.render_chat_notice_record(pane_id, entry.id, *level, text, details.as_deref(), cx)
    }

    fn render_chat_notice_record(
        &self,
        pane_id: &str,
        id: herdr_chat_protocol::EntryId,
        level: NoticeLevel,
        text: &str,
        details: Option<&str>,
        cx: &mut Context<Self>,
    ) -> Div {
        let theme = &self.theme;
        let hues = Hues::of(theme);
        let color = match level {
            NoticeLevel::Info => theme.subtext(),
            NoticeLevel::Warning => hues.yellow,
            NoticeLevel::Error => hues.red,
        };
        let Some(details) = details else {
            return div()
                .text_color(rgb(color))
                .child(self.render_chat_plain_text(
                    self.chat_text_scope(pane_id, id, 0),
                    (0, 0, 0),
                    text.to_owned(),
                    cx,
                ));
        };
        let expanded = self
            .chats
            .pane(pane_id)
            .is_some_and(|chat| chat.expanded.contains(&id));
        let pane_id = pane_id.to_owned();
        let toggle_pane = pane_id.clone();
        let text_pane = pane_id.clone();
        div()
            .flex()
            .flex_col()
            .min_w_0()
            .gap(px(4.))
            .text_color(rgb(color))
            .when(
                self.config.chat.layout == crate::config::ChatLayoutMode::IMessage,
                |card| {
                    card.bg(rgb(theme.background))
                        .rounded(px(corners::SMALL))
                        .p(px(4.))
                },
            )
            .child(
                div()
                    .id(("chat-notice", id.0))
                    .debug_selector(move || format!("chat-notice-{}", id.0))
                    .flex()
                    .items_center()
                    .min_w_0()
                    .gap(px(6.))
                    .cursor_pointer()
                    .child(
                        svg()
                            .path(if expanded {
                                "icons/chevron-down.svg"
                            } else {
                                "icons/chevron-right.svg"
                            })
                            .flex_none()
                            .size(px(10.))
                            .text_color(rgb(theme.muted)),
                    )
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .child(self.render_chat_plain_text(
                                self.chat_text_scope(&pane_id, id, 0),
                                (0, 0, 0),
                                safe_text(text, 240),
                                cx,
                            )),
                    )
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_mouse_up(
                        MouseButton::Left,
                        cx.listener(move |this, event: &MouseUpEvent, _, cx| {
                            this.click_chat_card_text(&text_pane, id, event.position, cx);
                        }),
                    )
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        cx.stop_propagation();
                        if let Some(pane) = this.chats.pane_mut(&toggle_pane) {
                            if !pane.expanded.remove(&id) {
                                pane.expanded.insert(id);
                            }
                            pane.refresh_text();
                            pane.remeasure(id);
                        }
                        cx.notify();
                    })),
            )
            .when(expanded, |card| {
                card.child(
                    div()
                        .id(("chat-notice-details", id.0))
                        .debug_selector(move || format!("chat-notice-details-{}", id.0))
                        .group("chat-agent-message")
                        .relative()
                        .min_w_0()
                        .w_full()
                        .child(
                            div()
                                .id(("chat-notice-source", id.0))
                                .debug_selector(move || format!("chat-notice-source-{}", id.0))
                                .w_full()
                                .overflow_x_scroll()
                                .text_font(&self.config.terminal)
                                .text_size(px((self.chat_text_size() - 1.).max(10.)))
                                // Imported content is inert source text, including
                                // XML, commands, URLs, and output-file paths.
                                .child(self.render_chat_plain_text(
                                    self.chat_text_scope(
                                        &pane_id,
                                        id,
                                        crate::chat::selection::DETAILS_PART,
                                    ),
                                    (0, 0, 0),
                                    details.to_owned(),
                                    cx,
                                )),
                        )
                        .child(self.render_chat_copy(id.0, details, true, cx)),
                )
            })
    }
}

/// Expanded tool output comes from the retained transcript, not its preview.
fn tool_text(content: &ToolContent) -> Option<String> {
    match content {
        ToolContent::Text { text } => Some(text.clone()),
        ToolContent::Diff { old, new, path } => Some(match old {
            Some(old) => format!("{path}\nBefore:\n{old}\nAfter:\n{new}"),
            None => format!("{path}\nNew file:\n{new}"),
        }),
        ToolContent::Terminal { id } => Some(format!(
            "Terminal {id}: output is available in the program's terminal view."
        )),
        ToolContent::Image { .. } => None,
    }
}

#[cfg(test)]
mod tests;

/// The HTML page a finished tool call wrote, by an absolute path, which
/// the user may preview.
fn page_written(tool: &Tool) -> Option<std::path::PathBuf> {
    if tool.status != ToolStatus::Completed || tool.kind != ToolKind::Edit {
        return None;
    }
    tool.locations.iter().find_map(|location| {
        // A location may end with the line the call touched.
        let path = match location.rsplit_once(':') {
            Some((path, line)) if line.bytes().all(|byte| byte.is_ascii_digit()) => path,
            _ => location.as_str(),
        };
        let path = std::path::Path::new(path);
        let html = path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| {
                extension.eq_ignore_ascii_case("html") || extension.eq_ignore_ascii_case("htm")
            });
        (html && path.is_absolute()).then(|| path.to_owned())
    })
}
