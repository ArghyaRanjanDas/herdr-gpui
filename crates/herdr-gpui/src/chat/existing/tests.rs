#![allow(clippy::unwrap_used)]
use super::*;

fn scope() -> Scope {
    Scope {
        target: ConnectTarget::Local,
        boot: "boot-a".into(),
        generation: (1, 1),
    }
}

fn binding() -> Binding {
    Binding {
        pane_id: "w1:p1".into(),
        agent: Agent::Codex,
        session_id: "12345678-1234-4123-8123-123456789abc".into(),
        herdr_session: None,
        pid: 21,
        shell_pid: 20,
        started_ticks: 123,
    }
}

#[test]
fn a_shell_return_cancels_a_pending_lookup_and_drops_the_overlay() {
    let flag = Arc::new(AtomicBool::new(false));
    let (_sender, receiver) = mpsc::sync_channel(1);
    let mut state = Existing {
        scope: Some(scope()),
        bindings: vec![binding()],
        candidates: vec!["w1:p1".into()],
        probe: Some(Probe {
            cancelled: flag.clone(),
            result: receiver,
        }),
        ..Default::default()
    };
    assert!(state.poll(Some(scope()), &[], Instant::now()));
    assert!(state.bindings().is_empty());
    assert!(flag.load(Ordering::Acquire));
}

#[test]
fn an_old_daemon_answer_is_not_applied_to_another_connection() {
    let (sender, receiver) = mpsc::sync_channel(1);
    sender.send(Ok(vec![binding()])).unwrap();
    let mut state = Existing {
        scope: Some(scope()),
        candidates: vec!["w1:p1".into()],
        probe: Some(Probe {
            cancelled: Arc::new(AtomicBool::new(false)),
            result: receiver,
        }),
        ..Default::default()
    };
    assert!(state.poll(None, &[], Instant::now()));
    assert!(state.bindings().is_empty());
    assert!(state.scope.is_none());
}

#[test]
fn failed_queries_do_not_keep_old_sessions_attached() {
    let (sender, receiver) = mpsc::sync_channel(1);
    sender.send(Err(Error::Stopped)).unwrap();
    let mut state = Existing {
        scope: Some(scope()),
        bindings: vec![binding()],
        candidates: vec!["w1:p1".into()],
        probe: Some(Probe {
            cancelled: Arc::new(AtomicBool::new(false)),
            result: receiver,
        }),
        ..Default::default()
    };
    assert!(state.poll(Some(scope()), &["w1:p1".into()], Instant::now()));
    assert!(state.bindings().is_empty());
    assert!(matches!(state.error(), Some(Error::Stopped)));
}

#[test]
fn detects_actual_agent_programs_not_arguments_to_other_programs() {
    let args = |values: &[&str]| {
        values
            .iter()
            .map(|value| (*value).into())
            .collect::<Vec<_>>()
    };
    assert!(is_agent(
        Agent::Codex,
        &args(&["/usr/local/bin/codex", "resume"])
    ));
    assert!(is_agent(
        Agent::Claude,
        &args(&["node", "/opt/node_modules/@anthropic-ai/claude-code/cli.js"])
    ));
    assert!(!is_agent(Agent::Codex, &args(&["echo", "codex"])));
    assert!(!is_agent(Agent::Codex, &args(&["bash", "-c", "codex"])));
    assert!(!is_agent(Agent::Codex, &args(&["claude"])));
}

fn process_info(rows: &[(u32, &[&str])]) -> teleport::ProcessInfo {
    serde_json::from_value(serde_json::json!({
        "shell_pid":1,"foreground_process_group_id":10,
        "foreground_processes":rows.iter().map(|(pid, argv)| serde_json::json!({
            "pid":pid,"argv":argv,
        })).collect::<Vec<_>>(),
    }))
    .unwrap()
}

fn process_identities(rows: &[(u32, u32)]) -> HashMap<u32, ProcessIdentity> {
    rows.iter()
        .map(|&(pid, parent)| {
            (
                pid,
                ProcessIdentity {
                    parent,
                    group: 10,
                    started: u64::from(pid),
                },
            )
        })
        .collect()
}

#[test]
fn npm_wrapper_and_native_child_resolve_to_the_native_agent() {
    let info = process_info(&[
        (10, &["node", "/bin/codex"]),
        (20, &["/native/codex", "resume"]),
    ]);
    assert_eq!(
        agent_pid(
            Agent::Codex,
            &info,
            &process_identities(&[(10, 1), (20, 10)])
        ),
        Some(20)
    );
}

#[test]
fn only_verified_launcher_ancestors_can_accompany_a_native_agent() {
    let info = process_info(&[(10, &["node", "/bin/codex"]), (20, &["codex"])]);
    assert_eq!(
        agent_pid(
            Agent::Codex,
            &info,
            &process_identities(&[(10, 1), (20, 1)])
        ),
        None,
        "an independent matching launcher is ambiguous"
    );
    assert_eq!(
        agent_pid(Agent::Codex, &info, &process_identities(&[(20, 10)])),
        None,
        "a vanished launcher cannot establish ancestry"
    );
    let mut identities = process_identities(&[(10, 1), (20, 10)]);
    identities.get_mut(&10).unwrap().started = 99;
    assert_eq!(
        agent_pid(Agent::Codex, &info, &identities),
        None,
        "PID reuse cannot make a new parent older than its child"
    );
    identities.get_mut(&10).unwrap().started = 10;
    identities.get_mut(&20).unwrap().group = 50;
    assert_eq!(
        agent_pid(Agent::Codex, &info, &identities),
        None,
        "a changed process group invalidates the snapshot"
    );
}

