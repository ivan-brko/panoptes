---
name: add-agent
description: >
  Step-by-step checklist for adding a new agent type to Panoptes.
  Follow the checklist to add the enum variant, create the adapter
  implementation, and update the factory method.
---

# Add Agent Skill

Step-by-step checklist for adding a new agent type to Panoptes.

## Steps

### 1. Add AgentType Enum Variant

In `src/agent/mod.rs`, add the new variant:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum AgentType {
    #[default]
    ClaudeCode,
    Shell,
    OpenAICodex,
    NewAgent,  // Your new agent type
}
```

### 2. Implement Display

Update `display_name()` in `src/agent/mod.rs`:

```rust
impl AgentType {
    pub fn display_name(&self) -> &str {
        match self {
            AgentType::ClaudeCode => "Claude Code",
            AgentType::Shell => "Shell",
            AgentType::OpenAICodex => "Codex",
            AgentType::NewAgent => "New Agent",
        }
    }
}
```

### 3. Create Adapter Module

Create `src/agent/new_agent.rs`. `spawn()` has a default implementation on the
trait (setup hooks → build env → build args → PTY), so an adapter only supplies
the varying parts:

```rust
use crate::config::Config;
use anyhow::Result;
use std::collections::HashMap;
use std::path::PathBuf;

use super::adapter::{AgentAdapter, SpawnConfig};

pub struct NewAgentAdapter;

impl AgentAdapter for NewAgentAdapter {
    fn name(&self) -> &str {
        "New Agent"
    }

    fn command(&self) -> &str {
        "new-agent"
    }

    fn default_args(&self) -> Vec<String> {
        vec![]
    }

    fn supports_hooks(&self) -> bool {
        false
    }

    fn generate_env(
        &self,
        _config: &Config,
        spawn_config: &SpawnConfig,
    ) -> HashMap<String, String> {
        let mut env = HashMap::new();
        env.insert(
            "PANOPTES_SESSION_ID".to_string(),
            spawn_config.session_id.to_string(),
        );
        env
    }

    fn setup_hooks(&self, config: &Config, spawn_config: &SpawnConfig) -> Result<Vec<PathBuf>> {
        // Install hook scripts / settings here if the agent supports them.
        // Use `super::install_executable_script(&path, &content)` to write
        // a script and mark it executable in one step.
        Ok(vec![])
    }

    fn build_args(&self, spawn_config: &SpawnConfig) -> Vec<String> {
        // Combine default_args, resume handling and the initial prompt into
        // the final command line. See CodexAdapter::build_args for a worked
        // example with a resume subcommand and positional prompt.
        let mut args = self.default_args();
        if let Some(ref prompt) = spawn_config.initial_prompt {
            args.push(prompt.clone());
        }
        args
    }

    fn agent_session_id(&self, spawn_config: &SpawnConfig) -> Option<String> {
        // Some(...) only if Panoptes can dictate the agent's conversation ID
        // upfront (as Claude does via --session-id). Return None if the agent
        // mints its own ID and it must be discovered later (as Codex does).
        None
    }

    // Only override spawn() if the default lifecycle
    // (setup_hooks → generate_env → build_args → PtyHandle::spawn)
    // genuinely does not fit.
}
```

### 4. Export from agent/mod.rs

In `src/agent/mod.rs`, add:

```rust
pub mod new_agent;
pub use new_agent::NewAgentAdapter;
```

### 5. Update Factory Method

In `src/agent/mod.rs`, update `create_adapter()`:

```rust
impl AgentType {
    pub fn create_adapter(&self) -> Box<dyn AgentAdapter> {
        match self {
            AgentType::ClaudeCode => Box::new(ClaudeCodeAdapter::new()),
            AgentType::Shell => Box::new(ShellAdapter::new()),
            AgentType::OpenAICodex => Box::new(CodexAdapter::new()),
            AgentType::NewAgent => Box::new(NewAgentAdapter),
        }
    }
}
```

Also update the `From<crate::session::SessionType>` impl in the same file if
the new agent gets its own `SessionType` variant.

### 6. Update Hook Handling (if applicable)

If the new agent reports state via the HTTP hook server, have its hook script
POST the envelope shape `HookEvent` expects (see `src/hooks/mod.rs`):
`{"session_id": ..., "event": ..., "timestamp": ..., "payload": {...}}`.
If it introduces new event names, extend `HookEventType` and the state
machine's `translate_hook` in `src/session/state_machine.rs`.

### 7. Update Session Manager (if needed)

If the agent has special session handling requirements (transcript tailing,
conversation-ID discovery), see `src/transcript/` and how
`src/session/manager.rs` wires Codex up.

## AgentAdapter Trait Reference

From `src/agent/adapter.rs` (object-safe; `spawn` is a default method):

```rust
pub trait AgentAdapter: Send + Sync {
    /// Display name of this agent
    fn name(&self) -> &str;

    /// Command used to invoke this agent
    fn command(&self) -> &str;

    /// Default command-line arguments
    fn default_args(&self) -> Vec<String>;

    /// Whether this agent supports hooks for state tracking
    fn supports_hooks(&self) -> bool;

    /// Environment variables for the agent process
    fn generate_env(&self, config: &Config, spawn_config: &SpawnConfig) -> HashMap<String, String>;

    /// Create hook scripts/settings; returns paths to clean up on session end
    fn setup_hooks(&self, config: &Config, spawn_config: &SpawnConfig) -> Result<Vec<PathBuf>>;

    /// Complete argument list for a spawn (resume, prompt, defaults)
    fn build_args(&self, spawn_config: &SpawnConfig) -> Vec<String>;

    /// Agent-native conversation ID, if Panoptes can dictate it upfront
    fn agent_session_id(&self, spawn_config: &SpawnConfig) -> Option<String>;

    /// Spawn the agent in a PTY — default implementation:
    /// setup_hooks → generate_env → build_args → PtyHandle::spawn
    fn spawn(&self, config: &Config, spawn_config: &SpawnConfig) -> Result<SpawnResult> { ... }
}
```

## Verification

1. Run `cargo build` to check for compile errors
2. Run `cargo lint` (clippy with `-D warnings`)
3. Run `cargo test`
4. Test spawning a session with the new agent type
5. Verify hook events are received (if applicable)
6. Verify session state transitions work correctly
