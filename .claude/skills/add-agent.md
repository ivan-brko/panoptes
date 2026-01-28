# Add Agent Skill

Step-by-step checklist for adding a new agent type to Panoptes.

## Steps

### 1. Add AgentType Enum Variant

In `src/agent/mod.rs`, add the new variant:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AgentType {
    ClaudeCode,
    NewAgent,  // Your new agent type
}
```

### 2. Implement Display

Update the `Display` impl if it exists, or add display helpers:

```rust
impl AgentType {
    pub fn display_name(&self) -> &'static str {
        match self {
            AgentType::ClaudeCode => "Claude Code",
            AgentType::NewAgent => "New Agent",
        }
    }
}
```

### 3. Create Adapter Module

Create `src/agent/new_agent.rs`:

```rust
use anyhow::Result;
use std::path::Path;

use crate::agent::AgentAdapter;

pub struct NewAgentAdapter {
    // Configuration fields
}

impl NewAgentAdapter {
    pub fn new() -> Self {
        Self {
            // Initialize fields
        }
    }
}

impl AgentAdapter for NewAgentAdapter {
    fn spawn_command(&self) -> String {
        // Return the command to spawn the agent
        "new-agent".to_string()
    }

    fn spawn_args(&self, working_dir: &Path, session_id: &str) -> Vec<String> {
        // Return command arguments
        vec![
            "--workdir".to_string(),
            working_dir.display().to_string(),
            "--session".to_string(),
            session_id.to_string(),
        ]
    }

    fn environment(&self) -> Vec<(String, String)> {
        // Return any required environment variables
        vec![]
    }

    fn hook_script(&self, session_id: &str, hook_port: u16) -> Option<String> {
        // Return hook script content if the agent supports hooks
        // Return None if hooks aren't supported
        None
    }
}
```

### 4. Export from agent/mod.rs

In `src/agent/mod.rs`, add:

```rust
mod new_agent;
pub use new_agent::NewAgentAdapter;
```

### 5. Update Factory Method

In `src/agent/mod.rs`, update `create_adapter()`:

```rust
impl AgentType {
    pub fn create_adapter(&self, config: &Config) -> Box<dyn AgentAdapter> {
        match self {
            AgentType::ClaudeCode => Box::new(ClaudeCodeAdapter::new(config)),
            AgentType::NewAgent => Box::new(NewAgentAdapter::new()),
        }
    }
}
```

### 6. Update Hook Handling (if applicable)

If the new agent sends hook events, update `src/hooks/mod.rs` to parse its event format:

```rust
impl HookEvent {
    pub fn from_agent_payload(agent: AgentType, payload: &str) -> Result<Self> {
        match agent {
            AgentType::ClaudeCode => Self::from_claude_payload(payload),
            AgentType::NewAgent => Self::from_new_agent_payload(payload),
        }
    }
}
```

### 7. Update Session Manager (if needed)

If the agent has special session handling requirements, update `src/session/manager.rs`.

## AgentAdapter Trait Reference

```rust
pub trait AgentAdapter: Send + Sync {
    /// The command to execute (e.g., "claude", "aider")
    fn spawn_command(&self) -> String;

    /// Arguments for the command
    fn spawn_args(&self, working_dir: &Path, session_id: &str) -> Vec<String>;

    /// Environment variables to set
    fn environment(&self) -> Vec<(String, String)>;

    /// Hook script content (if agent supports hooks)
    fn hook_script(&self, session_id: &str, hook_port: u16) -> Option<String>;
}
```

## Verification

1. Run `cargo build` to check for compile errors
2. Run `cargo clippy -- -D warnings`
3. Test spawning a session with the new agent type
4. Verify hook events are received (if applicable)
5. Verify session state transitions work correctly
