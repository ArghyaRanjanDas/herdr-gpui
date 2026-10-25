//! Bounded, pure projections of native Codex rollout and Claude JSONL records.
//! Logs are data: no links, tool calls, permissions, or files in them are run.

use std::{
    collections::{HashMap, HashSet},
    fs::File,
    io::{self, Read, Seek, SeekFrom},
    path::PathBuf,
    time::SystemTime,
};

use herdr_chat_protocol::{
    ConfigOption, Entry, EntryId, EntryKind, OptionCategory, OptionKind, SessionInfo, Tool,
    ToolContent, ToolKind, ToolStatus, Transcript, Usage,
};
use serde_json::Value;

use crate::{
    agents::Agent,
    session::{MAX_ENTRIES, MAX_TRANSCRIPT_TEXT, keep_from},
};

mod codex_messages;
mod notices;
mod question_replies;

const TAIL: u64 = 8 * 1024 * 1024;
const RECORD: usize = 1024 * 1024;
const TEXT: usize = 128 * 1024;
const BLOCKS: usize = 1024;

pub(crate) struct Source {
    agent: Agent,
    path: PathBuf,
    pinned: Option<File>,
    stamp: Option<(u64, Option<SystemTime>)>,
}

impl Source {
    pub(crate) fn new(path: PathBuf, agent: Agent) -> Self {
        Self {
            agent,
            path,
            pinned: None,
            stamp: None,
        }
    }

    /// An observer pins the file it verified, so replacing a pathname cannot
    /// swap another conversation into an already attached view.
    pub(crate) fn pinned(path: PathBuf, agent: Agent, file: File) -> Self {
        Self {
            agent,
            path,
            pinned: Some(file),
            stamp: None,
        }
    }

    pub(crate) fn read(&mut self) -> io::Result<Option<Transcript>> {
        // Reject devices and FIFOs before opening, since opening a FIFO can
        // wait for another process indefinitely.
        if self.pinned.is_none() && !std::fs::metadata(&self.path)?.is_file() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "transcript must be a regular file",
            ));
        }
        let mut file = match &self.pinned {
            Some(file) => file.try_clone()?,
            None => File::open(&self.path)?,
        };
        let metadata = file.metadata()?;
        if !metadata.is_file() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "transcript must be a regular file",
            ));
        }
        let stamp = (metadata.len(), metadata.modified().ok());
        if stamp.1.is_some() && self.stamp == Some(stamp) {
            return Ok(None);
        }
        // ponytail: reparse at most 8 MiB only when the file changes. An
        // incremental reader is warranted if large active logs make this hot.
        let mut offset = metadata.len().saturating_sub(TAIL);
        let mut partial = false;
        if offset > 0 {
            file.seek(SeekFrom::Start(offset - 1))?;
            let mut previous = [0];
            file.read_exact(&mut previous)?;
            // Starting at a newline keeps the first complete record's ID.
            partial = previous[0] != b'\n';
        } else {
            file.seek(SeekFrom::Start(0))?;
        }
        file.seek(SeekFrom::Start(offset))?;
        let mut bytes = Vec::new();
        file.take(TAIL + 1).read_to_end(&mut bytes)?;
        if partial {
            // The first record may start before the bounded window.
            let skip = bytes
                .iter()
                .position(|byte| *byte == b'\n')
                .map_or(bytes.len(), |end| end + 1);
            offset += skip as u64;
            bytes.drain(..skip);
        }
        let transcript = parse(self.agent, &bytes, offset);
        self.stamp = Some(stamp);
        Ok(Some(transcript))
    }
}

fn limited(text: &str, limit: usize) -> String {
    let end = text.floor_char_boundary(limit.min(text.len()));
    text[..end].to_owned()
}

fn string<'a>(value: &'a Value, key: &str) -> &'a str {
    value.get(key).and_then(Value::as_str).unwrap_or_default()
}

fn text(value: &Value) -> String {
    if let Some(value) = value.as_str() {
        return limited(value, TEXT);
    }
    let Some(blocks) = value.as_array() else {
        return String::new();
    };
    let mut out = String::new();
    for block in blocks.iter().take(BLOCKS) {
        if matches!(
            string(block, "type"),
            "text" | "input_text" | "output_text" | "summary_text"
        ) {
            if !out.is_empty() {
                out.push('\n');
            }
            out.push_str(&limited(
                string(block, "text"),
                TEXT.saturating_sub(out.len()),
            ));
            if out.len() >= TEXT {
                break;
            }
        }
    }
    out
}

