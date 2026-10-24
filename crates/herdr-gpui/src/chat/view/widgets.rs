//! Native controls for a validated `herdr-ui` card. Editing only changes
//! local form state. Actions use the ordinary composer, after a user click.

use super::super::{
    Target,
    prepared::{Body, Part},
    widgets::{ActionMode, Component, Document, Input, Value, bounded_input, slider_value},
};
use crate::{
    HerdrWindow,
    config::{Theme, corners},
    search_input::{Changed, SearchInput},
};
use gpui::{prelude::*, *};
use herdr_chat_protocol::{Cursor, EntryId};
use std::rc::Rc;

#[derive(Clone)]
struct Request {
    target: Target,
    cursor: Option<Cursor>,
    entry: EntryId,
    block: usize,
    document: Rc<Document>,
}

impl Request {
    fn key(&self) -> (EntryId, usize) {
        (self.entry, self.block)
    }

    /// A callback from an old frame cannot edit or send into a replacement
    /// session, a replaced message, or a removed card.
    fn live(&self, view: &HerdrWindow) -> bool {
        view.chats.pane(&self.target.pane_id).is_some_and(|pane| {
            pane.target == self.target
                && pane.session.since().map(|cursor| cursor.epoch)
                    == self.cursor.map(|cursor| cursor.epoch)
                && pane.prepared.body(self.entry).is_some_and(|body| {
                    matches!(body.as_ref(), Body::Message(parts)
                        if matches!(parts.get(self.block), Some(Part::Widget(document))
                            if document.source == self.document.source))
                })
        })
    }
}

#[derive(Clone)]
struct SliderDrag(String);

fn button(theme: &Theme, id: String, label: String, selected: bool) -> Stateful<Div> {
    let fill = if selected {
        theme.primary()
    } else {
        theme.surface
    };
    let text = super::text_ink(
        if selected {
            theme.text_on(fill)
        } else {
            theme.foreground
        },
        fill,
    );
    div()
        .id(SharedString::from(id.clone()))
        .debug_selector(move || id.clone())
        .role(Role::Button)
        .aria_label(label.clone())
        .px(px(12.))
        .py(px(7.))
        .rounded(px(corners::CONTROL))
        .border_1()
        .border_color(rgb(if selected { fill } else { theme.active }))
        .bg(rgb(fill))
        .text_color(rgb(text))
        .cursor_pointer()
        .hover(move |button| button.border_color(rgb(text)))
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .child(label)
}

fn label(theme: &Theme, text: &str) -> Div {
    div()
        .text_size(px(12.))
        .font_weight(FontWeight::MEDIUM)
        .text_color(rgb(theme.subtext()))
        .child(text.to_owned())
}

fn field(theme: &Theme, title: &str, control: impl IntoElement) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(px(7.))
        .min_w_0()
        .child(label(theme, title))
        .child(control)
}

