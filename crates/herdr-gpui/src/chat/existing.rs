//! Discover the exact CLI conversation already running in a pane. Reads only
//! daemon metadata and process identity; never opens unrelated session history.

use crate::teleport::{self, Envelope, Host, ProcessInfoResult, SessionKind, SnapshotResult};
use herdr_chat_protocol::Agent;
use herdr_client::{ConnectTarget, ScriptLimits, shell_quote};
use std::{
    collections::{BTreeSet, HashMap},
    hash::{Hash, Hasher},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};

/// The client snapshot carries no agent session ids, so a new conversation in
/// the same pane is found only by probing. Probes start at `REFRESH`, back off
/// to `SETTLED` while the answer stays the same, and return to `REFRESH` when it
/// changes or the panes' reported activity does.
const REFRESH: Duration = Duration::from_secs(2);
const SETTLED: Duration = Duration::from_secs(30);
/// One unreachable probe keeps the overlays; this many in a row drop them.
const MAX_FAILURES: u8 = 2;
mod error;
pub(crate) use error::Error;

const MAX_MATCHING_PROCESSES: usize = 16;
const LIMITS: ScriptLimits = ScriptLimits {
    output: 1024 * 1024,
    idle: Duration::from_secs(10),
};

/// An answer is valid only for this particular connected daemon generation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Scope {
    pub(crate) target: ConnectTarget,
    pub(crate) boot: String,
    pub(crate) generation: (u64, u64),
}

/// The process and conversation both matter: one process may open several
/// subagent transcripts, and a pane may run another agent after this one exits.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Binding {
    pub(crate) pane_id: String,
    pub(crate) agent: Agent,
    pub(crate) session_id: String,
    pub(crate) herdr_session: Option<String>,
    pub(crate) pid: u32,
    pub(crate) shell_pid: u32,
    pub(crate) started_ticks: u64,
}

impl Hash for Binding {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.pane_id.hash(state);
        std::mem::discriminant(&self.agent).hash(state);
        self.session_id.hash(state);
        self.herdr_session.hash(state);
        self.pid.hash(state);
        self.shell_pid.hash(state);
        self.started_ticks.hash(state);
    }
}

struct Probe {
    cancelled: Arc<AtomicBool>,
    result: mpsc::Receiver<Result<Vec<Binding>, Error>>,
}

impl Drop for Probe {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Release);
    }
}

#[derive(Default)]
pub(crate) struct Existing {
    activity: u64,
    bindings: Vec<Binding>,
    candidates: Vec<String>,
    due: Option<Instant>,
    error: Option<Error>,
    failures: u8,
    interval: Duration,
    probe: Option<Probe>,
    scope: Option<Scope>,
}

impl Existing {
    pub(crate) fn bindings(&self) -> &[Binding] {
        &self.bindings
    }

    pub(crate) fn error(&self) -> Option<&Error> {
        self.error.as_ref()
    }

    /// Nonblocking. Call with only panes currently reported as Codex/Claude.
    /// Losing a pane or connection removes its overlay immediately, without
    /// waiting for a probe that was started under older metadata. `activity`
    /// fingerprints what the daemon reports about those panes, such as their
    /// statuses; a change brings the next probe forward.
    pub(crate) fn poll(
        &mut self,
        scope: Option<Scope>,
        candidates: &[String],
        activity: u64,
        now: Instant,
    ) -> bool {
        let candidates: Vec<_> = candidates.iter().take(super::MAX_CHATS).cloned().collect();
        let mut changed = false;
        if self.scope != scope || self.candidates != candidates {
            self.probe = None;
            self.due = None;
            changed = true;
            if self.scope != scope {
                self.bindings.clear();
            }
            self.scope = scope;
            self.candidates = candidates;
            self.bindings
                .retain(|binding| self.candidates.contains(&binding.pane_id));
            self.error = None;
            self.failures = 0;
            self.interval = REFRESH;
        }
        if self.activity != activity {
            self.activity = activity;
            self.interval = REFRESH;
            if self.probe.is_none() {
                self.due = None;
            }
        }
        if self.scope.is_none() || self.candidates.is_empty() {
            changed |= !self.bindings.is_empty();
            self.bindings.clear();
            return changed;
        }
        let result = self
            .probe
            .as_ref()
            .and_then(|probe| match probe.result.try_recv() {
                Ok(result) => Some(result),
                Err(mpsc::TryRecvError::Empty) => None,
                Err(mpsc::TryRecvError::Disconnected) => Some(Err(Error::Stopped)),
            });
        if let Some(result) = result {
            self.probe = None;
            match result {
                Ok(bindings) => {
                    let settled = self.bindings == bindings && self.error.is_none();
                    changed |= !settled;
                    self.interval = if settled {
                        (self.interval * 2).clamp(REFRESH, SETTLED)
                    } else {
                        REFRESH
                    };
                    self.bindings = bindings;
                    self.error = None;
                    self.failures = 0;
                }
                Err(error) => self.fail(error),
            }
            self.due = Some(now + self.interval);
            changed |= self.error.is_some();
        }
        if self.probe.is_none() && self.due.is_none_or(|due| now >= due) {
            let Some(scope) = &self.scope else {
                return changed;
            };
            match start(scope.clone(), self.candidates.clone()) {
                Ok(probe) => self.probe = Some(probe),
                Err(error) => {
                    self.fail(error);
                    self.due = Some(now + self.interval);
                    changed = true;
                }
            }
        }
        changed
    }