struct Projection {
    answered: HashMap<(String, usize), herdr_chat_protocol::QuestionAnswer>,
    transcript: Transcript,
    tools: HashMap<String, usize>,
}

impl Projection {
    fn push(&mut self, id: EntryId, kind: EntryKind) {
        self.transcript.entries.push(Entry { id, kind });
    }

    fn message(&mut self, id: EntryId, role: &str, text: String) {
        if text.is_empty() {
            return;
        }
        let kind = match role {
            "user" => EntryKind::User { text },
            "assistant" => EntryKind::Agent { text },
            "analysis" => EntryKind::Thought { text },
            _ => return,
        };
        self.push(id, kind);
    }

    fn tool(&mut self, id: EntryId, call: &str, name: &str, input: &Value) {
        if call.is_empty() || name.is_empty() || call.len() > 512 {
            return;
        }
        let input = input
            .as_str()
            .map_or_else(|| input.to_string(), str::to_owned);
        let tool = Tool {
            call_id: call.to_owned(),
            content: vec![ToolContent::Text {
                text: limited(&input, TEXT),
            }],
            kind: ToolKind::Other,
            locations: Vec::new(),
            status: ToolStatus::Pending,
            title: limited(name, 256),
        };
        self.tools
            .insert(call.to_owned(), self.transcript.entries.len());
        self.push(id, EntryKind::Tool(tool));
    }

    fn output(&mut self, call: &str, output: &Value, failed: bool) {
        let Some(index) = self.tools.get(call) else {
            return;
        };
        let EntryKind::Tool(tool) = &mut self.transcript.entries[*index].kind else {
            return;
        };
        tool.status = if failed {
            ToolStatus::Failed
        } else {
            ToolStatus::Completed
        };
        let output = text(output);
        if !output.is_empty() {
            tool.content.truncate(1);
            tool.content.push(ToolContent::Text { text: output });
        }
    }

    fn codex(&mut self, id: EntryId, record: &Value) {
        let payload = &record["payload"];
        match string(record, "type") {
            "session_meta" => {
                let session = payload
                    .get("session_id")
                    .or_else(|| payload.get("id"))
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                self.session(session, string(payload, "cwd"));
            }
            "turn_context" => {
                self.reported_option(OptionCategory::Model, "model", string(payload, "model"));
                self.reported_option(
                    OptionCategory::ThoughtLevel,
                    "thinking",
                    string(payload, "effort"),
                );
            }
            "compacted" => {
                let message = string(payload, "message");
                self.transcript.truncated |= message.len() > TEXT;
                let mut notice = notices::source("Context compacted".into(), message);
                if message.is_empty()
                    && let EntryKind::Notice { details, .. } = &mut notice
                {
                    *details = None;
                }
                self.push(id, notice);
                if let Some(answers) = payload["retained_context"]["verified_answers"].as_array() {
                    self.transcript.truncated |= answers.len() >= BLOCKS;
                    for (index, answer) in answers.iter().take(BLOCKS - 1).enumerate() {
                        if let Some(notice) = self.retained_answer(answer, "order") {
                            self.push(EntryId(id.0 + index as u64 + 1), notice);
                        }
                    }
                }
            }
            "retained_context" if string(payload, "type") == "verified_answer" => {
                if let Some(notice) = self.retained_answer(payload, "acceptance_order") {
                    self.push(id, notice);
                }
            }
            "event_msg" if string(payload, "type") == "token_count" => {
                let info = &payload["info"];
                if let (Some(size), Some(used)) = (
                    info["model_context_window"]
                        .as_u64()
                        .filter(|size| *size > 0),
                    info["last_token_usage"]["total_tokens"].as_u64(),
                ) {
                    self.transcript.usage = Some(Usage {
                        cost: None,
                        size,
                        used,
                    });
                }
            }
            "response_item" => match string(payload, "type") {
                "message" => self.codex_message(id, record),
                "reasoning" => self.message(id, "analysis", text(&payload["summary"])),
                "function_call" => self.tool(
                    id,
                    string(payload, "call_id"),
                    string(payload, "name"),
                    &payload["arguments"],
                ),
                "custom_tool_call" => self.tool(
                    id,
                    string(payload, "call_id"),
                    string(payload, "name"),
                    &payload["input"],
                ),
                "function_call_output" | "custom_tool_call_output" => {
                    self.output(string(payload, "call_id"), &payload["output"], false)
                }
                _ => {}
            },
            // event_msg often repeats response_item text. Read one canonical
            // representation, never turn historical approval events into UI.
            _ => {}
        }
    }

