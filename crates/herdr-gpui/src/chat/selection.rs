//! Transcript text selection, independent of which virtualized rows are mounted.

mod document;

pub(crate) use document::{Document, Segment};

use super::rich::MAX_CODE_LINES;
use document::Order;
use herdr_chat_protocol::EntryId;
use std::ops::Range;

/// Which of an entry's texts a segment belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Region {
    /// One prepared body part, by index. An entry drawn without prepared
    /// parts, such as a notice or a folded summary, has its text in `Body(0)`.
    Body(usize),
    /// The original record behind a notice, shown when it is expanded.
    Details,
    /// The header above historical Q&A or a tool call.
    Label,
    /// The original side of an expanded before/after preview.
    ToolBefore(usize),
    /// An expanded tool call's locations, one row each.
    ToolLocations,
    /// One decoded, visible tool preview.
    ToolPreview(usize),
    /// The label above one decoded tool preview.
    ToolPreviewLabel(usize),
    /// An expanded tool call's full title.
    ToolTitle,
}

/// Where a segment sits in its region: the block, the row within it, and
/// the table column.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub(crate) struct Cell {
    pub(crate) block: usize,
    pub(crate) row: usize,
    pub(crate) column: usize,
}

impl Cell {
    pub(crate) fn block(block: usize) -> Self {
        Self {
            block,
            ..Self::default()
        }
    }

    pub(crate) fn row(row: usize) -> Self {
        Self {
            row,
            ..Self::default()
        }
    }
}

