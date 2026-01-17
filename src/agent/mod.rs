//! Agent adapter module
//!
//! This module defines the abstraction layer for different AI coding agents.
//! Version 1.0 supports Claude Code only, but the architecture allows for
//! adding other agents (Aider, OpenAI Codex, etc.) in the future.

pub mod adapter;
pub mod claude;

pub use adapter::{AgentAdapter, SpawnConfig, SpawnResult};
pub use claude::ClaudeCodeAdapter;

use serde::{Deserialize, Serialize};

/// Supported agent types
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum AgentType {
    /// Claude Code CLI
    #[default]
    ClaudeCode,
    // Future agents:
    // Aider,
    // OpenAICodex,
    // Cursor,
}

impl AgentType {
    /// Get the display name for this agent type
    pub fn display_name(&self) -> &str {
        match self {
            AgentType::ClaudeCode => "Claude Code",
        }
    }

    /// Get the command used to invoke this agent
    pub fn command(&self) -> &str {
        match self {
            AgentType::ClaudeCode => "claude",
        }
    }

    /// Check if this agent supports hooks for state tracking
    pub fn supports_hooks(&self) -> bool {
        match self {
            AgentType::ClaudeCode => true,
        }
    }

    /// Create an adapter instance for this agent type
    pub fn create_adapter(&self) -> Box<dyn AgentAdapter> {
        match self {
            AgentType::ClaudeCode => Box::new(ClaudeCodeAdapter::new()),
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
    }

    #[test]
    fn test_agent_type_command() {
        assert_eq!(AgentType::ClaudeCode.command(), "claude");
    }

    #[test]
    fn test_agent_type_supports_hooks() {
        assert!(AgentType::ClaudeCode.supports_hooks());
    }

    #[test]
    fn test_agent_type_serialization() {
        let agent = AgentType::ClaudeCode;
        let json = serde_json::to_string(&agent).unwrap();
        let parsed: AgentType = serde_json::from_str(&json).unwrap();
        assert_eq!(agent, parsed);
    }

    #[test]
    fn test_agent_type_create_adapter() {
        let adapter = AgentType::ClaudeCode.create_adapter();
        assert_eq!(adapter.name(), "Claude Code");
        assert_eq!(adapter.command(), "claude");
        assert!(adapter.supports_hooks());
    }
}
