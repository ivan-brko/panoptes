//! Input handling module
//!
//! This module handles keyboard input conversion from crossterm KeyEvent
//! to terminal escape sequences that can be sent to the PTY.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::prelude::Rect;

/// Result of processing a key event
pub enum KeyAction {
    /// Forward the key to the PTY as escape sequence bytes
    Forward(Vec<u8>),
    /// Exit signal (ESC without modifiers was pressed)
    Exit,
    /// Ignore this key
    Ignore,
}

/// Process a key event and determine what action to take
///
/// # Arguments
/// * `key` - The key event from crossterm
///
/// # Returns
/// * `KeyAction::Exit` - if plain ESC was pressed (no modifiers)
/// * `KeyAction::Forward(bytes)` - bytes to send to PTY
/// * `KeyAction::Ignore` - if key should be ignored
pub fn process_key(key: KeyEvent) -> KeyAction {
    // Check for plain ESC (exit signal)
    if key.code == KeyCode::Esc && key.modifiers.is_empty() {
        return KeyAction::Exit;
    }

    // Convert key to bytes for PTY
    let bytes = key_event_to_bytes(key);
    if bytes.is_empty() {
        KeyAction::Ignore
    } else {
        KeyAction::Forward(bytes)
    }
}

/// Convert a crossterm KeyEvent to terminal escape sequence bytes
pub fn key_event_to_bytes(key: KeyEvent) -> Vec<u8> {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let alt = key.modifiers.contains(KeyModifiers::ALT);
    let shift = key.modifiers.contains(KeyModifiers::SHIFT);

    match key.code {
        // Basic keys
        KeyCode::Enter => vec![b'\r'],
        KeyCode::Tab => vec![b'\t'],
        KeyCode::BackTab => vec![0x1b, b'[', b'Z'], // Shift+Tab (CSI Z)
        KeyCode::Backspace => vec![0x7f],
        KeyCode::Esc => vec![0x1b],

        // Arrow keys with modifiers
        KeyCode::Up => {
            if ctrl || alt || shift {
                arrow_with_modifiers(b'A', ctrl, alt, shift)
            } else {
                vec![0x1b, b'[', b'A']
            }
        }
        KeyCode::Down => {
            if ctrl || alt || shift {
                arrow_with_modifiers(b'B', ctrl, alt, shift)
            } else {
                vec![0x1b, b'[', b'B']
            }
        }
        KeyCode::Right => {
            if ctrl || alt || shift {
                arrow_with_modifiers(b'C', ctrl, alt, shift)
            } else {
                vec![0x1b, b'[', b'C']
            }
        }
        KeyCode::Left => {
            if ctrl || alt || shift {
                arrow_with_modifiers(b'D', ctrl, alt, shift)
            } else {
                vec![0x1b, b'[', b'D']
            }
        }

        // Navigation keys
        KeyCode::Home => vec![0x1b, b'[', b'H'],
        KeyCode::End => vec![0x1b, b'[', b'F'],
        KeyCode::PageUp => vec![0x1b, b'[', b'5', b'~'],
        KeyCode::PageDown => vec![0x1b, b'[', b'6', b'~'],
        KeyCode::Insert => vec![0x1b, b'[', b'2', b'~'],
        KeyCode::Delete => vec![0x1b, b'[', b'3', b'~'],

        // Function keys (F1-F12)
        KeyCode::F(1) => vec![0x1b, b'O', b'P'],
        KeyCode::F(2) => vec![0x1b, b'O', b'Q'],
        KeyCode::F(3) => vec![0x1b, b'O', b'R'],
        KeyCode::F(4) => vec![0x1b, b'O', b'S'],
        KeyCode::F(5) => vec![0x1b, b'[', b'1', b'5', b'~'],
        KeyCode::F(6) => vec![0x1b, b'[', b'1', b'7', b'~'],
        KeyCode::F(7) => vec![0x1b, b'[', b'1', b'8', b'~'],
        KeyCode::F(8) => vec![0x1b, b'[', b'1', b'9', b'~'],
        KeyCode::F(9) => vec![0x1b, b'[', b'2', b'0', b'~'],
        KeyCode::F(10) => vec![0x1b, b'[', b'2', b'1', b'~'],
        KeyCode::F(11) => vec![0x1b, b'[', b'2', b'3', b'~'],
        KeyCode::F(12) => vec![0x1b, b'[', b'2', b'4', b'~'],
        KeyCode::F(_) => vec![],

        // Character input
        KeyCode::Char(c) => {
            if ctrl {
                // Ctrl+A through Ctrl+Z -> 0x01 through 0x1A
                if c.is_ascii_alphabetic() {
                    vec![(c.to_ascii_lowercase() as u8) - b'a' + 1]
                } else {
                    // Some special Ctrl combinations
                    match c {
                        '[' | '3' => vec![0x1b],       // Ctrl+[ = ESC
                        '\\' | '4' => vec![0x1c],      // Ctrl+\ = FS
                        ']' | '5' => vec![0x1d],       // Ctrl+] = GS
                        '6' | '^' => vec![0x1e],       // Ctrl+^ = RS
                        '7' | '/' | '_' => vec![0x1f], // Ctrl+_ = US
                        '2' | '@' | ' ' => vec![0x00], // Ctrl+@ = NUL
                        _ => vec![],
                    }
                }
            } else if alt {
                // Alt+char -> ESC followed by char
                let mut bytes = vec![0x1b];
                let mut buf = [0u8; 4];
                bytes.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
                bytes
            } else {
                let mut buf = [0u8; 4];
                c.encode_utf8(&mut buf).as_bytes().to_vec()
            }
        }

        // Null and other special
        KeyCode::Null => vec![0],
        _ => vec![],
    }
}

