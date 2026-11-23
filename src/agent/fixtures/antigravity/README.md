# Antigravity CLI (`agy`) fixtures

These were recorded from a real `agy` **1.5.15** session (macOS, consumer Google sign-in) for the
PAN-59 spike. PAN-61's state-machine tests read them.

## How they were captured

- `agy` ran in a PTY at 120x40. A small driver answered terminal queries the way Panoptes'
  `QueryResponder` does.
- The session used a private home, `agy --gemini_dir=<abs path>`, and a status line injected
  with `agy --settings <overlay.json>`.
- The status line, plus every hook event in `<gemini_dir>/config/hooks.json` and the workspace's
  `.agents/hooks.json` (`SessionStart` and `Stop` only), ran a logger. The logger appended stdin
  to a file.
- The status-line command printed one line. Hooks printed `{}`, except `PreToolUse`, which
  printed nothing (see the warning below).

### Redaction

| Real value | Replaced with |
| --- | --- |
| Conversation ids | `11111111-…` (main conversation), `22222222-…` |
| Panoptes session id | `00000000-0000-4000-8000-0000000000aa` |
| Email | `user@example.com` |
| `gemini_dir` | `/home/user/.gemini` |
| Project | `/home/user/projects/demo` |
| Logger script | `/home/user/.panoptes/bin/agy-logger` |

Prompts were written for the spike. Quota numbers are left as captured.

## Files

### `status-line/*.json`: one status-line stdin payload per state

| File | `agent_state` | Notes |
| --- | --- | --- |
| `authenticating.json` | `authenticating` | First payload. `conversation_id` is `""`, `model` is null, there is no `email` or `quota` |
| `initializing.json` | `initializing` | Fresh home: shown during the onboarding screens |
| `idle-before-first-prompt.json` | `idle` | **`conversation_id` is `""` until the first prompt** of a new conversation. On `--conversation <id>` it is set from the first idle payload |
| `working.json` | `working` | First payload of a turn. `conversation_id` is now set |
| `tool-confirmation-pending.json` | `tool_use` | **Waiting for approval**: `tool_confirmation_pending: true`. The key is absent (not `false`) otherwise |
| `idle.json` | `idle` | Turn finished |
| `working-background-task-running.json` | `working` | `task_count: 1` after a background command starts |
| `idle-background-task-running.json` | `idle` | Turn ended while the background command still runs (`task_count: 1`) |
| `working-self-wake.json` | `working` | **Self-wake**: the background command finished and agy started a turn with no user input |
| `idle-after-self-wake.json` | `idle` | `task_count` is gone again |
| `idle-{auto,plan,accept-edits}-mode.json` | `idle` | `cycle_mode` is the agent mode. It is **absent in the default (ask) mode** |

Payload facts PAN-61 relies on:

- `agent_state` was only ever `authenticating`, `initializing`, `idle`, `working` or `tool_use`.
  - `tool_use` appeared **only** together with `tool_confirmation_pending: true`. Auto-approved
    tools stayed in `working` throughout.
  - `error` and `reviewing` were not produced. An unknown `--model` silently falls back to the
    default rather than erroring.
- Present: `context_window.{total_input_tokens,total_output_tokens,context_window_size,used_percentage,current_usage}`,
  `model.{id,display_name,effort}`, `quota.<family>-weekly.{remaining_fraction,reset_time,reset_in_seconds}`,
  `plan_tier`, `email` and `terminal_width`.
  - On this consumer tier there is **no `cost` key and no 5-hour window**.
- **`transcript_path` in the status line is wrong.** It points at
  `<gemini_dir>/antigravity/brain/<id>/…`, which does not exist. The real file is under
  `<gemini_dir>/antigravity-cli/brain/<id>/…`, which is what the hooks' `transcriptPath` names.
- `session_id` equals `conversation_id`.
- The status line re-runs about every 300 ms while anything changes (token counts, quota). The
  sequences below collapse runs of identical state into one entry.

### `hooks/*.json`: one stdin payload per hook event

- `SessionStart`: fires at the **first prompt of a new conversation** (not at launch), and **not**
  on `--conversation` resume.
