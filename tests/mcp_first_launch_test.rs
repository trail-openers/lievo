// End-to-end first-launch test for `lievo mcp` (issue #872): spawn the real
// binary as a child process on a fresh temp git repo with an empty DB (HOME
// and LIEVO_DB both in temp dirs, so the child's resolved data dir is under
// the temp dir and the real ~/.lievo is never touched), speak the MCP
// JSON-RPC protocol over stdio, and verify:
//
// 1. `initialize` is answered (the server resolved and auto-registered the
//    repo and started the core index in the background — issue #864).
// 2. `tools/list` returns exactly `lievo_explore` (LIEVO_MCP_TOOLS unset).
// 3. `tools/call lievo_explore` returns either the success-shaped
//    "indexing in progress" response or real index results; polling with a
//    bounded deadline, real results (non-empty `symbols`) arrive within
//    the deadline — the core index of a tiny repo takes seconds.
//
// The `notifications/initialized` notification must be sent after
// `initialize` and before `tools/list` (rmcp requires it).

use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::{Duration, Instant};

/// Drop guard: kills the child `lievo mcp` process when the guard is
/// dropped — including on a test panic, so no live child can outlive the
/// test (and hold its temp dirs open) on any failure path.
struct ChildGuard {
    child: Option<Child>,
    stderr_path: PathBuf,
}

impl Drop for ChildGuard {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

/// Last ~50 lines of the child's stderr capture file — surfaced in panic
/// messages so any failure path (timeout or assertion) shows what the
/// server logged.
fn tail_stderr(path: &std::path::Path) -> String {
    std::fs::read_to_string(path)
        .ok()
        .map(|raw| {
            raw.lines()
                .rev()
                .take(50)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default()
}

/// Panicking message builder that appends the child's stderr tail to
/// `msg`.
fn with_stderr(path: &std::path::Path, msg: &str) -> String {
    let tail = tail_stderr(path);
    if tail.is_empty() {
        format!("{msg} (child stderr: empty)")
    } else {
        format!("{msg}\n--- child stderr (last ~50 lines) ---\n{tail}")
    }
}

fn lievo_bin() -> PathBuf {
    let exe = std::env::current_exe().expect("current_exe");
    let debug = exe.parent().and_then(|p| p.parent()).expect("debug dir");
    debug.join("lievo")
}

/// Fresh tempdir git repo with one file and one commit on main.
fn git_repo() -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::TempDir::new().unwrap();
    let root = dir.path().to_path_buf();
    std::fs::write(root.join("hello.rs"), "fn hello() -> u32 {\n    42\n}\n").unwrap();
    let repo = git2::Repository::init(&root).unwrap();
    repo.set_head("refs/heads/main").unwrap();
    let mut index = repo.index().unwrap();
    index
        .add_all(["**"], git2::IndexAddOption::DEFAULT, None)
        .unwrap();
    index.write().unwrap();
    let tree_oid = index.write_tree().unwrap();
    let tree = repo.find_tree(tree_oid).unwrap();
    let sig = git2::Signature::now("lievo-test", "lievo@test").unwrap();
    repo.commit(Some("refs/heads/main"), &sig, &sig, "first", &tree, &[])
        .unwrap();
    (dir, root)
}

/// Spawn `lievo mcp` with cwd in the git repo and HOME + LIEVO_DB in temp
/// dirs (never the real ~/.lievo). `LIEVO_DB` is a full file path inside the
/// temp dir (SqliteStorage::open treats it as a file path, not a directory).
/// The child speaks newline-delimited
// JSON-RPC on stdout; a reader thread ships lines to a channel so the test
/// can interleave requests and responses. The returned `ChildGuard` kills
/// the child on drop (any path, including test panic).
fn spawn_mcp(
    repo: &std::path::Path,
) -> (
    tempfile::TempDir,
    ChildGuard,
    std::io::BufWriter<std::process::ChildStdin>,
    Receiver<String>,
) {
    let base = tempfile::tempdir().unwrap();
    let home = base.path().join("home");
    let db = base.path().join("lievo.db");
    let stderr_path = base.path().join("mcp-stderr.log");
    let _ = std::fs::create_dir_all(&home);

    let mut cmd = Command::new(lievo_bin());
    cmd.arg("mcp")
        .current_dir(repo)
        .env("HOME", &home)
        .env("LIEVO_DB", &db)
        .env_remove("LIEVO_PROJECT_DIR")
        .env_remove("CLAUDE_PROJECT_DIR")
        .env_remove("LIEVO_NO_REFRESH")
        .env_remove("LIEVO_MCP_TOOLS")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(std::process::Stdio::from(
            std::fs::File::create(&stderr_path).expect("create child stderr capture file"),
        ));
    let mut child = cmd.spawn().expect("failed to spawn lievo mcp");
    let stdin = child.stdin.take().expect("child stdin piped");
    let stdout = child.stdout.take().expect("child stdout piped");
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let mut reader = BufReader::new(stdout);
        let mut line = String::new();
        loop {
            line.clear();
            match reader.read_line(&mut line) {
                Ok(0) => break,
                Ok(_) => {
                    let _ = tx.send(line.trim().to_string());
                }
                Err(_) => break,
            }
        }
    });
    (
        base,
        ChildGuard {
            child: Some(child),
            stderr_path,
        },
        std::io::BufWriter::new(stdin),
        rx,
    )
}

