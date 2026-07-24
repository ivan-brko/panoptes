//! Writing text to the system clipboard
//!
//! Panoptes is a local process on the user's own machine, so the clipboard is
//! reachable directly: `pbcopy` on macOS, and the same helper's Linux
//! equivalents when one is installed. OSC 52 - asking the terminal to do it -
//! is the fallback rather than the lead, because iTerm2 ships with
//! "Applications in terminal may access clipboard" disabled, where OSC 52
//! silently does nothing at all.

use std::io::Write;
use std::process::{Command, Stdio};

use anyhow::{Context, Result};

/// Clipboard helpers to try, in order, before falling back to OSC 52
///
/// macOS always has `pbcopy`; the Linux entries are there so a Panoptes built
/// for a developer's Linux box copies too, and are simply skipped when the
/// binary is absent.
const HELPERS: &[(&str, &[&str])] = &[
    ("pbcopy", &[]),
    ("wl-copy", &[]),
    ("xclip", &["-selection", "clipboard"]),
    ("xsel", &["--clipboard", "--input"]),
];

/// Put `text` on the system clipboard
///
/// Tries the platform's clipboard helper first and falls back to OSC 52,
/// which asks the terminal itself to do it. Returns an error only when
/// everything failed, so the caller can say so rather than leaving the user
/// believing a copy happened.
pub fn copy(text: &str) -> Result<()> {
    for (program, args) in HELPERS {
        match copy_with_helper(program, args, text) {
            Ok(()) => return Ok(()),
            Err(e) => {
                tracing::debug!(program, error = %e, "Clipboard helper unavailable");
            }
        }
    }
    copy_with_osc52(text).context("no clipboard helper worked and OSC 52 failed")
}

/// Pipe `text` into an external clipboard helper
fn copy_with_helper(program: &str, args: &[&str], text: &str) -> Result<()> {
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .with_context(|| format!("failed to spawn {}", program))?;

    child
        .stdin
        .take()
        .context("clipboard helper has no stdin")?
        .write_all(text.as_bytes())
        .with_context(|| format!("failed to write to {}", program))?;

    let status = child
        .wait()
        .with_context(|| format!("failed to wait for {}", program))?;
    if !status.success() {
        anyhow::bail!("{} exited with {}", program, status);
    }
    Ok(())
}

/// Ask the terminal to set the clipboard (OSC 52)
///
/// Written to the host's stdout, the same path agent-issued OSC 52 sequences
/// are forwarded on. Terminals that have the feature disabled ignore this
/// silently, which is exactly why it is the fallback and not the lead.
fn copy_with_osc52(text: &str) -> Result<()> {
    let sequence = format!("\x1b]52;c;{}\x07", base64_encode(text.as_bytes()));
    let mut out = std::io::stdout().lock();
    out.write_all(sequence.as_bytes())
        .context("failed to write OSC 52 to the terminal")?;
    out.flush().context("failed to flush OSC 52")?;
    Ok(())
}

/// Standard base64 with padding
///
/// Hand-rolled rather than pulled in: OSC 52 payloads are the only thing
/// Panoptes encodes, and the alphabet is twenty lines.
fn base64_encode(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

    let mut out = String::with_capacity(bytes.len() * 4 / 3 + 4);
    for chunk in bytes.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = *chunk.get(1).unwrap_or(&0) as u32;
        let b2 = *chunk.get(2).unwrap_or(&0) as u32;
        let triple = (b0 << 16) | (b1 << 8) | b2;

        out.push(ALPHABET[(triple >> 18) as usize & 0x3f] as char);
        out.push(ALPHABET[(triple >> 12) as usize & 0x3f] as char);
        out.push(if chunk.len() > 1 {
            ALPHABET[(triple >> 6) as usize & 0x3f] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            ALPHABET[triple as usize & 0x3f] as char
        } else {
            '='
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_base64_matches_the_rfc_test_vectors() {
        assert_eq!(base64_encode(b""), "");
        assert_eq!(base64_encode(b"f"), "Zg==");
        assert_eq!(base64_encode(b"fo"), "Zm8=");
        assert_eq!(base64_encode(b"foo"), "Zm9v");
        assert_eq!(base64_encode(b"foob"), "Zm9vYg==");
        assert_eq!(base64_encode(b"fooba"), "Zm9vYmE=");
        assert_eq!(base64_encode(b"foobar"), "Zm9vYmFy");
    }

    /// The bytes that exercise the top of the alphabet and the `+/` pair
    #[test]
    fn test_base64_encodes_high_bytes() {
        assert_eq!(base64_encode(&[0xff, 0xff, 0xff]), "////");
        assert_eq!(base64_encode(&[0xfb, 0xef, 0xbe]), "++++");
        assert_eq!(base64_encode("héllo".as_bytes()), "aMOpbGxv");
    }

    /// A missing helper is the normal case on every platform but one, so it
    /// has to read as "try the next one", not as a crash
    #[test]
    fn test_missing_helper_is_an_error_not_a_panic() {
        let err = copy_with_helper("panoptes-no-such-clipboard-helper", &[], "text");
        assert!(err.is_err());
    }

    /// The spawn/write/wait path itself, driven against a stand-in helper so
    /// the test suite never touches the developer's real clipboard
    #[test]
    fn test_helper_is_fed_on_stdin_and_its_exit_status_is_honored() {
        copy_with_helper("cat", &[], "selected text").expect("writing to a helper should work");

        let err = copy_with_helper("sh", &["-c", "cat >/dev/null; exit 3"], "selected text")
            .expect_err("a helper that fails must not report success");
        assert!(err.to_string().contains("exited with"), "{err:#}");
    }
}
