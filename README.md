# mu · µ · 無

A small Rust coding-agent TUI. Match the post-train, not the other way around.

One prompt: `You are coding agent`. One tool:
`bash(command: str, timeoutMs?: int): str`, described only as
`Special bash commands: view_image`.

No provider adapters, autonomous prompt additions, approvals, compaction, or
workspace checkpoints. Use a compatible Responses API directly, or let CLIProxyAPI
normalize providers. **Commands run with your
permissions, without a sandbox.** Use a disposable environment for untrusted work.

## Run

```sh
nix develop
cargo run --release
# or: nix run
```

Configure your Responses API endpoint (CLIProxyAPI example):

```sh
export MU_BASE_URL=http://127.0.0.1:8317/v1
export MU_API_KEY=your-key       # optional; also accepts OPENAI_API_KEY
export MU_MODEL=your-model
export MU_EFFORT=high           # optional; passed through unchanged
export MU_CONTEXT=128000        # display-only context capacity, in tokens
mu                             # or cargo run --release
```

Outside Nix: Linux, Rust (edition 2024), Bash, curl (7.76+), and ripgrep (`rg`) for
file search. `MU_MODEL` defaults to `gpt-5`; it must name a model served by your endpoint.
No model discovery or provider-specific configuration.

## Use

| Command | Action |
| --- | --- |
| `/model <model> [effort]` | Set model; omitting effort clears it |
| `/new` | Empty session, same directory/model; reload project instructions |
| `/resume` | Pick a saved session (newest first) |
| `/tree` | Pick any conversation point, including another branch |
| `/copy [agent\|user]` | Copy the last agent response or user message; defaults to agent |
| `/quit` | Cancel active work, save the completed boundary, leave |

Enter sends. While working, it queues steering messages. After all calls in the
current response have results, queued messages are appended **before** the next
request. If the response has no calls, queued messages still start another turn.
Esc (after dismissing any completion menu) or Ctrl+C stops; queued messages survive cancellation/errors until Enter
retries or you switch sessions. Ctrl+D on an empty input quits.

- Alt+Enter (or Ctrl+J): newline. Bracketed paste preserves newlines.
- Left/Right, Home/End, Ctrl+A/E/U/W: edit.
- Ctrl+O: expand/collapse thoughts and long commands/results.
- PgUp/PgDn or mouse wheel: scroll.
- Pickers: arrows or j/k, Home/End, Enter, Esc.
- Empty Enter continues the selected point without adding a user message.

`/tree` changes **conversation history only**. It never checks out, reverts, or
otherwise changes project files. Old futures remain selectable. A tree point is
either a user message or a complete response **plus all its tool results**; it
cannot leave unmatched tool calls in replay. Selecting a point does not execute
anything. Commands already run are never undone.

Sessions are private JSON files in `$XDG_STATE_HOME/mu` (normally
`~/.local/state/mu`). They contain the prompt snapshot, full branch tree, tool
outputs, opaque reasoning, and attached images. `/resume` restores the session's
original working directory. New sessions stay in memory until the first message
is sent; `/new`, `/model`, and quitting alone don't create session files. Older
empty sessions are hidden in `/resume`, not deleted. The picker reads small
`.meta` sidecars instead of full transcripts; missing/stale caches are rebuilt
lazily, so existing sessions remain compatible. Selecting a saved session does
not rewrite it. Session/model changes require the agent to be idle.
Do not concurrently edit the same session from multiple mu instances.

## Mentions, skills, completion

```text
explain @src/main.rs
compare @/absolute/file.rs @"path with spaces.txt"
/review focus on error handling @src/api.rs
```

Type `@` to search project files by fuzzy path/name. The scan runs off the UI
thread, respects ignore files, and excludes `.git`. `@/`, `@~/`, `@./`, and `@../`
browse directories instead (including paths outside the project). Use ↑/↓ to
choose, Tab to insert, Esc to dismiss. Enter also accepts a partial file match
**without sending**; an exact file path or a finished quoted mention sends
normally. Selection quotes filenames with spaces automatically.