/// Send one newline-delimited JSON-RPC message to the child.
fn send(stdin: &mut impl Write, msg: &serde_json::Value) {
    let mut bytes = serde_json::to_string(msg).unwrap();
    bytes.push('\n');
    stdin.write_all(bytes.as_bytes()).unwrap();
    stdin.flush().unwrap();
}

/// Read child stdout lines until the message for `id` arrives; server
/// notifications and other responses are skipped. `recv_timeout` owns the
/// timeout (single consistent message naming the request id and elapsed
/// time); `stderr_path` is included in the message so a dead or wedged
/// server is diagnosable.
fn next_response(
    rx: &Receiver<String>,
    id: i32,
    deadline: Instant,
    stderr_path: &std::path::Path,
) -> serde_json::Value {
    let start = Instant::now();
    let line = rx.recv_timeout(deadline - start).unwrap_or_else(|_| {
        panic!(
            "{}",
            with_stderr(
                stderr_path,
                &format!(
                    "timed out after {:.1}s waiting for MCP response to request id {id}",
                    start.elapsed().as_secs_f64()
                )
            )
        )
    });
    let v: serde_json::Value = serde_json::from_str(&line).expect("valid JSON: {line}");
    if v.get("id").and_then(|x| x.as_i64()) == Some(id as i64) {
        return v;
    }
    // Not our response (a server notification or a later response) — loop
    // back and keep waiting within the same deadline.
    next_response(rx, id, deadline, stderr_path)
}

/// `tools/call lievo_explore` once; returns the text content of the
/// success-shaped result.
fn call_explore(
    stdin: &mut impl Write,
    rx: &Receiver<String>,
    id: i64,
    deadline: Instant,
    stderr_path: &std::path::Path,
) -> String {
    send(
        stdin,
        &serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "tools/call",
            "params": {
                "name": "lievo_explore",
                "arguments": { "query": "hello" }
            }
        }),
    );
    let resp = next_response(rx, id as i32, deadline, stderr_path);
    let result = resp
        .get("result")
        .unwrap_or_else(|| panic!("tools/call failed: {resp}"));
    result
        .get("content")
        .and_then(|c| c.as_array())
        .and_then(|arr| arr.first())
        .and_then(|b| b.get("text"))
        .and_then(|t| t.as_str())
        .expect("tools/call result must carry a text content block")
        .to_string()
}

/// Poll `lievo_explore` every 2s until the response JSON has a non-empty
/// `symbols` array (real index results); the indexing-in-progress response
/// carries `"symbols": []`. Returns the final text and attempt count.
fn call_explore_until_results(
    stdin: &mut impl Write,
    rx: &Receiver<String>,
    deadline: Duration,
    stderr_path: &std::path::Path,
) -> (String, u32) {
    let start = Instant::now();
    let mut id = 100_i64;
    let mut attempts = 0_u32;
    loop {
        id += 1;
        attempts += 1;
        let text = call_explore(stdin, rx, id, start + deadline, stderr_path);
        let parsed: serde_json::Value =
            serde_json::from_str(&text).unwrap_or(serde_json::Value::Null);
        let has_symbols = parsed
            .get("symbols")
            .and_then(|s| s.as_array())
            .is_some_and(|a| !a.is_empty());
        if has_symbols || start.elapsed() >= deadline {
            return (text, attempts);
        }
        std::thread::sleep(Duration::from_secs(2));
    }
}

