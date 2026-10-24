//! Resolve only the exact session advertised by the daemon. In particular a
//! Codex process can hold several child-agent rollouts: newest-file guessing
//! would silently show the wrong conversation.

use super::Options;
use crate::{agents::Agent, mirror::transcript::Source};
use anyhow::{Context as _, bail, ensure};
use serde_json::Value;
use std::{
    fs::{self, File},
    io::{self, BufRead, BufReader, Read, Seek, SeekFrom},
    path::{Path, PathBuf},
};

const HEADER: u64 = 512 * 1024;
const MAX_FDS: usize = 2048;
const MAX_PROJECTS: usize = 512;
const MAX_DAY_ENTRIES: usize = 4096;

pub(super) struct Identity {
    pub(super) path: PathBuf,
    pub(super) cwd: Option<String>,
    process: PathBuf,
    started: u64,
    file: File,
    agent: Agent,
    session: String,
}

impl Identity {
    pub(super) fn open(options: &Options) -> anyhow::Result<Self> {
        ensure!(
            cfg!(target_os = "linux"),
            "existing terminal observation requires Linux process metadata"
        );
        Self::at(options, Path::new("/proc"), std::env::home_dir().as_deref())
    }

    pub(super) fn at(
        options: &Options,
        proc_root: &Path,
        home: Option<&Path>,
    ) -> anyhow::Result<Self> {
        let process = proc_root.join(options.pid.to_string());
        ensure!(
            start_ticks(&process)? == options.started,
            "the original agent process has changed"
        );
        let cwd = fs::read_link(process.join("cwd")).ok();
        let mut candidates = Vec::new();
        if let Ok(entries) = fs::read_dir(process.join("fd")) {
            for entry in entries.take(MAX_FDS).flatten() {
                let Ok(path) = fs::read_link(entry.path()) else {
                    continue;
                };
                if candidate_name(&path, options) && !candidates.contains(&path) {
                    candidates.push(path);
                }
            }
        }
        // Claude opens its log only while writing. Search exact UUID names
        // within its private projects directory; never recurse over histories.
        if options.agent == Agent::Claude
            && let Some(base) = config_directory(&process, home, Agent::Claude)?
            && let Ok(projects) = fs::read_dir(base.join("projects"))
        {
            for project in projects.take(MAX_PROJECTS).flatten() {
                if !project.file_type()?.is_dir() {
                    continue;
                }
                let path = project.path().join(format!("{}.jsonl", options.session));
                if path.is_file() && !candidates.contains(&path) {
                    candidates.push(path);
                }
            }
        }
        // A daemon-backed Codex TUI has no rollout descriptor of its own.
        // Its UUIDv7 bounds the lookup to three date directories; the exact
        // filename and header still have to identify the advertised thread.
        if options.agent == Agent::Codex && candidates.is_empty() {
            candidates = codex_rollouts(&process, home, options)?;
        }
        let matches: Vec<_> = candidates
            .into_iter()
            .filter(|path| matches_session(path, options).unwrap_or(false))
            .collect();
        ensure!(
            matches.len() == 1,
            "cannot uniquely identify the running agent's transcript"
        );
        let Some(path) = matches.into_iter().next() else {
            bail!("no matching transcript")
        };
        let identity = Self {
            file: File::open(&path)?,
            agent: options.agent,
            session: options.session.clone(),
            path,
            cwd: cwd.map(|path| path.to_string_lossy().into_owned()),
            process,
            started: options.started,
        };
        identity.check()?;
        Ok(identity)
    }

    pub(super) fn check(&self) -> anyhow::Result<()> {
        ensure!(
            self.process_matches()?,
            "the original agent exited or was replaced"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            let pinned = self.file.metadata()?;
            let current = fs::metadata(&self.path)?;
            ensure!(
                (pinned.dev(), pinned.ino()) == (current.dev(), current.ino()),
                "the transcript rotated; reconnect to verify its new file"
            );
        }
        ensure!(
            matches_file(self.file.try_clone()?, self.agent, &self.session)?,
            "the pinned transcript no longer describes the original session"
        );
        Ok(())
    }

    pub(super) fn process_matches(&self) -> anyhow::Result<bool> {
        match start_ticks(&self.process) {
            Ok(ticks) => Ok(ticks == self.started),
            Err(error)
                if error
                    .downcast_ref::<io::Error>()
                    .is_some_and(|error| error.kind() == io::ErrorKind::NotFound) =>
            {
                Ok(false)
            }
            Err(error) => Err(error),
        }
    }

    pub(super) fn source(&self) -> io::Result<Source> {
        Ok(Source::pinned(
            self.path.clone(),
            self.agent,
            self.file.try_clone()?,
        ))
    }
}

