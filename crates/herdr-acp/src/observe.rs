//! A view of a verified, already-running CLI. Its original terminal owns all
//! input. This child only projects history; closing the GUI stops this child,
//! never the agent. No persistent sidecar, replacement session, or socket.

mod identity;
#[cfg(test)]
mod tests;

use crate::{
    agents::Agent,
    mirror::Mirror,
    session::{Attach, ConnId, Input},
};
use anyhow::{Context as _, bail};
use crossbeam_channel::{Receiver, bounded};
use herdr_chat_protocol::{
    ByeReason, ClientMessage, ServerMessage, SessionInfo, VERSION, read_frame, write_frame,
};
use std::{
    io, thread,
    time::{Duration, Instant},
};

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Options {
    pub agent: Agent,
    pub session: String,
    pub pid: u32,
    pub started: u64,
    pub pane: Option<String>,
}

const POLL: Duration = Duration::from_secs(1);
const CONN: ConnId = ConnId(1);

/// Reading and parsing stdin is bounded and separate from polling the file.
/// EOF tears down the observer even when the agent itself is idle.
fn inputs() -> io::Result<Receiver<ClientMessage>> {
    let (sender, receiver) = bounded(32);
    thread::Builder::new()
        .name("chat-observer-input".into())
        .spawn(move || {
            let mut stdin = io::stdin().lock();
            while let Ok(Some(message)) = read_frame(&mut stdin) {
                if sender.send(message).is_err() {
                    break;
                }
            }
        })?;
    Ok(receiver)
}

pub(crate) fn run(options: Options) -> anyhow::Result<()> {
    let receiver = inputs()?;
    let greeting = receiver.recv_timeout(Duration::from_secs(5))?;
    if !matches!(
        greeting,
        ClientMessage::Hello {
            version: VERSION,
            ..
        }
    ) {
        bail!("the observer requires a compatible chat greeting");
    }
    let identity = identity::Identity::open(&options)?;
    let mut source = identity.source()?;
    let initial = source
        .read()?
        .context("the running agent's transcript is unavailable")?;
    identity.check()?;
    let session = SessionInfo {
        id: options.session.clone(),
        cwd: identity.cwd.clone(),
        title: None,
    };
    let project = |mut transcript: herdr_chat_protocol::Transcript| {
        // The bounded tail need not include the original session header.
        transcript.session = Some(session.clone());
        transcript
    };
    let mut mirror = Mirror::new(project(initial), options.pane);
    let (outbox, outgoing) = bounded(4096);
    mirror.input(Input::Attach(Attach {
        conn: CONN,
        local: false,
        outbox,
        since: None,
    }));
    let mut stdout = io::stdout().lock();
    let mut next = Instant::now() + POLL;
    loop {
        for message in outgoing.try_iter() {
            write_frame(&mut stdout, &message)?;
        }
        match receiver.recv_timeout(next.saturating_duration_since(Instant::now())) {
            Ok(ClientMessage::Command { id, command }) => {
                // Historical permission requests are never actionable here.
                mirror.input(Input::Command {
                    conn: CONN,
                    id,
                    command,
                });
            }
            Ok(ClientMessage::Hello { .. }) => bail!("duplicate observer greeting"),
            Err(crossbeam_channel::RecvTimeoutError::Disconnected) => break,
            Err(crossbeam_channel::RecvTimeoutError::Timeout) => {}
        }
        if Instant::now() >= next {
            if !identity.process_matches()? {
                write_frame(
                    &mut stdout,
                    &ServerMessage::Bye {
                        reason: ByeReason::Exiting,
                    },
                )?;
                break;
            }
            // Rotation or a temporarily incomplete header closes only this
            // observer; the client may reconnect to the same live agent.
            identity.check()?;
            if let Some(transcript) = source.read()? {
                identity.check()?;
                mirror.update(project(transcript));
            }
            next = Instant::now() + POLL;
        }
    }
    Ok(())
}