Mentions are whitespace-delimited; double-quote paths containing spaces. Emails
aren't mentions, and `@@name` stays verbatim without expansion. Only explicitly
mentioned files are read. Text must be UTF-8 without NUL bytes: at most 1 MiB per
file/skill and 4 MiB total context per message. Missing/binary/oversized files
produce an error and keep your draft, rather than silently sending without them.
Images still use the bash `view_image` bridge.

`/skill-name [request]` injects the full `SKILL.md` contents along with your request.
Names come from the `name:` frontmatter field, falling back to the skill directory
name. Local skills override global names; built-in commands take precedence.
The skill catalog refreshes on startup, `/new`, and `/resume`.

Typing `/` completes both built-ins and skills. **Enter executes an exact or
unique prefix immediately** (`/q` quits, `/mo model effort` sets the model).
Ambiguous prefixes require more typing, Tab, or an explicit ↑/↓ selection;
Tab only fills the command, never runs it.

Files and invoked skills are snapshotted **on submission**, including while
queuing steering. Their contents are appended to that **user message**, never
the system prompt. Resuming, branching, or replaying history doesn't reread them.
Attached contents aren't recursively expanded for further mentions. The transcript
and `/copy user` retain the short message you typed, not the injected contents.

## What goes on the wire

HTTP SSE `POST <MU_BASE_URL>/responses`, via curl. Each request replays only the
selected path with `store: false`, the same instructions, and the single bash
tool. Completed Responses items are preserved, including encrypted reasoning.
When effort is set, `reasoning.summary: auto` requests streaming summaries.
Thought visibility depends on what the proxy/model exposes; mu cannot recover
hidden chain-of-thought.

Only these optional sections are added to the system prompt:

```text
Project Instructions:
[ancestor AGENTS.md files, outermost first, through the working directory]

Skills:
/absolute/path/SKILL.md: description
```

Skills are discovered in `~/.agents/skills` and ancestor `.agents/skills`
directories. Descriptions come from the `description:` frontmatter field
(single-line or indented block). Skill bodies are **not** injected into the system
prompt; `/skill-name` explicitly attaches them to a user message. Both sections
are snapshotted when creating a session so later file edits don't silently
invalidate its prompt prefix.

Bash runs in a fresh shell in the session directory, without interactive startup
files. `cd` and environment changes don't persist between calls. Default timeout:
120 seconds; stdout/stderr: first 64 KiB combined, with truncation/exit markers.
Timeout or cancellation kills the process group. Deliberately detached processes
may outlive it. There are no automatic HTTP retries or automatic tool replays.

Inside bash, `view_image /path/to/image` queues an image for the next request.
The bridge accepts PNG/JPEG/GIF/WebP (up to 20 MiB each, eight per call), returns a
textual tool result, then attaches Responses `input_image` content in a user
message after the tool results. The selected model must support images.

The status line shows the **last response's** input/output tokens, cached input
and read percentage, then `(input + output) / MU_CONTEXT` in thousands. `?` means
cache telemetry was omitted. Capacity is not inferred from a model name and no
context trimming happens. A one-line cache-miss warning appears only when an
explicitly zero cache read follows a nonzero read on a growing context; a first
request or missing telemetry is not called a miss.

Markdown is intentionally just a colorizer. Clipboard uses `wl-copy`, then
`xclip`, when available; otherwise OSC 52 (your terminal must allow it).

## Check

```sh
nix develop --command cargo test
nix develop --command cargo clippy --all-targets -- -D warnings
nix develop --command cargo build --release
nix develop --command python3 tests/smoke.py
nix build
```

The smoke test drives a real PTY against a local fake Responses server: streaming,
steering, image transport, cache warnings, branching, resume, cancellation,
clipboard, incomplete streams, file/skill snapshots, completion, and resize. It
needs no API credentials.

About 2.4k lines of Rust, including unit tests. Measured on x86_64-linux: **765 KiB**
stripped release ELF and **2.7 MiB** idle RSS. Those figures exclude external
curl/bash/rg processes and their libraries; history/images grow memory and storage.