- `PreInvocation` / `PostInvocation`: around every model call. A tool-using turn has several.
- `PreToolUse`: carries `toolCall.{id,name,args}` and `stepIdx`. Shell commands are
  `name: "run_command"` with `args.CommandLine`.
- `PostToolUse`: carries the same `toolCall.id`. For a background command it fires **when the
  command finishes**, which is what starts the self-wake turn
  (`PostToolUse-background-command-finished.json`).
- `Stop`: `terminationReason: "NO_TOOL_CALL"` plus `fullyIdle`.
  - `Stop-background-task-running.json` is `fullyIdle: false`, from a turn that ended with a
    background command still running.
  - `Stop.json` is `fullyIdle: true`.
- `_process-context.json`: hook and status-line processes **inherit the agent's environment**, so
  `PANOPTES_*` arrives. Their working directories differ:
  - A hook runs in the directory that holds its `hooks.json`.
  - The status line runs in the agent's cwd.

> **Warning: `PreToolUse` output `{}` means *deny*.** A `PreToolUse` hook that prints `{}` (no
> `decision`) blocks the tool ("tool call denied by pre-tool hook"); see sequence 01. A hook that
> prints **nothing** is neutral. Do not register `PreToolUse` at all unless it must decide something.

> **Esc-cancel fires no hook.** A turn interrupted with Esc produces `PreInvocation` and then
> nothing: no `PostInvocation` and no `Stop` (sequence 05). Only the status line returns to `idle`.

### `sequences/*.jsonl`: status line and hooks merged in arrival order

Each line is `{"t": seconds, "source": "status_line"|"hook", "hooks_json": "user"|"workspace", "payload": {…}}`.

| File | What happens |
| --- | --- |
| `01-startup-and-hook-denied-tool.jsonl` | Launch, onboarding, then the first prompt. The `{}`-printing `PreToolUse` hook denies `echo hi`: there is no approval and no `PostToolUse` |
| `02-approval-turn.jsonl` | Default mode: `working`, then `tool_use` + pending (`PreToolUse` fires just before), approve, `PostToolUse`, `idle` and `Stop` |
| `03-background-command-self-wake.jsonl` | Approval, then `sleep 20 && echo done` in the background. The turn ends (`Stop fullyIdle:false`, `idle` with `task_count:1`). About 15 s later `PostToolUse` arrives, and so do `PreInvocation`, `working`, `Stop fullyIdle:true` and `idle` |
| `04-resume-auto-mode-turn.jsonl` | `agy --conversation <id>`. `conversation_id` is set from the first idle payload, and there is no `SessionStart`. In auto mode `echo resumed` runs with no approval and no `tool_use` state |
| `05-esc-cancel.jsonl` | Prompt, then Esc after 3 s: `working`, then `idle`, and only `PreInvocation` among the hooks |

### `transcript.jsonl`

This is the first conversation's `<gemini_dir>/antigravity-cli/brain/<id>/.system_generated/logs/transcript.jsonl`.

- Each line is one step: `step_index`, `source` (`USER_EXPLICIT` / `MODEL` / `SYSTEM`), `type`
  (`USER_INPUT`, `PLANNER_RESPONSE`, `GENERIC`, `SYSTEM_MESSAGE`, `ERROR_MESSAGE`), `status`,
  `created_at` and `content`.
  - Model steps add `model`, `input_tokens`, `cache_read_tokens`, `output_tokens` and `tool_calls`.
- `transcript_full.jsonl` sits next to it, and so do the conversation's SQLite database
  `<gemini_dir>/antigravity-cli/conversations/<id>.db` (tables `steps`, `trajectory_meta`,
  `gen_metadata`, …, with protobuf blobs) and `conversation_summaries.db`.
- Panoptes does not need any of these for state, because the status line carries it. They would
  only matter for an importer or usage history.

### `terminal-sequences.txt`

This file holds the escape sequences agy writes at startup, on leaving the splash, and on exit,
with notes on which ones Panoptes answers. Replaying the full raw capture through `panoptes-vt100`
0.16.2 gives a clean screen with no leaked sequences.
