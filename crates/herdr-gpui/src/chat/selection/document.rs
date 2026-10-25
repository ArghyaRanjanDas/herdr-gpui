//! A cached readable projection of prepared bodies, in transcript order.

use super::{
    Caret, DETAILS_PART, Key, LABEL_PART, MAX_CODE_LINE_CHARS, MAX_CODE_LINES, Position, TAB_WIDTH,
};
use crate::{
    chat::{
        prepared::{Body, Part, tool_label, tool_text},
        rich::{Block, Inline},
    },
    release_notes::{Kind, Line},
};
use herdr_chat_protocol::{Entry, EntryId, EntryKind, Transcript};
use std::{
    collections::{HashMap, HashSet},
    ops::Range,
    rc::Rc,
    sync::Arc,
};
use unicode_width::UnicodeWidthChar;

enum Text {
    Owned(String),
    Shared(Arc<str>),
    Lines(Rc<[Line]>, usize),
}

struct CodeSource {
    text: Arc<str>,
    range: Range<usize>,
    /// Original and displayed byte boundaries, including the final boundary.
    offsets: Vec<(usize, usize)>,
}

pub(crate) struct Segment {
    position: Position,
    text: Text,
    atoms: Vec<Range<usize>>,
    code: Option<CodeSource>,
    pub(super) separator: &'static str,
}

impl Segment {
    pub(crate) fn text(&self) -> &str {
        match &self.text {
            Text::Owned(text) => text,
            Text::Shared(text) => text,
            Text::Lines(lines, index) => &lines[*index].text,
        }
    }

    pub(crate) fn atomic(&self) -> &[Range<usize>] {
        &self.atoms
    }

    /// Original byte offset relative to this code line; ordinary prose is an
    /// identity map. A caret inside expanded tab spaces maps to the tab start.
    pub(crate) fn source_offset(&self, offset: usize) -> usize {
        let offset = self
            .text()
            .floor_char_boundary(offset.min(self.text().len()));
        self.code.as_ref().map_or(offset, |code| {
            let index = code
                .offsets
                .partition_point(|&(_, shown)| shown <= offset)
                .saturating_sub(1);
            code.offsets.get(index).map_or(0, |&(source, _)| source)
        })
    }

    pub(crate) fn display_offset(&self, offset: usize) -> Option<usize> {
        self.code.as_ref().map_or_else(
            || {
                (offset <= self.text().len() && self.text().is_char_boundary(offset))
                    .then_some(offset)
            },
            |code| {
                code.offsets
                    .binary_search_by_key(&offset, |&(source, _)| source)
                    .ok()
                    .map(|index| code.offsets[index].1)
            },
        )
    }

    pub(super) fn caret(&self, offset: usize) -> usize {
        let offset = self
            .text()
            .floor_char_boundary(offset.min(self.text().len()));
        self.atoms
            .iter()
            .find(|atom| atom.start < offset && offset < atom.end)
            .map_or(offset, |atom| {
                if offset - atom.start < atom.end - offset {
                    atom.start
                } else {
                    atom.end
                }
            })
    }

    pub(super) fn copy(&self, offsets: Range<usize>) -> &str {
        if offsets.is_empty() {
            return "";
        }
        let Some(code) = &self.code else {
            return &self.text()[offsets];
        };
        let start = self.source_offset(offsets.start);
        let index = code
            .offsets
            .partition_point(|&(_, shown)| shown < offsets.end);
        let end = code
            .offsets
            .get(index)
            .map_or(code.range.len(), |&(source, _)| source);
        &code.text[code.range.start + start..code.range.start + end.max(start)]
    }
}

pub(super) struct EntryText {
    pub(super) id: EntryId,
    pub(super) revision: u64,
    pub(super) version: u64,
    expanded: bool,
    pub(super) segments: Vec<Segment>,
    positions: HashMap<Position, usize>,
}

#[derive(Default)]
pub(crate) struct Document {
    pub(super) entries: Vec<Rc<EntryText>>,
    positions: HashMap<EntryId, usize>,
    epoch: u64,
    remote_links: bool,
    expanded: HashSet<EntryId>,
    version: u64,
}

impl Document {
    pub(crate) fn key(
        &self,
        entry: EntryId,
        part: usize,
        block: usize,
        row: usize,
        column: usize,
    ) -> Option<Key> {
        let projection = self.entries.get(*self.positions.get(&entry)?)?;
        let position = Position {
            part,
            block,
            row,
            column,
        };
        projection.positions.get(&position)?;
        Some(Key {
            entry,
            revision: projection.version,
            epoch: self.epoch,
            position,
        })
    }

    pub(crate) fn segment(&self, key: Key) -> Option<&Segment> {
        let (entry, segment) = self.order(key)?;
        self.entries.get(entry)?.segments.get(segment)
    }

    pub(super) fn order(&self, key: Key) -> Option<(usize, usize)> {
        if key.epoch != self.epoch {
            return None;
        }
        let entry = *self.positions.get(&key.entry)?;
        let projection = self.entries.get(entry)?;
        (projection.version == key.revision).then_some(())?;
        Some((entry, *projection.positions.get(&key.position)?))
    }