#[test]
fn two_native_agents_remain_ambiguous_even_when_related() {
    let info = process_info(&[(10, &["codex"]), (20, &["codex"])]);
    assert_eq!(
        agent_pid(
            Agent::Codex,
            &info,
            &process_identities(&[(10, 1), (20, 10)])
        ),
        None
    );
}

#[test]
fn a_chain_of_known_launchers_is_checked_without_unbounded_parent_walks() {
    let info = process_info(&[
        (10, &["node", "/bin/codex"]),
        (20, &["bun", "/bin/codex"]),
        (30, &["codex"]),
    ]);
    assert_eq!(
        agent_pid(
            Agent::Codex,
            &info,
            &process_identities(&[(10, 1), (20, 10), (30, 20)])
        ),
        Some(30)
    );
    let info = process_info(&[(10, &["node", "/bin/claude"])]);
    assert_eq!(
        agent_pid(Agent::Claude, &info, &process_identities(&[(10, 1)])),
        Some(10)
    );
}

#[cfg(target_os = "linux")]
mod host {
    use super::*;
    use std::{fs, os::unix::fs::PermissionsExt};

    fn fixture(session_changes: bool, shell: bool) -> (tempfile::TempDir, Host) {
        let temp = tempfile::tempdir().unwrap();
        let bin = temp.path().join(".local/bin");
        fs::create_dir_all(&bin).unwrap();
        let script = bin.join("herdr");
        fs::write(
            &script,
            r#"#!/bin/sh
case "$1 $2" in
"api snapshot")
  if [ -e "$HOME/queried" ]; then cat "$HOME/after.json"; else
    : > "$HOME/queried"
    cat "$HOME/before.json"
  fi ;;
"pane process-info") cat "$HOME/process.json" ;;
*) exit 99 ;;
esac
"#,
        )
        .unwrap();
        fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
        let snapshot = |session: &str| {
            serde_json::json!({"result":{"snapshot":{
            "workspaces":[],"tabs":[],"panes":[{
                "pane_id":"w1:p1","tab_id":"w1:t1","agent":"codex",
                "agent_session":{"agent":"codex","kind":"id","source":"herdr:codex","value":session}
            }]
        }}}).to_string()
        };
        fs::write(
            temp.path().join("before.json"),
            snapshot(&binding().session_id),
        )
        .unwrap();
        let after = if session_changes {
            "87654321-4321-4321-8321-cba987654321"
        } else {
            &binding().session_id
        };
        fs::write(temp.path().join("after.json"), snapshot(after)).unwrap();
        let pid = std::process::id();
        let stat = fs::read_to_string(format!("/proc/{pid}/stat")).unwrap();
        let group: u32 = stat
            .rsplit_once(')')
            .unwrap()
            .1
            .split_whitespace()
            .nth(2)
            .unwrap()
            .parse()
            .unwrap();
        fs::write(temp.path().join("process.json"), serde_json::json!({"result":{"process_info":{
            "shell_pid": if shell {pid} else {pid - 1}, "foreground_process_group_id":group,
            "foreground_processes":[{"pid":pid,"argv":if shell {vec!["bash"]} else {vec!["codex","resume"]}}]
        }}}).to_string()).unwrap();
        let mut host = Host::new(&ConnectTarget::Local).unwrap();
        host.env = vec![("HOME".into(), temp.path().to_string_lossy().into_owned())];
        (temp, host)
    }

    #[test]
    fn discovers_the_exact_existing_session_and_process_start_time() {
        let (_temp, host) = fixture(false, false);
        let result =
            inspect_host(&host, &scope(), &["w1:p1".into()], &AtomicBool::new(false)).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].session_id, binding().session_id);
        assert_eq!(result[0].pid, std::process::id());
        assert!(result[0].started_ticks > 0);
    }

    #[test]
    fn refuses_a_session_changed_during_inspection() {
        let (_temp, host) = fixture(true, false);
        let result =
            inspect_host(&host, &scope(), &["w1:p1".into()], &AtomicBool::new(false)).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn stale_agent_metadata_cannot_attach_to_a_shell() {
        let (_temp, host) = fixture(false, true);
        let result =
            inspect_host(&host, &scope(), &["w1:p1".into()], &AtomicBool::new(false)).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn ssh_session_names_remain_in_the_identity() {
        let (_temp, host) = fixture(false, false);
        let mut scope = scope();
        scope.target = ConnectTarget::Ssh {
            target: "synthetic-host".into(),
            session: "test-session".into(),
        };
        let result =
            inspect_host(&host, &scope, &["w1:p1".into()], &AtomicBool::new(false)).unwrap();
        assert_eq!(result[0].herdr_session.as_deref(), Some("test-session"));
    }
}
