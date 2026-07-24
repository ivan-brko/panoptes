//! End-to-end mouse selection, driven against the real binary
//!
//! These are `#[ignore]`d: each one spawns Panoptes in a PTY, plays iTerm2 at
//! the other end, and asserts on the *system clipboard*, which is too much
//! machinery — and too much of the developer's machine — for a plain
//! `cargo test`. Run them deliberately:
//!
//! ```text
//! cargo test --test selection_e2e -- --ignored           # all of them
//! cargo test --test selection_e2e -- --ignored shell     # just one
//! ```
//!
//! Being a test rather than a loose script buys the thing the harness most
//! needs: `CARGO_BIN_EXE_panoptes` is built by Cargo before the test runs, so
//! it is impossible to validate a stale binary. That was the trap the previous
//! scratchpad version left open, since `cargo test` alone never rebuilds it.
//!
//! Needs `python3` with [`pyte`](https://pypi.org/project/pyte/) — the harness
//! renders Panoptes' output to know where on screen to click.

use std::path::PathBuf;
use std::process::Command;

/// Run one harness scenario, failing the test with its output on a non-zero exit
fn run_scenario(scenario: &str) {
    let harness = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("e2e")
        .join("drive_selection.py");
    assert!(harness.exists(), "harness missing at {}", harness.display());

    let output = Command::new("python3")
        .arg(&harness)
        .arg(scenario)
        // Cargo built this for us, so the harness can never drive a stale
        // binary the way a hand-run script could
        .env("PANOPTES_BIN", env!("CARGO_BIN_EXE_panoptes"))
        .output()
        .expect("failed to run python3 — the harness needs python3 with `pyte`");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "scenario `{scenario}` failed\n--- stdout ---\n{stdout}\n--- stderr ---\n{stderr}"
    );
    // Every check the scenario ran, so a passing run still says what it proved
    println!("{stdout}");
}

/// Drag, click, double- and triple-click, backwards and multi-row drags, edge
/// auto-scroll, a child that takes the mouse, and a session whose process died
#[test]
#[ignore = "spawns a PTY and uses the system clipboard; run with --ignored"]
fn selection_over_a_shell_session() {
    run_scenario("shell");
}

/// Codex routes wheel events before the PTY forward, unlike a shell, so its
/// ordering through `handle_mouse_event` is its own path
///
/// Needs a real, authenticated `~/.codex`.
#[test]
#[ignore = "needs an authenticated ~/.codex; run with --ignored"]
fn selection_over_a_codex_session() {
    run_scenario("codex");
}

/// With no clipboard helper that works, the copy has to leave as OSC 52 —
/// the path macOS's `pbcopy` otherwise always wins before
#[test]
#[ignore = "spawns a PTY and uses the system clipboard; run with --ignored"]
fn copying_falls_back_to_osc52() {
    run_scenario("osc52");
}