    fn session(&mut self, id: &str, cwd: &str) {
        if id.is_empty() {
            return;
        }
        self.transcript.session = Some(SessionInfo {
            id: limited(id, 512),
            cwd: (!cwd.is_empty()).then(|| limited(cwd, 4096)),
            title: None,
        });
    }

    fn reported_option(&mut self, category: OptionCategory, id: &str, value: &str) {
        if value.is_empty() {
            return;
        }
        let option = ConfigOption {
            category,
            description: None,
            id: id.into(),
            // Empty choices expose only the reported value, never a setter.
            kind: OptionKind::Select {
                choices: Vec::new(),
                current: limited(value, 256),
            },
            name: id.into(),
        };
        if let Some(existing) = self
            .transcript
            .options
            .iter_mut()
            .find(|item| item.id == id)
        {
            *existing = option;
        } else {
            self.transcript.options.push(option);
        }
    }

    fn claude(&mut self, base: EntryId, record: &Value) {
        let role = string(record, "type");
        if !matches!(role, "user" | "assistant") || record["isSidechain"] == true {
            return;
        }
        self.session(string(record, "sessionId"), string(record, "cwd"));
        if role == "assistant" {
            self.reported_option(
                OptionCategory::Model,
                "model",
                string(&record["message"], "model"),
            );
        }
        let content = &record["message"]["content"];
        if let Some(content) = content.as_str() {
            self.claude_message(base, record, content);
            return;
        }
        let Some(blocks) = content.as_array() else {
            return;
        };
        if blocks.len() > BLOCKS {
            self.transcript.truncated = true;
        }
        for (index, block) in blocks.iter().take(BLOCKS).enumerate() {
            let id = EntryId(base.0 + index as u64);
            match string(block, "type") {
                "text" => self.claude_message(id, record, string(block, "text")),
                "thinking" if role == "assistant" => {
                    self.message(id, "analysis", limited(string(block, "thinking"), TEXT))
                }
                "tool_use" if role == "assistant" => self.tool(
                    id,
                    string(block, "id"),
                    string(block, "name"),
                    &block["input"],
                ),
                "tool_result" if role == "user" => self.output(
                    string(block, "tool_use_id"),
                    &block["content"],
                    block["is_error"] == true,
                ),
                _ => {}
            }
        }
    }

    fn claude_message(&mut self, id: EntryId, record: &Value, content: &str) {
        if let Some(notice) = notices::project(record, content) {
            self.transcript.truncated |= content.len() > TEXT;
            self.push(id, notice);
        } else {
            self.message(id, string(record, "type"), limited(content, TEXT));
        }
    }
}

fn parse(agent: Agent, bytes: &[u8], mut offset: u64) -> Transcript {
    let mut projection = Projection {
        answered: HashMap::new(),
        transcript: Transcript {
            agent: agent.display_name().to_owned(),
            read_only: true,
            truncated: offset > 0,
            ..Transcript::default()
        },
        tools: HashMap::new(),
    };
    let mut seen = HashSet::new();
    for line in bytes.split_inclusive(|byte| *byte == b'\n') {
        let start = offset;
        offset = offset.saturating_add(line.len() as u64);
        // A writer may be halfway through its final JSON record.
        if line.last() != Some(&b'\n') {
            break;
        }
        let record_limit = if matches!(agent, Agent::Codex) {
            TAIL as usize
        } else {
            RECORD
        };
        if line.len() > record_limit {
            projection.transcript.truncated = true;
            continue;
        }
        let Ok(record) = serde_json::from_slice::<Value>(line) else {
            projection.transcript.truncated |= line.len() > RECORD;
            continue;
        };
        if line.len() > RECORD && !codex_messages::image_record(&record) {
            projection.transcript.truncated = true;
            continue;
        }
        let Some(base) = start
            .checked_add(1)
            .and_then(|offset| offset.checked_mul(BLOCKS as u64))
        else {
            break;
        };
        match agent {
            Agent::Codex => projection.codex(EntryId(base), &record),
            Agent::Claude => {
                let uuid = string(&record, "uuid");
                if uuid.is_empty() || seen.insert(limited(uuid, 512)) {
                    projection.claude(EntryId(base), &record);
                }
            }
            _ => {}
        }
    }
    let transcript = &mut projection.transcript;
    let keep = keep_from(&transcript.entries);
    if keep > 0 {
        transcript.entries.drain(..keep);
        transcript.truncated = true;
    }
    projection.transcript
}

#[cfg(test)]
mod tests;