/// Read only the known config-directory override from the verified process.
/// Other environment variables never leave this bounded buffer or get logged.
fn config_directory(
    process: &Path,
    home: Option<&Path>,
    agent: Agent,
) -> io::Result<Option<PathBuf>> {
    const MAX_ENVIRONMENT: u64 = 128 * 1024;
    let (variable, default): (&[u8], &str) = match agent {
        Agent::Claude => (b"CLAUDE_CONFIG_DIR=", ".claude"),
        Agent::Codex => (b"CODEX_HOME=", ".codex"),
        _ => return Ok(None),
    };
    let mut bytes = Vec::new();
    match File::open(process.join("environ")) {
        Ok(file) => {
            file.take(MAX_ENVIRONMENT + 1).read_to_end(&mut bytes)?;
            if bytes.len() > MAX_ENVIRONMENT as usize {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "process environment exceeds the observation limit",
                ));
            }
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    let custom = bytes
        .split(|byte| *byte == 0)
        .find_map(|entry| entry.strip_prefix(variable))
        .filter(|value| !value.is_empty());
    let Some(custom) = custom else {
        return Ok(home.map(|home| home.join(default)));
    };
    let custom = std::str::from_utf8(custom).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "agent config directory is not valid UTF-8",
        )
    })?;
    let path = PathBuf::from(custom);
    let path = if path.is_absolute() {
        path
    } else {
        fs::read_link(process.join("cwd"))?.join(path)
    };
    Ok(Some(path))
}

fn codex_rollouts(
    process: &Path,
    home: Option<&Path>,
    options: &Options,
) -> anyhow::Result<Vec<PathBuf>> {
    let uuid = uuid::Uuid::parse_str(&options.session)?;
    ensure!(
        uuid.get_version() == Some(uuid::Version::SortRand),
        "Codex transcript lookup without a descriptor requires a UUIDv7 session"
    );
    let (seconds, nanos) = uuid
        .get_timestamp()
        .context("Codex session has no creation timestamp")?
        .to_unix();
    let date = chrono::DateTime::from_timestamp(i64::try_from(seconds)?, nanos)
        .context("Codex session creation timestamp is out of range")?
        .date_naive();
    let Some(base) = config_directory(process, home, Agent::Codex)? else {
        return Ok(Vec::new());
    };
    let mut candidates = Vec::new();
    // Rollout filenames may use the host's local date around UTC midnight.
    for day in [date.pred_opt(), Some(date), date.succ_opt()]
        .into_iter()
        .flatten()
    {
        let directory = base
            .join("sessions")
            .join(day.format("%Y/%m/%d").to_string());
        let entries = match fs::read_dir(directory) {
            Ok(entries) => entries,
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error.into()),
        };
        for (index, entry) in entries.enumerate() {
            ensure!(
                index < MAX_DAY_ENTRIES,
                "Codex transcript directory exceeds the observation limit"
            );
            let entry = entry?;
            let path = entry.path();
            if candidate_name(&path, options) && entry.file_type()?.is_file() {
                candidates.push(path);
            }
        }
    }
    Ok(candidates)
}

fn start_ticks(process: &Path) -> anyhow::Result<u64> {
    let mut stat = String::new();
    File::open(process.join("stat"))?
        .take(8192)
        .read_to_string(&mut stat)?;
    // comm may contain spaces and parentheses; fields resume after its last ')'.
    let rest = stat.rsplit_once(')').context("invalid process stat")?.1;
    rest.split_whitespace()
        .nth(19)
        .context("missing process start time")?
        .parse()
        .context("invalid process start time")
}

fn candidate_name(path: &Path, options: &Options) -> bool {
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    match options.agent {
        Agent::Codex => {
            name.starts_with("rollout-") && name.ends_with(&format!("-{}.jsonl", options.session))
        }
        Agent::Claude => name == format!("{}.jsonl", options.session),
        _ => false,
    }
}

fn matches_session(path: &Path, options: &Options) -> io::Result<bool> {
    if !fs::metadata(path)?.is_file() {
        return Ok(false);
    }
    matches_file(File::open(path)?, options.agent, &options.session)
}

fn matches_file(mut file: File, agent: Agent, session: &str) -> io::Result<bool> {
    if !file.metadata()?.is_file() {
        return Ok(false);
    }
    file.seek(SeekFrom::Start(0))?;
    // Stop at the first session record; no reason to read an entire tail to
    // establish identity, and malformed files still have a firm byte limit.
    let reader = BufReader::new(file.take(HEADER));
    for line in reader.split(b'\n') {
        let line = line?;
        let Ok(record) = serde_json::from_slice::<Value>(&line) else {
            continue;
        };
        let id = match agent {
            Agent::Codex if record["type"] == "session_meta" => record["payload"]
                .get("id")
                .or_else(|| record["payload"].get("session_id"))
                .and_then(Value::as_str),
            Agent::Claude => record.get("sessionId").and_then(Value::as_str),
            _ => None,
        };
        if let Some(id) = id {
            return Ok(id == session);
        }
    }
    Ok(false)
}