    /// A single unreachable probe, such as one dropped SSH round trip, keeps
    /// the overlays. Repeated failures are not evidence that the old agent
    /// still owns the pane, so they return it to its original terminal.
    fn fail(&mut self, error: Error) {
        self.failures = self.failures.saturating_add(1);
        if self.failures >= MAX_FAILURES {
            self.bindings.clear();
        }
        self.error = Some(error);
        self.interval = REFRESH;
    }
}

fn start(scope: Scope, candidates: Vec<String>) -> Result<Probe, Error> {
    let cancelled = Arc::new(AtomicBool::new(false));
    let flag = cancelled.clone();
    let (sender, result) = mpsc::sync_channel(1);
    std::thread::Builder::new()
        .name("herdr-existing-agent".into())
        .spawn(move || {
            let _ = sender.send(inspect(&scope, &candidates, &flag));
        })
        .map_err(Error::Worker)?;
    Ok(Probe { cancelled, result })
}

fn query(host: &Host, body: &str, cancelled: &AtomicBool) -> Result<Vec<u8>, Error> {
    let mut output = Vec::new();
    host.stream(body, std::io::empty(), &mut output, LIMITS, cancelled)
        .map_err(Error::Query)?;
    Ok(output)
}

fn snapshot(host: &Host, cancelled: &AtomicBool) -> Result<SnapshotResult, Error> {
    let bytes = query(host, "herdr_cli api snapshot\n", cancelled)?;
    serde_json::from_slice::<Envelope<SnapshotResult>>(&bytes)
        .map(|answer| answer.result)
        .map_err(Error::Answer)
}

fn inspect(
    scope: &Scope,
    candidates: &[String],
    cancelled: &AtomicBool,
) -> Result<Vec<Binding>, Error> {
    let host = teleport::host_for(&scope.target)?;
    inspect_host(&host, scope, candidates, cancelled)
}

fn inspect_host(
    host: &Host,
    scope: &Scope,
    candidates: &[String],
    cancelled: &AtomicBool,
) -> Result<Vec<Binding>, Error> {
    let first = snapshot(host, cancelled)?;
    let mut selected = Vec::new();
    for pane in &first.snapshot.panes {
        if !candidates.contains(&pane.pane_id) {
            continue;
        }
        let Some(session) = &pane.agent_session else {
            continue;
        };
        if session.kind != SessionKind::Id || uuid::Uuid::parse_str(&session.value).is_err() {
            continue;
        }
        let agent = match (session.agent.as_str(), pane.agent.as_deref()) {
            ("codex", Some("codex")) => Agent::Codex,
            ("claude", Some("claude")) => Agent::Claude,
            _ => continue,
        };
        selected.push((pane, agent, session));
    }
    if selected.is_empty() {
        return Ok(Vec::new());
    }
    let process_script = selected.iter().map(|(pane, _, _)| {
        let pane = shell_quote(&pane.pane_id);
        format!("printf '\\036%s\\n' {pane}\nherdr_cli pane process-info --pane {pane} 2>/dev/null || :\n")
    }).collect::<String>();
    let bytes = query(host, &process_script, cancelled)?;
    let infos: HashMap<_, _> = sections(&bytes)
        .filter_map(|(pane, json)| {
            let info = serde_json::from_str::<Envelope<ProcessInfoResult>>(json).ok()?;
            Some((pane, info.result.process_info))
        })
        .collect();
    let pids: BTreeSet<_> = selected
        .iter()
        .filter_map(|(pane, agent, _)| {
            let info = infos.get(pane.pane_id.as_str())?;
            let matches: Vec<_> = info
                .foreground_processes
                .iter()
                .filter(|process| is_agent(*agent, &process.argv))
                .take(MAX_MATCHING_PROCESSES + 1)
                .map(|process| process.pid)
                .collect();
            (matches.len() <= MAX_MATCHING_PROCESSES).then_some(matches)
        })
        .flatten()
        .collect();
    if pids.is_empty() {
        return Ok(Vec::new());
    }
    // Linux is the supported host for this observer. Read only known pids;
    // globbing stays disabled because a process name may contain metacharacters.
    let mut script = String::from("set -f\n[ \"$(uname -s)\" = Linux ] || exit 1\n");
    for pid in pids {
        script.push_str(&format!(
            "printf '\\036%s\\n' {pid}\nif [ -r /proc/{pid}/stat ]; then\n  IFS= read -r stat < /proc/{pid}/stat || :\n  fields=${{stat##*) }}\n  set -- $fields\n  if [ \"$#\" -ge 20 ]; then printf '%s %s %s\\n' \"$2\" \"$3\" \"${{20}}\"; fi\nfi\n",
        ));
    }
    let bytes = query(host, &script, cancelled)?;
    let processes: HashMap<_, _> = sections(&bytes)
        .filter_map(|(pid, fields)| {
            let mut fields = fields.split_whitespace();
            let process = ProcessIdentity {
                parent: fields.next()?.parse().ok()?,
                group: fields.next()?.parse().ok()?,
                started: fields.next()?.parse::<u64>().ok().filter(|n| *n > 0)?,
            };
            if fields.next().is_some() {
                return None;
            }
            Some((pid.parse::<u32>().ok()?, process))
        })
        .collect();
    let last = snapshot(host, cancelled)?;
    Ok(selected
        .iter()
        .filter_map(|(pane, agent, session)| {
            let info = infos.get(pane.pane_id.as_str())?;
            let pid = agent_pid(*agent, info, &processes)?;
            let unchanged = last.snapshot.pane(&pane.pane_id).is_some_and(|after| {
                pane.agent_session == after.agent_session && pane.agent == after.agent
            });
            if !unchanged {
                return None;
            }
            let herdr_session = match &scope.target {
                ConnectTarget::Session { name, .. } => Some(name.clone()),
                ConnectTarget::Ssh { session, .. } => Some(session.clone()),
                _ => None,
            };
            Some(Binding {
                pane_id: pane.pane_id.clone(),
                agent: *agent,
                session_id: session.value.clone(),
                herdr_session,
                pid,
                shell_pid: info.shell_pid?,
                started_ticks: processes.get(&pid)?.started,
            })
        })
        .collect())
}