    pub(super) fn ends(&self) -> Option<(Caret, Caret)> {
        let first = self
            .entries
            .iter()
            .find(|entry| !entry.segments.is_empty())?;
        let last = self
            .entries
            .iter()
            .rev()
            .find(|entry| !entry.segments.is_empty())?;
        let caret = |entry: &EntryText, segment: &Segment, offset| Caret {
            key: Key {
                entry: entry.id,
                revision: entry.version,
                epoch: self.epoch,
                position: segment.position,
            },
            offset,
        };
        Some((
            caret(first, first.segments.first()?, 0),
            caret(
                last,
                last.segments.last()?,
                last.segments.last()?.text().len(),
            ),
        ))
    }

    pub(in crate::chat) fn set_remote_links(&mut self, remote: bool) {
        if self.remote_links != remote {
            self.clear();
            self.remote_links = remote;
        }
    }

    pub(in crate::chat) fn clear(&mut self) {
        self.entries.clear();
        self.positions.clear();
        self.epoch = self.epoch.wrapping_add(1);
    }

    pub(in crate::chat) fn set_expanded(&mut self, expanded: &HashSet<EntryId>) {
        self.expanded.clone_from(expanded);
    }

    pub(in crate::chat) fn update(
        &mut self,
        transcript: &Transcript,
        keys: &[(EntryId, u64)],
        bodies: &HashMap<EntryId, (u64, Rc<Body>)>,
    ) {
        let mut cached: HashMap<_, _> = self
            .entries
            .drain(..)
            .map(|entry| (entry.id, entry))
            .collect();
        self.positions.clear();
        for (entry, &(id, revision)) in transcript.entries.iter().zip(keys) {
            if entry.id != id {
                continue;
            }
            let expanded = self.expanded.contains(&id);
            let projected = match cached.remove(&id) {
                Some(projected)
                    if projected.revision == revision && projected.expanded == expanded =>
                {
                    projected
                }
                _ => {
                    self.version = self.version.wrapping_add(1);
                    Rc::new(project(
                        entry,
                        revision,
                        self.version,
                        bodies.get(&id).map(|(_, body)| body.as_ref()),
                        self.remote_links,
                        expanded,
                    ))
                }
            };
            self.positions.insert(id, self.entries.len());
            self.entries.push(projected);
        }
    }
}

fn project(
    entry: &Entry,
    revision: u64,
    version: u64,
    body: Option<&Body>,
    remote: bool,
    expanded: bool,
) -> EntryText {
    let mut builder = Builder {
        segments: Vec::new(),
        remote,
    };
    match body {
        Some(Body::Message(parts)) => builder.parts(parts),
        Some(Body::Answers { parts, .. }) => {
            if let EntryKind::Notice { text, .. } = &entry.kind {
                builder.plain(LABEL_PART, 0, 0, crate::notifications::safe_text(text, 240));
            }
            builder.parts(parts);
        }
        Some(Body::Summary(summary)) => {
            if expanded {
                for (index, line) in summary.lines.iter().enumerate() {
                    builder.plain(0, 0, index, line.text.clone());
                }
            } else {
                builder.plain(0, 0, 0, summary.gist.clone());
            }
        }
        Some(Body::Tool(_)) => {
            if let EntryKind::Tool(tool) = &entry.kind {
                builder.plain(
                    LABEL_PART,
                    0,
                    0,
                    crate::notifications::safe_text(tool_label(&tool.title), 300),
                );
                if expanded {
                    builder.plain(0, 0, 0, tool.title.clone());
                    for (row, location) in tool.locations.iter().enumerate() {
                        builder.plain(1, 0, row, location.clone());
                    }
                    for (block, content) in tool.content.iter().enumerate() {
                        if let Some(text) = tool_text(content) {
                            builder.plain(2, block, 0, text);
                        }
                    }
                }
            }
        }
        Some(Body::Image(_)) => {}
        None => match &entry.kind {
            EntryKind::Thought { text } if !expanded => {
                let first = text
                    .lines()
                    .find(|line| !line.trim().is_empty())
                    .unwrap_or("");
                builder.plain(0, 0, 0, crate::notifications::safe_text(first, 120));
            }
            EntryKind::Notice {
                text,
                details: Some(_),
                ..
            } => builder.plain(0, 0, 0, crate::notifications::safe_text(text, 240)),
            EntryKind::User { text }
            | EntryKind::Agent { text }
            | EntryKind::Thought { text }
            | EntryKind::Notice { text, .. } => {
                let end = text.floor_char_boundary(text.len().min(256 * 1024));
                builder.plain(0, 0, 0, text[..end].to_owned());
            }
            EntryKind::Turn(turn) => builder.plain(0, 0, 0, turn.describe()),
            EntryKind::Tool(_) | EntryKind::Image { .. } => {}
        },
    }
    if expanded
        && let EntryKind::Notice {
            details: Some(details),
            ..
        } = &entry.kind
    {
        let end = details.floor_char_boundary(details.len().min(256 * 1024));
        builder.plain(DETAILS_PART, 0, 0, details[..end].to_owned());
    }
    let positions = builder
        .segments
        .iter()
        .enumerate()
        .map(|(index, segment)| (segment.position, index))
        .collect();
    EntryText {
        id: entry.id,
        revision,
        version,
        expanded,
        segments: builder.segments,
        positions,
    }
}

