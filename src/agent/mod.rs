//! Agent adapter module
//!
//! This module defines the abstraction layer for different AI coding agents.
//! Currently supports Claude Code and OpenAI Codex CLI, as well as generic
//! shell sessions for running bash/zsh alongside agent sessions.

pub mod adapter;
pub mod claude;
pub mod codex;
pub mod events;
pub mod shell;

pub use adapter::{AgentAdapter, SpawnConfig, SpawnResult};
pub use claude::ClaudeCodeAdapter;
pub use codex::CodexAdapter;
pub use shell::ShellAdapter;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;

/// Write a script to disk and mark it executable (0755)
///
/// The shared install step for every hook and helper script an adapter ships:
/// parent directory, write, chmod. Content is written unconditionally so a
/// script left by an older Panoptes is refreshed rather than trusted.
pub(crate) fn install_executable_script(path: &Path, content: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create script directory {}", parent.display()))?;
    }

    std::fs::write(path, content)
        .with_context(|| format!("Failed to write script {}", path.display()))?;

    // Make executable (Unix only)
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(path)
            .with_context(|| format!("Failed to get metadata for {}", path.display()))?
            .permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(path, perms)
            .with_context(|| format!("Failed to set permissions on {}", path.display()))?;
    }

    Ok(())
}

/// Supported agent types
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum AgentType {
    /// Claude Code CLI
    #[default]
    ClaudeCode,
    /// Generic shell (bash/zsh/etc)
    Shell,
    /// OpenAI Codex CLI
    OpenAICodex,
}

impl AgentType {
    /// Get the display name for this agent type
    pub fn display_name(&self) -> &str {
        match self {
            AgentType::ClaudeCode => "Claude Code",
            AgentType::Shell => "Shell",
            AgentType::OpenAICodex => "Codex",
        }
    }

    /// Create an adapter instance for this agent type
    pub fn create_adapter(&self) -> Box<dyn AgentAdapter> {
        match self {
            AgentType::ClaudeCode => Box::new(ClaudeCodeAdapter::new()),
            AgentType::Shell => Box::new(ShellAdapter::new()),
            AgentType::OpenAICodex => Box::new(CodexAdapter::new()),
        }
    }
}

impl std::fmt::Display for AgentType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.display_name())
    }
}

/// The adapter that spawns a given kind of session
///
/// `SessionType` describes a session's state-tracking behaviour and is
/// persisted; `AgentType` picks the process to launch. They correspond
/// one-to-one, and this is the single place that correspondence is written
/// down.
impl From<crate::session::SessionType> for AgentType {
    fn from(session_type: crate::session::SessionType) -> Self {
        use crate::session::SessionType;
        match session_type {
            SessionType::ClaudeCode => AgentType::ClaudeCode,
            SessionType::OpenAICodex => AgentType::OpenAICodex,
            SessionType::Shell => AgentType::Shell,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_agent_type_display() {
        assert_eq!(AgentType::ClaudeCode.display_name(), "Claude Code");
        assert_eq!(AgentType::Shell.display_name(), "Shell");
        assert_eq!(AgentType::OpenAICodex.display_name(), "Codex");
    }

    #[test]
    fn test_agent_type_serialization() {
        let agent = AgentType::ClaudeCode;
        let json = serde_json::to_string(&agent).unwrap();
        let parsed: AgentType = serde_json::from_str(&json).unwrap();
        assert_eq!(agent, parsed);

        let shell = AgentType::Shell;
        let json = serde_json::to_string(&shell).unwrap();
        let parsed: AgentType = serde_json::from_str(&json).unwrap();
        assert_eq!(shell, parsed);

        let codex = AgentType::OpenAICodex;
        let json = serde_json::to_string(&codex).unwrap();
        let parsed: AgentType = serde_json::from_str(&json).unwrap();
        assert_eq!(codex, parsed);
    }

    #[test]
    fn test_agent_type_create_adapter() {
        let adapter = AgentType::ClaudeCode.create_adapter();
        assert_eq!(adapter.name(), "Claude Code");
        assert_eq!(adapter.command(), "claude");
        assert!(adapter.supports_hooks());

        let shell_adapter = AgentType::Shell.create_adapter();
        assert_eq!(shell_adapter.name(), "Shell");
        assert!(!shell_adapter.supports_hooks());

        let codex_adapter = AgentType::OpenAICodex.create_adapter();
        assert_eq!(codex_adapter.name(), "Codex");
        assert_eq!(codex_adapter.command(), "codex");
        assert!(codex_adapter.supports_hooks());
    }
}
