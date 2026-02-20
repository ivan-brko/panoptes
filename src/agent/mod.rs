//! Agent adapter module
//!
//! This module defines the abstraction layer for different AI coding agents.
//! Currently supports Claude Code and OpenAI Codex CLI, as well as generic
//! shell sessions for running bash/zsh alongside agent sessions.

pub mod adapter;
pub mod claude;
pub mod codex;
pub mod shell;

pub use adapter::{AgentAdapter, SpawnConfig, SpawnResult};
pub use claude::ClaudeCodeAdapter;
pub use codex::CodexAdapter;
pub use shell::ShellAdapter;

use serde::{Deserialize, Serialize};

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

    /// Get the command used to invoke this agent
    pub fn command(&self) -> &str {
        match self {
            AgentType::ClaudeCode => "claude",
            AgentType::Shell => std::env::var("SHELL")
                .map(|_| "shell")
                .unwrap_or("/bin/bash"),
            AgentType::OpenAICodex => "codex",
        }
    }

    /// Check if this agent supports hooks for state tracking
    pub fn supports_hooks(&self) -> bool {
        match self {
            AgentType::ClaudeCode => true,
            AgentType::Shell => false,
            AgentType::OpenAICodex => true,
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
    fn test_agent_type_command() {
        assert_eq!(AgentType::ClaudeCode.command(), "claude");
        assert_eq!(AgentType::OpenAICodex.command(), "codex");
        // Shell command depends on $SHELL env var
    }

    #[test]
    fn test_agent_type_supports_hooks() {
        assert!(AgentType::ClaudeCode.supports_hooks());
        assert!(!AgentType::Shell.supports_hooks());
        assert!(AgentType::OpenAICodex.supports_hooks());
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
