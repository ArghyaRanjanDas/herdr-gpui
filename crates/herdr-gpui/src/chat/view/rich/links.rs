//! Explicit rich-text links use the pane's existing document-opening policy.
//! Paths from SSH conversations stay visible and never address this laptop.

use super::*;
use crate::{
    chat::{Route, Target},
    terminal::{PaneLink, RowLink, file_target},
};
use herdr_chat_protocol::Cursor;
use std::borrow::Cow;

#[derive(Clone)]
pub(super) struct Links {
    view: WeakEntity<HerdrWindow>,
    target: Option<Target>,
    cursor: Option<Cursor>,
    selection_epoch: u64,
    local: bool,
}

impl Links {
    pub(super) fn new(view: &HerdrWindow, pane_id: &str, cx: &Context<HerdrWindow>) -> Self {
        let pane = view.chats.pane(pane_id);
        Self {
            view: cx.entity().downgrade(),
            target: pane.map(|pane| pane.target.clone()),
            cursor: pane.and_then(|pane| pane.session.since()),
            selection_epoch: view.selection_epoch,
            local: !view.selected_is_remote()
                && pane.is_some_and(|pane| pane.target.route == Route::Local),
        }
    }

    pub(super) fn content<'a>(&self, content: &'a [Inline]) -> Cow<'a, [Inline]> {
        if self.local {
            Cow::Borrowed(content)
        } else {
            remote_content(content)
        }
    }

    pub(super) fn open(&self, destination: &str, cx: &mut App) {
        if let Ok(url) = crate::browser::WebUrl::try_from(destination) {
            cx.open_url(url.as_str());
            return;
        }
        let Some(target) = self.target.as_ref().filter(|_| self.local) else {
            return;
        };
        let Some(link) = file_target(destination) else {
            return;
        };
        let _ = self.view.update(cx, |view, cx| {
            let current = view.chats.pane(&target.pane_id).is_some_and(|pane| {
                pane.target == *target
                    && pane.session.since().map(|cursor| cursor.epoch)
                        == self.cursor.map(|cursor| cursor.epoch)
            });
            if !current || view.selected_is_remote() || view.selection_epoch != self.selection_epoch
            {
                return;
            }
            let Some(file) = view.file_link(PaneLink {
                pane_id: target.pane_id.clone(),
                link: RowLink {
                    target: link,
                    row: 0,
                    columns: 0..0,
                },
            }) else {
                return;
            };
            view.open_file_link(
                file,
                view.config.open_files_in == crate::config::FileTarget::Editor,
                cx,
            );
        });
    }
}

fn remote_content(content: &[Inline]) -> Cow<'_, [Inline]> {
    if !content.iter().any(|part| {
        matches!(part, Inline::Text(_, style)
            if style.link.as_deref().is_some_and(|url| file_target(url).is_some()))
    }) {
        return Cow::Borrowed(content);
    }
    let mut shown = Vec::new();
    for (index, part) in content.iter().enumerate() {
        let Inline::Text(text, style) = part else {
            shown.push(part.clone());
            continue;
        };
        let Some(target) = style
            .link
            .as_deref()
            .filter(|url| file_target(url).is_some())
        else {
            shown.push(part.clone());
            continue;
        };
        let mut inert = style.clone();
        inert.link = None;
        shown.push(Inline::Text(text.clone(), inert));
        // Emphasis/code can split one label into several styled text spans.
        // Keep their formatting, then show its destination once at the end.
        if !matches!(content.get(index + 1), Some(Inline::Text(_, next))
            if next.link.as_deref() == Some(target))
        {
            shown.push(Inline::Text(format!(" ({target})"), Style::default()));
        }
    }
    Cow::Owned(shown)
}

#[cfg(test)]
mod tests;