/// Drawn and selectable prefixes must agree, including compact patch headers.
pub(crate) fn tool_code_limit(language: &str, expanded: bool) -> usize {
    if expanded {
        MAX_CODE_LINES
    } else if language.eq_ignore_ascii_case("diff") || language.eq_ignore_ascii_case("patch") {
        10
    } else {
        super::prepared::tool::COMPACT_LINES
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct Position {
    region: Region,
    cell: Cell,
}

/// A text segment in one exact prepared revision and snapshot. Old callbacks
/// cannot start a selection in a replacement session which reused entry ids.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct Key {
    entry: EntryId,
    revision: u64,
    epoch: u64,
    position: Position,
}

impl Key {
    pub(crate) fn entry(self) -> EntryId {
        self.entry
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Caret {
    pub(crate) key: Key,
    pub(crate) offset: usize,
}

#[derive(Clone, Debug)]
struct RangeSelection {
    anchor: Caret,
    head: Caret,
    /// Entry order/revisions throughout the selection, not just its endpoints.
    covered: Vec<(EntryId, u64)>,
}

#[derive(Default, Debug)]
pub(crate) struct Selection {
    range: Option<RangeSelection>,
    selecting: bool,
}

impl Selection {
    pub(crate) fn is_empty(&self) -> bool {
        self.range
            .as_ref()
            .is_none_or(|range| range.anchor == range.head)
    }

    pub(crate) fn is_selecting(&self) -> bool {
        self.selecting
    }

    pub(crate) fn clear(&mut self) -> bool {
        self.selecting = false;
        self.range.take().is_some()
    }

    pub(crate) fn release(&mut self) {
        self.selecting = false;
    }

    /// Returns whether stale selection state was cleared. An append outside
    /// its span leaves a selection intact; replacing any covered entry does not.
    pub(crate) fn validate(&mut self, document: &Document) -> bool {
        if self
            .range
            .as_ref()
            .is_some_and(|range| !range.valid(document))
        {
            self.clear()
        } else {
            false
        }
    }

    pub(crate) fn press(
        &mut self,
        document: &Document,
        caret: Caret,
        clicks: usize,
        extend: bool,
    ) -> bool {
        let Some(segment) = document.segment(caret.key) else {
            return false;
        };
        self.validate(document);
        self.selecting = true;
        let atom = segment
            .atomic()
            .iter()
            .find(|range| range.contains(&caret.offset))
            .cloned();
        let caret = Caret {
            offset: segment.caret(caret.offset),
            ..caret
        };
        if extend && let Some(range) = self.range.as_mut() {
            range.head = caret;
            range.remember(document);
            return true;
        }
        let offsets = match clicks {
            0 | 1 => caret.offset..caret.offset,
            2 => atom.unwrap_or_else(|| word_at(segment.text(), caret.offset)),
            _ => 0..segment.text().len(),
        };
        let mut range = RangeSelection {
            anchor: Caret {
                offset: offsets.start,
                ..caret
            },
            head: Caret {
                offset: offsets.end,
                ..caret
            },
            covered: Vec::new(),
        };
        range.remember(document);
        self.range = Some(range);
        true
    }

    pub(crate) fn drag(&mut self, document: &Document, caret: Caret) -> bool {
        if !self.selecting || self.validate(document) {
            return false;
        }
        let Some(segment) = document.segment(caret.key) else {
            return false;
        };
        let Some(range) = self.range.as_mut() else {
            return false;
        };
        let head = Caret {
            offset: segment.caret(caret.offset),
            ..caret
        };
        if range.head == head {
            return false;
        }
        range.head = head;
        range.remember(document);
        true
    }

    pub(crate) fn select_all(&mut self, document: &Document) -> bool {
        let Some((anchor, head)) = document.ends() else {
            return self.clear();
        };
        let mut range = RangeSelection {
            anchor,
            head,
            covered: Vec::new(),
        };
        range.remember(document);
        self.range = Some(range);
        self.selecting = false;
        true
    }

    pub(crate) fn highlight(&self, document: &Document, key: Key) -> Option<Range<usize>> {
        let range = self.range.as_ref()?;
        let (start, end) = range.ordered(document)?;
        let position = document.order(key)?;
        if position < document.order(start.key)? || position > document.order(end.key)? {
            return None;
        }
        let from = if key == start.key { start.offset } else { 0 };
        let to = if key == end.key {
            end.offset
        } else {
            document.segment(key)?.text().len()
        };
        (from < to).then_some(from..to)
    }

    /// Selected visible text, with original tabs in code and TeX for math.
    /// Refuse over-limit copies rather than silently copying a partial range.
    pub(crate) fn text(&self, document: &Document) -> Option<String> {
        let range = self.range.as_ref().filter(|range| range.valid(document))?;
        let (start, end) = range.ordered(document)?;
        if start == end {
            return None;
        }
        let from = document.order(start.key)?;
        let to = document.order(end.key)?;
        let mut text = String::new();
        let mut first = true;
        for (entry_index, entry) in document
            .entries
            .iter()
            .enumerate()
            .take(to.entry + 1)
            .skip(from.entry)
        {
            for (segment_index, segment) in entry.segments.iter().enumerate() {
                let position = Order {
                    entry: entry_index,
                    segment: segment_index,
                };
                if position < from || position > to {
                    continue;
                }
                let separator = if first {
                    ""
                } else if segment_index == 0 {
                    "\n\n"
                } else {
                    segment.separator
                };
                let offsets = (if position == from { start.offset } else { 0 })
                    ..(if position == to {
                        end.offset
                    } else {
                        segment.text().len()
                    });
                let selected = segment.copy(offsets);
                if text
                    .len()
                    .saturating_add(separator.len())
                    .saturating_add(selected.len())
                    > crate::terminal::MAX_SELECTION_BYTES
                {
                    return None;
                }
                text.push_str(separator);
                text.push_str(selected);
                first = false;
            }
        }
        (!text.is_empty()).then_some(text)
    }
}

impl RangeSelection {
    fn ordered(&self, document: &Document) -> Option<(Caret, Caret)> {
        let anchor = (document.order(self.anchor.key)?, self.anchor.offset);
        let head = (document.order(self.head.key)?, self.head.offset);
        Some(if anchor <= head {
            (self.anchor, self.head)
        } else {
            (self.head, self.anchor)
        })
    }

    fn remember(&mut self, document: &Document) {
        self.covered.clear();
        let Some((start, end)) = self.ordered(document) else {
            return;
        };
        let (Some(start), Some(end)) = (document.order(start.key), document.order(end.key)) else {
            return;
        };
        self.covered.extend(
            document.entries[start.entry..=end.entry]
                .iter()
                .map(|entry| (entry.id, entry.version)),
        );
    }

    fn valid(&self, document: &Document) -> bool {
        let Some((start, end)) = self.ordered(document) else {
            return false;
        };
        let (Some(start), Some(end)) = (document.order(start.key), document.order(end.key)) else {
            return false;
        };
        document.entries[start.entry..=end.entry]
            .iter()
            .map(|entry| (entry.id, entry.version))
            .eq(self.covered.iter().copied())
    }
}

fn word_at(text: &str, offset: usize) -> Range<usize> {
    let offset = text.floor_char_boundary(offset.min(text.len()));
    let Some(here) = text[offset..].chars().next() else {
        return offset..offset;
    };
    let is_word = |ch: char| ch.is_alphanumeric() || ch == '_';
    if !is_word(here) {
        return offset..offset + here.len_utf8();
    }
    let start = text[..offset]
        .char_indices()
        .rev()
        .take_while(|&(_, ch)| is_word(ch))
        .last()
        .map_or(offset, |(at, _)| at);
    let end = text[offset..]
        .char_indices()
        .find(|&(_, ch)| !is_word(ch))
        .map_or(text.len(), |(at, _)| offset + at);
    start..end
}

#[cfg(test)]
mod tests;
