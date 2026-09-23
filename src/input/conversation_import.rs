//! Input for the conversation import picker
//!
//! Opened with `i` at a branch level of pane 1, once the background scan has
//! found something (`App::start_conversation_import`). `↑`/`↓` move, `Enter`
//! adopts the selected conversation as a resumable session, `Esc` closes.
//!
//! The key handling is a pure function of [`AppState`] so it can be tested
//! without an `App`; only adopting needs one.

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind};

use crate::app::{cycle_next, cycle_prev, App, AppState, InputMode};

/// What a key press in the picker asks for
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PickerAction {
    /// Nothing beyond what was already applied to the state (or nothing)
    None,
    /// Adopt the selected conversation
    Adopt,
}

/// Handle a key while the import picker is open
pub fn handle_importing_conversation_key(app: &mut App, key: KeyEvent) -> Result<()> {
    if picker_key(&mut app.state, key) == PickerAction::Adopt {
        app.adopt_selected_conversation();
    }
    Ok(())
}

/// Apply a picker key to the state, returning what is left for the app to do
pub fn picker_key(state: &mut AppState, key: KeyEvent) -> PickerAction {
    if key.kind != KeyEventKind::Press {
        return PickerAction::None;
    }
    let Some(import) = state.conversation_import.as_mut() else {
        // Nothing to pick from: the mode is stale, so leave it
        state.input_mode = InputMode::Normal;
        return PickerAction::None;
    };
    let count = import.conversations.len();

    match key.code {
        KeyCode::Esc => {
            state.conversation_import = None;
            state.input_mode = InputMode::Normal;
        }
        KeyCode::Down => import.selected = cycle_next(import.selected, count),
        KeyCode::Up => import.selected = cycle_prev(import.selected, count),
        KeyCode::Enter if count > 0 => return PickerAction::Adopt,
        _ => {}
    }
    PickerAction::None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::ConversationImport;
    use crate::transcript::scan::FoundConversation;
    use crate::transcript::TranscriptKind;
    use crossterm::event::KeyModifiers;
    use std::path::PathBuf;
    use uuid::Uuid;

    fn press(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn conversation(id: &str) -> FoundConversation {
        FoundConversation {
            kind: TranscriptKind::Claude,
            id: id.to_string(),
            title: Some(format!("title {id}")),
            last_active: chrono::Utc::now(),
            config_id: None,
            account_name: None,
            path: PathBuf::from(format!("/tmp/{id}.jsonl")),
        }
    }

    fn picker(count: usize) -> AppState {
        let project_id = Uuid::new_v4();
        let branch_id = Uuid::new_v4();
        let mut state = AppState::default();
        state.navigate_to_branch(project_id, branch_id);
        state.input_mode = InputMode::ImportingConversation;
        state.conversation_import = Some(ConversationImport {
            project_id,
            branch_id,
            working_dir: PathBuf::from("/tmp"),
            conversations: (0..count).map(|i| conversation(&i.to_string())).collect(),
            selected: 0,
            truncated: false,
        });
        state
    }

    #[test]
    fn test_arrows_move_the_selection_and_wrap() {
        let mut state = picker(3);
        assert_eq!(
            picker_key(&mut state, press(KeyCode::Down)),
            PickerAction::None
        );
        assert_eq!(state.conversation_import.as_ref().unwrap().selected, 1);
        picker_key(&mut state, press(KeyCode::Up));
        picker_key(&mut state, press(KeyCode::Up));
        assert_eq!(
            state.conversation_import.as_ref().unwrap().selected,
            2,
            "up from the top wraps to the bottom"
        );
    }

    #[test]
    fn test_enter_adopts_and_esc_closes() {
        let mut state = picker(2);
        assert_eq!(
            picker_key(&mut state, press(KeyCode::Enter)),
            PickerAction::Adopt
        );
        // Adopting is the app's job; the picker leaves the state for it
        assert!(state.conversation_import.is_some());

        assert_eq!(
            picker_key(&mut state, press(KeyCode::Esc)),
            PickerAction::None
        );
        assert!(state.conversation_import.is_none());
        assert_eq!(state.input_mode, InputMode::Normal);
    }

    #[test]
    fn test_other_keys_do_nothing() {
        // In particular `q` must not quit and `i` must not rescan from inside
        // the picker: the global keys do not apply outside normal mode
        let mut state = picker(2);
        for code in [KeyCode::Char('q'), KeyCode::Char('i'), KeyCode::Tab] {
            assert_eq!(picker_key(&mut state, press(code)), PickerAction::None);
        }
        assert_eq!(state.input_mode, InputMode::ImportingConversation);
        assert_eq!(state.conversation_import.as_ref().unwrap().selected, 0);
    }

    #[test]
    fn test_a_stale_mode_without_a_picker_returns_to_normal() {
        let mut state = AppState {
            input_mode: InputMode::ImportingConversation,
            ..Default::default()
        };
        assert_eq!(
            picker_key(&mut state, press(KeyCode::Enter)),
            PickerAction::None
        );
        assert_eq!(state.input_mode, InputMode::Normal);
    }
}