struct Builder {
    segments: Vec<Segment>,
    remote: bool,
}

impl Builder {
    fn push(
        &mut self,
        position: Position,
        text: Text,
        atoms: Vec<Range<usize>>,
        code: Option<CodeSource>,
        separator: &'static str,
    ) {
        self.segments.push(Segment {
            position,
            text,
            atoms,
            code,
            separator,
        });
    }

    fn plain(&mut self, part: usize, block: usize, row: usize, text: String) {
        self.push(
            Position {
                part,
                block,
                row,
                column: 0,
            },
            Text::Owned(text),
            Vec::new(),
            None,
            "\n",
        );
    }

    fn lines(&mut self, part: usize, lines: &Rc<[Line]>) {
        for (row, _) in lines.iter().enumerate() {
            self.push(
                Position {
                    part,
                    block: 0,
                    row,
                    column: 0,
                },
                Text::Lines(lines.clone(), row),
                Vec::new(),
                None,
                "\n",
            );
        }
    }

    fn parts(&mut self, parts: &[Part]) {
        for (part, content) in parts.iter().enumerate() {
            let first = self.segments.len();
            match content {
                Part::Rich(blocks) => self.blocks(part, blocks),
                Part::Text(lines) => self.lines(part, lines),
                // Source owned by a preview/card is not visible transcript
                // text. A range across it must never reveal the hidden payload.
                Part::Diagram(_, _) | Part::Html { .. } | Part::Widget(_) => {}
                Part::Picture { alt } | Part::Remote { alt, .. } => {
                    self.plain(part, 0, 0, alt.clone())
                }
            }
            if let Some(segment) = self.segments.get_mut(first) {
                segment.separator = "\n\n";
            }
        }
    }

    fn blocks(&mut self, part: usize, blocks: &[Block]) {
        for (block, content) in blocks.iter().enumerate() {
            let position = Position {
                part,
                block,
                row: 0,
                column: 0,
            };
            match content {
                Block::Paragraph { kind, content, .. } => {
                    if *kind != Kind::Rule {
                        self.inline(position, content, "\n\n");
                    }
                }
                Block::Math { source, .. } => self.push(
                    position,
                    Text::Shared(source.clone()),
                    std::iter::once(0..source.len()).collect(),
                    None,
                    "\n\n",
                ),
                Block::Code { source, .. } => {
                    let mut offset = 0;
                    for (row, raw) in source
                        .split_inclusive('\n')
                        .take(MAX_CODE_LINES)
                        .enumerate()
                    {
                        let line = raw
                            .strip_suffix('\n')
                            .map(|line| line.strip_suffix('\r').unwrap_or(line))
                            .unwrap_or(raw);
                        let (text, offsets, end) = code_line(line);
                        self.push(
                            Position { row, ..position },
                            Text::Owned(text),
                            Vec::new(),
                            Some(CodeSource {
                                text: source.clone(),
                                range: offset..offset + end,
                                offsets,
                            }),
                            if row == 0 { "\n\n" } else { "\n" },
                        );
                        offset += raw.len();
                    }
                }
                Block::Table { rows, .. } => {
                    for (row, cells) in rows.iter().enumerate() {
                        for (column, content) in cells.iter().enumerate() {
                            self.inline(
                                Position {
                                    row,
                                    column,
                                    ..position
                                },
                                content,
                                if column > 0 {
                                    "\t"
                                } else if row > 0 {
                                    "\n"
                                } else {
                                    "\n\n"
                                },
                            );
                        }
                    }
                }
            }
        }
    }

    fn inline(&mut self, position: Position, content: &[Inline], separator: &'static str) {
        let mut text = String::new();
        let mut atoms = Vec::new();
        for (index, inline) in content.iter().enumerate() {
            match inline {
                Inline::Text(value, style) => {
                    text.push_str(value);
                    if self.remote
                        && let Some(target) = style
                            .link
                            .as_deref()
                            .filter(|url| crate::terminal::file_target(url).is_some())
                        && !matches!(content.get(index + 1), Some(Inline::Text(_, next)) if next.link.as_deref() == Some(target))
                    {
                        text.push_str(" (");
                        text.push_str(target);
                        text.push(')');
                    }
                }
                Inline::Math { source, .. } => {
                    let start = text.len();
                    text.push_str(source);
                    atoms.push(start..text.len());
                }
                Inline::Break => text.push('\n'),
            }
        }
        self.push(position, Text::Owned(text), atoms, None, separator);
    }
}

fn code_line(raw: &str) -> (String, Vec<(usize, usize)>, usize) {
    let mut text = String::new();
    let mut offsets = Vec::new();
    let (mut column, mut end) = (0, 0);
    for (at, ch) in raw.char_indices().take(MAX_CODE_LINE_CHARS) {
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
    (text, offsets, end)
}