#[derive(Clone, Copy)]
struct ProcessIdentity {
    parent: u32,
    group: u32,
    started: u64,
}

/// npm launchers remain in the foreground alongside their native child. Pick
/// that child only when all other matches are verified launcher ancestors.
fn agent_pid(
    agent: Agent,
    info: &teleport::ProcessInfo,
    processes: &HashMap<u32, ProcessIdentity>,
) -> Option<u32> {
    let group = info.foreground_process_group_id?;
    let matching: Vec<_> = info
        .foreground_processes
        .iter()
        .filter(|process| is_agent(agent, &process.argv))
        .take(MAX_MATCHING_PROCESSES + 1)
        .collect();
    if matching.is_empty()
        || matching.len() > MAX_MATCHING_PROCESSES
        || matching.iter().any(|process| {
            processes
                .get(&process.pid)
                .is_none_or(|stat| stat.group != group)
        })
    {
        return None;
    }
    if matching.len() == 1 {
        return Some(matching[0].pid);
    }
    let mut native = matching.iter().filter(|process| {
        process.argv.first().and_then(|arg| arg.rsplit('/').next()) == Some(agent.name())
    });
    let pid = native.next()?.pid;
    if native.next().is_some() {
        return None;
    }
    for wrapper in matching.iter().filter(|process| process.pid != pid) {
        let mut child = pid;
        let ancestor = (0..matching.len()).any(|_| {
            let Some(stat) = processes.get(&child) else {
                return false;
            };
            let Some(parent) = processes.get(&stat.parent) else {
                return false;
            };
            if parent.started > stat.started
                || parent.group != group
                || !matching.iter().any(|process| process.pid == stat.parent)
            {
                return false;
            }
            child = stat.parent;
            child == wrapper.pid
        });
        if !ancestor {
            return None;
        }
    }
    Some(pid)
}

fn sections(bytes: &[u8]) -> impl Iterator<Item = (&str, &str)> {
    std::str::from_utf8(bytes)
        .unwrap_or_default()
        .split('\u{1e}')
        .filter_map(|part| part.split_once('\n'))
}

fn is_agent(agent: Agent, argv: &[String]) -> bool {
    let Some(program) = argv.first().and_then(|arg| arg.rsplit('/').next()) else {
        return false;
    };
    if program == agent.name() {
        return true;
    }
    if !matches!(program, "node" | "nodejs" | "bun") {
        return false;
    }
    argv.get(1).is_some_and(|script| match agent {
        Agent::Claude => {
            script.ends_with("/@anthropic-ai/claude-code/cli.js") || script.ends_with("/claude")
        }
        Agent::Codex => {
            script.ends_with("/@openai/codex/bin/codex.js") || script.ends_with("/codex")
        }
        _ => false,
    })
}

#[cfg(test)]
mod tests;