#[test]
fn mcp_first_launch_serves_explore_and_indexes_in_background() {
    let (_repo_dir, repo) = git_repo();
    let (_base, mut child_guard, mut stdin, rx) = spawn_mcp(&repo);
    let overall_deadline = Instant::now() + Duration::from_secs(120);
    let stderr_path = child_guard.stderr_path.clone();

    // 1. Initialize handshake: send initialize, then notifications/initialized
    //    (rmcp requires the notification before tools/list).
    send(
        &mut stdin,
        &serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": { "name": "lievo-first-launch-test", "version": "0.1.0" }
            }
        }),
    );
    let init = next_response(&rx, 1, overall_deadline, &stderr_path);
    assert!(
        init.get("result")
            .and_then(|r| r.get("serverInfo"))
            .is_some(),
        "{}",
        with_stderr(
            &stderr_path,
            &format!("initialize must return serverInfo, got {init}")
        )
    );
    send(
        &mut stdin,
        &serde_json::json!({
            "jsonrpc": "2.0",
            "method": "notifications/initialized"
        }),
    );

    // 2. tools/list must return exactly lievo_explore (allowlist unset).
    send(
        &mut stdin,
        &serde_json::json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/list",
            "params": {}
        }),
    );
    let list = next_response(&rx, 2, overall_deadline, &stderr_path);
    let tool_names: Vec<&str> = list
        .get("result")
        .and_then(|r| r.get("tools"))
        .and_then(|t| t.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|t| t.get("name").and_then(|n| n.as_str()))
                .collect()
        })
        .unwrap_or_default();
    assert_eq!(
        tool_names,
        vec!["lievo_explore"],
        "{}",
        with_stderr(
            &stderr_path,
            &format!("exactly lievo_explore must be listed, got {tool_names:?}")
        )
    );

    // 3. tools/call lievo_explore: indexing-in-progress first, then real
    //    results — poll until the core index (started by the child on serve
    //    start) completes; a tiny repo indexes in seconds, 120s is a bound.
    let (final_text, attempts) =
        call_explore_until_results(&mut stdin, &rx, Duration::from_secs(120), &stderr_path);
    let parsed: serde_json::Value = serde_json::from_str(&final_text)
        .unwrap_or_else(|_| panic!("explore response must be JSON: {final_text}"));
    assert!(
        parsed
            .get("symbols")
            .and_then(|s| s.as_array())
            .is_some_and(|a| !a.is_empty()),
        "{}",
        with_stderr(
            &stderr_path,
            &format!(
                "lievo_explore must return real index results within the deadline \
                 (attempts={attempts}); last response: {final_text}"
            )
        )
    );
    assert!(
        !parsed
            .get("indexing")
            .and_then(|i| i.as_bool())
            .unwrap_or(false),
        "{}",
        with_stderr(
            &stderr_path,
            &format!("final response must not still be the in-progress shape: {final_text}")
        )
    );

    // Clean shutdown: closing the protocol channel (drop stdin) ends the
    // child's stdio stream. Wait briefly for the child to exit on its own;
    // whatever the outcome, drop the guard last — its Drop kills and waits
    // on any child still alive, so no path (success, assertion failure, or
    // timeout panic) leaks a live child.
    drop(stdin);
    if let Some(mut child) = child_guard.child.take() {
        let mut waited = Duration::from_millis(0);
        loop {
            match child.try_wait() {
                Ok(Some(_)) => break,
                Ok(None) => {
                    if waited >= Duration::from_secs(10) {
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(100));
                    waited += Duration::from_millis(100);
                }
                Err(_) => break,
            }
        }
    }
    drop(child_guard);
}