/// Generate arrow key sequence with modifiers
/// Format: CSI 1 ; <modifier> <direction>
fn arrow_with_modifiers(direction: u8, ctrl: bool, alt: bool, shift: bool) -> Vec<u8> {
    let modifier = 1 + (shift as u8) + (alt as u8 * 2) + (ctrl as u8 * 4);
    format!("\x1b[1;{}{}", modifier, direction as char).into_bytes()
}

/// Convert a mouse event to SGR mouse escape sequence bytes
///
/// # Arguments
/// * `mouse` - The mouse event from crossterm
/// * `content_area` - The content area rect where the PTY is rendered
///
/// # Returns
/// * `Some(bytes)` - SGR mouse escape sequence to send to PTY
/// * `None` - if the mouse event is outside the content area or not relevant
///
/// SGR format: `\x1b[<button;col;row{M|m}`
/// - M = press/motion, m = release
/// - Coordinates are 1-indexed
pub fn mouse_event_to_bytes(mouse: MouseEvent, content_area: Rect) -> Option<Vec<u8>> {
    // Check if mouse is within content area
    if mouse.column < content_area.x
        || mouse.column >= content_area.x + content_area.width
        || mouse.row < content_area.y
        || mouse.row >= content_area.y + content_area.height
    {
        return None;
    }

    // Translate screen coordinates to content-relative coordinates (1-indexed for SGR)
    let col = (mouse.column - content_area.x) + 1;
    let row = (mouse.row - content_area.y) + 1;

    // Determine button code and press/release
    let (button, is_release) = match mouse.kind {
        MouseEventKind::Down(btn) => {
            let code = match btn {
                MouseButton::Left => 0,
                MouseButton::Middle => 1,
                MouseButton::Right => 2,
            };
            (code, false)
        }
        MouseEventKind::Up(btn) => {
            let code = match btn {
                MouseButton::Left => 0,
                MouseButton::Middle => 1,
                MouseButton::Right => 2,
            };
            (code, true)
        }
        MouseEventKind::Drag(btn) => {
            // Drag is button + 32
            let code = match btn {
                MouseButton::Left => 32,
                MouseButton::Middle => 33,
                MouseButton::Right => 34,
            };
            (code, false)
        }
        MouseEventKind::ScrollUp => (64, false),
        MouseEventKind::ScrollDown => (65, false),
        MouseEventKind::ScrollLeft => (66, false),
        MouseEventKind::ScrollRight => (67, false),
        MouseEventKind::Moved => {
            // Mouse motion without button - button code 35
            (35, false)
        }
    };

    // Add modifier bits to button code
    let mut final_button = button;
    if mouse.modifiers.contains(KeyModifiers::SHIFT) {
        final_button += 4;
    }
    if mouse.modifiers.contains(KeyModifiers::ALT) {
        final_button += 8;
    }
    if mouse.modifiers.contains(KeyModifiers::CONTROL) {
        final_button += 16;
    }

    // Generate SGR escape sequence
    let suffix = if is_release { 'm' } else { 'M' };
    let sequence = format!("\x1b[<{};{};{}{}", final_button, col, row, suffix);

    Some(sequence.into_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_plain_esc_is_exit() {
        let key = KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE);
        assert!(matches!(process_key(key), KeyAction::Exit));
    }

    #[test]
    fn test_alt_esc_is_forwarded() {
        let key = KeyEvent::new(KeyCode::Esc, KeyModifiers::ALT);
        match process_key(key) {
            KeyAction::Forward(bytes) => assert_eq!(bytes, vec![0x1b]),
            _ => panic!("Expected Forward"),
        }
    }

    #[test]
    fn test_enter() {
        let key = KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE);
        assert_eq!(key_event_to_bytes(key), vec![b'\r']);
    }

    #[test]
    fn test_char() {
        let key = KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE);
        assert_eq!(key_event_to_bytes(key), vec![b'a']);
    }

    #[test]
    fn test_ctrl_c() {
        let key = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
        assert_eq!(key_event_to_bytes(key), vec![0x03]);
    }

    #[test]
    fn test_arrow_up() {
        let key = KeyEvent::new(KeyCode::Up, KeyModifiers::NONE);
        assert_eq!(key_event_to_bytes(key), vec![0x1b, b'[', b'A']);
    }

    #[test]
    fn test_alt_char() {
        let key = KeyEvent::new(KeyCode::Char('x'), KeyModifiers::ALT);
        assert_eq!(key_event_to_bytes(key), vec![0x1b, b'x']);
    }

    #[test]
    fn test_unicode() {
        let key = KeyEvent::new(KeyCode::Char('é'), KeyModifiers::NONE);
        let bytes = key_event_to_bytes(key);
        assert_eq!(bytes, "é".as_bytes());
    }

    #[test]
    fn test_ctrl_shift_arrow() {
        let key = KeyEvent::new(KeyCode::Up, KeyModifiers::CONTROL | KeyModifiers::SHIFT);
        let bytes = key_event_to_bytes(key);
        // Modifier = 1 + 1 (shift) + 4 (ctrl) = 6
        assert_eq!(bytes, b"\x1b[1;6A");
    }

    #[test]
    fn test_mouse_scroll_up() {
        let content_area = Rect::new(1, 2, 78, 20);
        let mouse = MouseEvent {
            kind: MouseEventKind::ScrollUp,
            column: 10,
            row: 5,
            modifiers: KeyModifiers::NONE,
        };
        let bytes = mouse_event_to_bytes(mouse, content_area).unwrap();
        // col = 10 - 1 + 1 = 10, row = 5 - 2 + 1 = 4
        assert_eq!(bytes, b"\x1b[<64;10;4M");
    }

    #[test]
    fn test_mouse_scroll_down() {
        let content_area = Rect::new(1, 2, 78, 20);
        let mouse = MouseEvent {
            kind: MouseEventKind::ScrollDown,
            column: 10,
            row: 5,
            modifiers: KeyModifiers::NONE,
        };
        let bytes = mouse_event_to_bytes(mouse, content_area).unwrap();
        // col = 10 - 1 + 1 = 10, row = 5 - 2 + 1 = 4
        assert_eq!(bytes, b"\x1b[<65;10;4M");
    }

    #[test]
    fn test_mouse_left_click() {
        let content_area = Rect::new(0, 0, 80, 24);
        let mouse = MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 5,
            row: 10,
            modifiers: KeyModifiers::NONE,
        };
        let bytes = mouse_event_to_bytes(mouse, content_area).unwrap();
        // col = 5 + 1 = 6, row = 10 + 1 = 11
        assert_eq!(bytes, b"\x1b[<0;6;11M");
    }

    #[test]
    fn test_mouse_left_release() {
        let content_area = Rect::new(0, 0, 80, 24);
        let mouse = MouseEvent {
            kind: MouseEventKind::Up(MouseButton::Left),
            column: 5,
            row: 10,
            modifiers: KeyModifiers::NONE,
        };
        let bytes = mouse_event_to_bytes(mouse, content_area).unwrap();
        // Release uses 'm' suffix
        assert_eq!(bytes, b"\x1b[<0;6;11m");
    }

    #[test]
    fn test_mouse_outside_content_area() {
        let content_area = Rect::new(1, 2, 78, 20);
        // Mouse is outside the content area (y < content_area.y)
        let mouse = MouseEvent {
            kind: MouseEventKind::ScrollUp,
            column: 10,
            row: 0,
            modifiers: KeyModifiers::NONE,
        };
        assert!(mouse_event_to_bytes(mouse, content_area).is_none());
    }

    #[test]
    fn test_mouse_with_modifiers() {
        let content_area = Rect::new(0, 0, 80, 24);
        let mouse = MouseEvent {
            kind: MouseEventKind::ScrollUp,
            column: 1,
            row: 1,
            modifiers: KeyModifiers::SHIFT | KeyModifiers::CONTROL,
        };
        let bytes = mouse_event_to_bytes(mouse, content_area).unwrap();
        // button = 64 + 4 (shift) + 16 (ctrl) = 84
        assert_eq!(bytes, b"\x1b[<84;2;2M");
    }
}
