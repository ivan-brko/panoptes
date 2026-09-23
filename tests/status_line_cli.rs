//! The `panoptes status-line` subcommand, run as the real binary
//!
//! Claude's status line runs it on every refresh for users with no status
//! line of their own, so it must print the compact line, stay quiet on input
//! it cannot read, and never fail.

#![cfg(unix)]

use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

use panoptes::agent::ClaudeCodeAdapter;

const FIXTURE: &str = include_str!("fixtures/claude_status_line.json");

/// Run `program args...` with `stdin`, in an empty environment apart from
/// what is given, returning (stdout, success)
fn run(program: &Path, args: &[&str], env: &[(&str, &str)], stdin: &str) -> (String, bool) {
    let mut cmd = Command::new(program);
    cmd.args(args)
        .env_clear()
        .env("TZ", "UTC")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    for (key, value) in env {
        cmd.env(key, value);
    }
    let mut child = cmd.spawn().expect("spawn");
    child
        .stdin
        .take()
        .unwrap()
        .write_all(stdin.as_bytes())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    (
        String::from_utf8(output.stdout).unwrap(),
        output.status.success(),
    )
}

fn panoptes() -> &'static Path {
    Path::new(env!("CARGO_BIN_EXE_panoptes"))
}

/// The compact line for the fixture, whenever the test runs
///
/// The reset part depends on the clock: it is left out once the captured
/// reset time has passed.
fn assert_fixture_line(line: &str) {
    assert!(line.starts_with("5h 21%"), "{line:?}");
    assert!(line.contains(" · wk 11% · "), "{line:?}");
    assert!(line.ends_with(" · $0.14"), "{line:?}");
    assert!(!line.contains('\n'), "{line:?}");
}

#[test]
fn test_status_line_subcommand_prints_the_compact_line() {
    let (stdout, success) = run(panoptes(), &["status-line"], &[], FIXTURE);
    assert!(success);
    assert_fixture_line(&stdout);
}

#[test]
fn test_status_line_subcommand_is_silent_on_bad_input() {
    for input in ["", "not json", "{\"rate_limits\":", "[1,2,3]", "{}"] {
        let (stdout, success) = run(panoptes(), &["status-line"], &[], input);
        assert!(success, "for {input:?}");
        assert_eq!(stdout, "", "for {input:?}");
    }
}

#[test]
fn test_wrapper_default_path_prints_the_compact_line() {
    let dir = tempfile::TempDir::new().unwrap();
    let script = dir
        .path()
        .join("it's a $dir")
        .join("panoptes-statusline.sh");
    std::fs::create_dir_all(script.parent().unwrap()).unwrap();
    std::fs::write(
        &script,
        ClaudeCodeAdapter::generate_status_line_script("http://127.0.0.1:1/hook", Some(panoptes())),
    )
    .unwrap();

    // No session, so nothing is posted: only the line is under test here
    let (stdout, success) = run(
        Path::new("/bin/sh"),
        &[script.to_str().unwrap()],
        &[("PATH", "/usr/bin:/bin")],
        FIXTURE,
    );
    assert!(success);
    assert_fixture_line(&stdout);
}