impl HerdrWindow {
    pub(in crate::chat) fn render_chat_widget(
        &self,
        pane_id: &str,
        entry: EntryId,
        block: usize,
        document: &Rc<Document>,
        cx: &mut Context<Self>,
    ) -> Div {
        let Some(pane) = self.chats.pane(pane_id) else {
            return div();
        };
        let request = Request {
            target: pane.target.clone(),
            cursor: pane.session.since(),
            entry,
            block,
            document: document.clone(),
        };
        let theme = &self.theme;
        let prefix = format!("widget-{pane_id}-{}-{block}", entry.0);
        let form = pane.widgets.form(request.key(), document);
        let read_only = pane.session.transcript().read_only;
        let value = |component: &Component| {
            component.field().map(|(id, default)| {
                form.and_then(|form| form.values.get(id))
                    .cloned()
                    .unwrap_or(default)
            })
        };
        let mut content = div().flex().flex_col().gap(px(16.)).min_w_0();
        for (index, component) in document.components.iter().enumerate() {
            let id = format!("{prefix}-{index}");
            let part = match component {
                Component::Text { text } => div().min_w_0().child(text.clone()),
                Component::Card { title, text } => div()
                    .flex()
                    .flex_col()
                    .gap(px(6.))
                    .p(px(12.))
                    .rounded(px(corners::CONTROL))
                    .bg(rgb(theme.background))
                    .child(div().font_weight(FontWeight::SEMIBOLD).child(title.clone()))
                    .child(div().text_color(rgb(theme.subtext())).child(text.clone())),
                Component::Metric {
                    label: title,
                    value,
                    detail,
                } => div()
                    .flex()
                    .flex_col()
                    .gap(px(3.))
                    .child(label(theme, title))
                    .child(
                        div()
                            .text_size(px(self.chat_text_size() * 1.8))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(super::text_ink(theme.primary(), theme.surface)))
                            .child(value.clone()),
                    )
                    .when(!detail.is_empty(), |metric| {
                        metric.child(label(theme, detail))
                    }),
                Component::Choice {
                    id: name,
                    label: title,
                    options,
                    ..
                } => {
                    let selected = value(component);
                    let choices = div().flex().flex_wrap().gap(px(6.)).children(
                        options.iter().enumerate().map(|(option_index, option)| {
                            let request = request.clone();
                            let name = name.clone();
                            let option_value = option.clone();
                            button(
                                theme,
                                format!("{id}-{option_index}"),
                                option.clone(),
                                selected.as_ref() == Some(&Value::Text(option.clone())),
                            )
                            .on_click(cx.listener(
                                move |this, _, _, cx| {
                                    cx.stop_propagation();
                                    this.change_widget(
                                        &request,
                                        &name,
                                        Value::Text(option_value.clone()),
                                        cx,
                                    );
                                },
                            ))
                        }),
                    );
                    field(theme, title, choices)
                }
                Component::Toggle {
                    id: name,
                    label: title,
                    ..
                } => {
                    let on = matches!(value(component), Some(Value::Toggle(true)));
                    let request = request.clone();
                    let name = name.clone();
                    field(
                        theme,
                        title,
                        button(theme, id, if on { "On" } else { "Off" }.into(), on).on_click(
                            cx.listener(move |this, _, _, cx| {
                                cx.stop_propagation();
                                this.change_widget(&request, &name, Value::Toggle(!on), cx);
                            }),
                        ),
                    )
                }
                Component::Slider {
                    id: name,
                    label: title,
                    min,
                    max,
                    step,
                    value: initial,
                } => {
                    let current = match value(component) {
                        Some(Value::Number(value)) => value,
                        _ => *initial,
                    };
                    field(
                        theme,
                        title,
                        self.render_widget_slider(
                            &request,
                            &id,
                            name,
                            current,
                            (*min, *max, *step),
                            cx,
                        ),
                    )
                }
                Component::Input {
                    id: name,
                    label: title,
                    placeholder,
                    ..
                } => {
                    let shown = match value(component) {
                        Some(Value::Text(value)) => value,
                        _ => String::new(),
                    };
                    let input = form
                        .and_then(|form| form.inputs.get(name))
                        .map(|input| input.entity.clone());
                    let request = request.clone();
                    let name = name.clone();
                    let placeholder = placeholder.clone();
                    // Let unhandled keys reach the native input handler;
                    // chat's focus guards keep them out of the composer and terminal.
                    let body = div()
                        .id(SharedString::from(id.clone()))
                        .debug_selector(move || id.clone())
                        .min_h(px(36.))
                        .px(px(9.))
                        .py(px(6.))
                        .min_w_0()
                        .rounded(px(corners::CONTROL))
                        .border_1()
                        .border_color(rgb(theme.active))
                        .bg(rgb(theme.background))
                        .cursor_text()
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation());
                    let body = match input {
                        Some(input) => {
                            let mut font = self.config.ui.clone();
                            font.size = self.chat_text_size();
                            input.update(cx, |input, cx| {
                                input.set_appearance(font, self.theme.clone(), cx)
                            });
                            body.child(input)
                        }
                        None => body
                            .text_color(rgb(if shown.is_empty() {
                                theme.muted
                            } else {
                                theme.foreground
                            }))
                            .child(if shown.is_empty() {
                                placeholder.clone()
                            } else {
                                shown
                            })
                            .on_click(cx.listener(move |this, _, window, cx| {
                                cx.stop_propagation();
                                this.edit_widget_input(&request, &name, &placeholder, window, cx);
                            })),
                    };
                    field(theme, title, body)
                }
                Component::Table { columns, rows } => {
                    let row = |cells: &[String], heading: bool| {
                        div()
                            .flex()
                            .min_w_0()
                            .bg(rgb(if heading { theme.active } else { theme.surface }))
                            .children(cells.iter().map(|cell| {
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .p(px(8.))
                                    .when(heading, |cell| cell.font_weight(FontWeight::SEMIBOLD))
                                    .child(cell.clone())
                            }))
                    };
                    div()
                        .flex()
                        .flex_col()
                        .min_w_0()
                        .rounded(px(corners::CONTROL))
                        .overflow_hidden()
                        .border_1()
                        .border_color(rgb(theme.active))
                        .child(row(columns, true))
                        .children(rows.iter().map(|cells| row(cells, false)))
                }
                Component::Chart {
                    label: title,
                    points,
                } => {
                    let maximum = points
                        .iter()
                        .map(|point| point.value)
                        .fold(0., f64::max)
                        .max(1.);
                    let bars = div()
                        .flex()
                        .flex_col()
                        .gap(px(10.))
                        .children(points.iter().map(|point| {
                            div()
                                .flex()
                                .flex_col()
                                .gap(px(4.))
                                .child(
                                    div()
                                        .flex()
                                        .justify_between()
                                        .gap(px(8.))
                                        .child(point.label.clone())
                                        .child(point.value.to_string()),
                                )
                                .child(
                                    div()
                                        .h(px(7.))
                                        .w_full()
                                        .rounded_full()
                                        .bg(rgb(theme.active))
                                        .child(
                                            div()
                                                .h_full()
                                                .w(relative((point.value / maximum) as f32))
                                                .rounded_full()
                                                .bg(rgb(theme.primary())),
                                        ),
                                )
                        }));
                    field(theme, title, bars)
                }
                Component::Action {
                    label: title,
                    prompt,
                    mode,
                } => {
                    let request = request.clone();
                    let prompt = prompt.clone();
                    let mode = *mode;
                    let caption = match mode {
                        ActionMode::Draft => "Add to message",
                        ActionMode::Send => "Send to agent",
                    };
                    let caption = if read_only {
                        "Read-only transcript"
                    } else {
                        caption
                    };
                    div().flex().flex_col().gap(px(5.)).child(
                        crate::accessibility::disabled(
                            button(theme, id, format!("{title} · {caption}"), !read_only),
                            read_only,
                        )
                        .when(read_only, |button| button.opacity(0.5))
                        .when(!read_only, |button| {
                            button.on_click(cx.listener(move |this, _, window, cx| {
                                cx.stop_propagation();
                                this.widget_action(&request, &prompt, mode, window, cx);
                            }))
                        }),
                    )
                }
            };
            content = content.child(part);
        }
        div()
            .debug_selector(|| "chat-widget".into())
            .flex()
            .flex_col()
            .gap(px(14.))
            .min_w_0()
            .p(px(16.))
            .rounded(px(corners::CONTROL))
            .border_1()
            .border_color(rgb(theme.active))
            .bg(rgb(theme.surface))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(4.))
                    .child(
                        div()
                            .text_size(px(10.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(super::text_ink(theme.primary(), theme.surface)))
                            .child("INTERACTIVE"),
                    )
                    .when(!document.title.is_empty(), |header| {
                        header.child(
                            div()
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(document.title.clone()),
                        )
                    }),
            )
            .child(content)
    }

    fn render_widget_slider(
        &self,
        request: &Request,
        id: &str,
        name: &str,
        value: f64,
        range: (f64, f64, f64),
        cx: &mut Context<Self>,
    ) -> Div {
        let theme = &self.theme;
        let (min, max, step) = range;
        let fraction = ((value - min) / (max - min)).clamp(0., 1.) as f32;
        let drag_id = id.to_owned();
        let target_id = drag_id.clone();
        let request_drag = request.clone();
        let field_name = name.to_owned();
        let track = div()
            .id(SharedString::from(format!("{id}-track")))
            .flex_1()
            .min_w(px(60.))
            .h(px(24.))
            .flex()
            .items_center()
            .cursor_pointer()
            .child(
                div()
                    .w_full()
                    .h(px(6.))
                    .rounded_full()
                    .bg(rgb(theme.active))
                    .child(
                        div()
                            .w(relative(fraction))
                            .h_full()
                            .rounded_full()
                            .bg(rgb(theme.primary())),
                    ),
            )
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_drag(SliderDrag(drag_id), |_, _, _, cx| cx.new(|_| Empty))
            .on_drag_move(
                cx.listener(move |this, event: &DragMoveEvent<SliderDrag>, _, cx| {
                    if event.drag(cx).0 != target_id {
                        return;
                    }
                    let width = f32::from(event.bounds.size.width).max(1.);
                    let fraction = f64::from(
                        (f32::from(event.event.position.x) - f32::from(event.bounds.left()))
                            / width,
                    );
                    this.change_widget(
                        &request_drag,
                        &field_name,
                        Value::Number(slider_value(fraction, min, max, step)),
                        cx,
                    );
                }),
            );
        let stepper = |suffix: &str, glyph: &str, delta: f64, cx: &mut Context<Self>| {
            let request = request.clone();
            let name = name.to_owned();
            button(theme, format!("{id}-{suffix}"), glyph.into(), false).on_click(cx.listener(
                move |this, _, _, cx| {
                    cx.stop_propagation();
                    this.change_widget(
                        &request,
                        &name,
                        Value::Number((value + delta).clamp(min, max)),
                        cx,
                    );
                },
            ))
        };
        div()
            .flex()
            .flex_col()
            .gap(px(4.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(stepper("minus", "−", -step, cx))
                    .child(track)
                    .child(stepper("plus", "+", step, cx))
                    .child(div().min_w(px(44.)).text_right().child(value.to_string())),
            )
            .child(label(theme, &format!("{min} to {max}")))
    }

    fn change_widget(&mut self, request: &Request, id: &str, value: Value, cx: &mut Context<Self>) {
        if !request.live(self) {
            return;
        }
        if let Some(pane) = self.chats.pane_mut(&request.target.pane_id) {
            let form = pane.widgets.edit(request.key(), &request.document);
            if request.document.set(&mut form.values, id, value) {
                pane.remeasure(request.entry);
                cx.notify();
            }
        }
    }

    fn edit_widget_input(
        &mut self,
        request: &Request,
        id: &str,
        placeholder: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !request.live(self) {
            return;
        }
        let Some(pane) = self.chats.pane_mut(&request.target.pane_id) else {
            return;
        };
        let form = pane.widgets.edit(request.key(), &request.document);
        if let Some(input) = form.inputs.get(id) {
            let focus = input.entity.read(cx).focus.clone();
            window.focus(&focus, cx);
            return;
        }
        let value = match form.values.get(id) {
            Some(Value::Text(value)) => value.clone(),
            _ => return,
        };
        let input = cx.new(|cx| {
            let mut input = SearchInput::new(cx);
            input.set_frameless(cx);
            input.set_placeholder(placeholder, cx);
            input.set_text_at(&value, value.len(), cx);
            input.set_appearance(self.config.ui.clone(), self.theme.clone(), cx);
            input
        });
        let request_changed = request.clone();
        let name = id.to_owned();
        let changed = cx.subscribe(&input, move |this, input, _: &Changed, cx| {
            if input.read(cx).is_composing() {
                return;
            }
            let value = bounded_input(input.read(cx).text());
            if value != input.read(cx).text() {
                input.update(cx, |input, cx| input.set_text_at(&value, value.len(), cx));
            }
            this.change_widget(&request_changed, &name, Value::Text(value), cx);
        });
        let focus = input.read(cx).focus.clone();
        form.inputs.insert(
            id.to_owned(),
            Input {
                entity: input,
                _changed: changed,
            },
        );
        pane.remeasure(request.entry);
        window.focus(&focus, cx);
        cx.notify();
    }

    fn widget_action(
        &mut self,
        request: &Request,
        prefix: &str,
        mode: ActionMode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !request.live(self) {
            return;
        }
        let Some(pane) = self.chats.pane_mut(&request.target.pane_id) else {
            return;
        };
        if pane.session.transcript().read_only {
            return;
        }
        let form = pane.widgets.edit(request.key(), &request.document);
        if form
            .inputs
            .values()
            .any(|input| input.entity.read(cx).is_composing())
        {
            return;
        }
        let prompt = match request.document.prompt(prefix, &form.values) {
            Ok(prompt) => prompt,
            Err(error) => {
                pane.error = Some(error.to_string());
                cx.notify();
                return;
            }
        };
        let Some(composer) = pane.composer.clone() else {
            return;
        };
        if mode == ActionMode::Send
            && (!pane.attached() || !pane.session.transcript().status.accepts_prompts())
        {
            pane.error = Some("The agent is not ready to receive this form".into());
            cx.notify();
            return;
        }
        let existing = composer.read(cx).text().to_owned();
        if mode == ActionMode::Send
            && (!existing.is_empty()
                || !pane.draft.is_empty()
                || !pane.attachments.is_empty()
                || pane.uploading > 0)
        {
            pane.error = Some("Finish your current message before sending this form".into());
            cx.notify();
            return;
        }
        if !existing.is_empty() {
            pane.draft.push(existing);
            pane.draft.push(String::new());
        }
        let mut lines: Vec<String> = prompt.lines().map(str::to_owned).collect();
        let last = lines.pop().unwrap_or_default();
        pane.draft.extend(lines);
        pane.error = None;
        composer.update(cx, |input, cx| input.set_text_at(&last, last.len(), cx));
        let focus = composer.read(cx).focus.clone();
        window.focus(&focus, cx);
        if mode == ActionMode::Send {
            self.send_chat_composer(&request.target.pane_id, cx);
        }
        cx.notify();
    }
}

#[cfg(test)]
mod tests;
