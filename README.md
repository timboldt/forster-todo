# forster-todo

A terminal task manager implementing Mark Forster's **Final Version Perfected (FVP)** methodology, built with [ratatui](https://ratatui.rs).

FVP keeps a single, unordered list of everything you might do and helps you decide *what to do next* through a quick series of comparisons rather than up-front prioritizing. You never sort the list — you scan it.

## How FVP works

1. **Scan.** Starting from the top, the first task is automatically "dotted" (pre-selected) and becomes the *benchmark*. For each task below it you answer one question: *"Do I want to do this **more than** the benchmark?"* If yes, dot it — it becomes the new benchmark. If no, skip it.
2. **Act.** The **last dotted task** is always the one you do next.
3. **Repeat.** When you finish it, you drop back to the previous dotted task and keep working the chain; press `s` any time to re-scan for new or reconsidered tasks. (Classic FVP re-scans after every completion — here that step is manual so you can stay in flow.) When the dots run out, a fresh scan begins automatically.

The result is a self-maintaining chain of dots that always surfaces a sensible next action without formal priorities.

## Install & run

Requires a [Rust toolchain](https://rustup.rs).

```sh
# Run from source
cargo run

# Keep the task list in a specific file (handy for a project-local list)
cargo run -- --file ./tasks.txt

# Or install the binary
cargo install --path .
forster-todo
```

By default tasks are stored in your platform data directory (e.g.
`~/Library/Application Support/forster-todo/tasks.txt` on macOS,
`~/.local/share/forster-todo/tasks.txt` on Linux). Use `--file <path>` to override.

## Web view

```sh
forster-todo --web                       # serve on http://<your-ip>:9000
forster-todo --web 8080                  # or pick a port
forster-todo --web --auth me:secret      # require HTTP Basic auth
```

While the TUI runs, `--web` also serves a **web view** of the same in-memory
state. It listens on all interfaces (`0.0.0.0`), so you can open it from your
phone or another machine on the LAN at `http://<your-ip>:9000`. It offers three
tabs — **Selected** (the dotted chain, with the DO NOW task highlighted),
**Unfinished**, and **All** — and updates automatically as you work in the
terminal.

> Note: without `--auth`, anyone on your network can view and modify the list
> while the app is running. `--auth user:pass` adds HTTP Basic auth (the
> browser will prompt once). Basic auth over plain HTTP keeps casual users out,
> but credentials are only base64-encoded on the wire — don't reuse a password
> you care about.

The web view has the full functionality of the TUI:

- **Scan** — during pre-selection the page shows the FVP question ("Do you want
  to do X more than Y?") with **Yes / No / Back / Finish** buttons.
- **Act** — the DO NOW banner has **Complete** and **Scan** (resume) buttons.
- **Add a task** — appends an open task, exactly like `a` in the TUI.
- **Purge done** — backs up the task file, then removes finished tasks.
- The TUI keyboard shortcuts work on the page too (`Enter`/`→` dot, `↓` skip,
  `↑` back, `Esc` finish, `Space` complete, `s` scan).

Every action carries a version guard, so a stale browser tab can never act on
the wrong task — it just resyncs.

Architecturally, the task list and FVP state machine live in a shared core
(`src/session.rs`); the TUI and the web page are both presentation layers over
it, so they can never disagree about state. Changes made in the browser appear
in the terminal immediately, and vice versa.

## Keys

| Key | Mode | Action |
| --- | --- | --- |
| `↑` / `↓` | Scan | Move between candidates |
| `Enter` / `→` | Scan | Dot the current task (it becomes the benchmark) |
| `Esc` | Scan | Finish scanning and act on the last dotted task |
| `Space` | Action | Mark the current task done |
| `s` | Action | Resume scanning to dot more tasks |
| `a` | Any | Add tasks (sticky: `Enter` adds another, `Esc` exits) |
| `p` | Any | Purge done tasks (backs up the file first) |
| `?` | Any | Toggle help |
| `q` | Any | Save & quit |

## Storage format

Tasks are stored as plain text, one per line, so the file is easy to hand-edit while the app is closed:

```text
[x] Call the dentist          # done
[.] Write the quarterly report # dotted (pre-selected)
[ ] Buy milk                   # open
Pick up dry cleaning           # a bare line is treated as a new open task
```

- `[x]` = done, `[.]` = dotted, `[ ]` = open.
- Blank lines are ignored.
- Any line that doesn't start with a known marker is treated as a new open task — so you can add tasks just by typing them. They're normalized to `[ ] ...` on the next save.

Completed tasks are kept in the file as history and filtered out of the active
list. When the history builds up, **purge** (`p` in the TUI, "Purge done" on the
web) first copies the file to a dated backup next to it — `tasks.txt-20260712`,
with `-2`, `-3`, … suffixes if you purge more than once a day — and then removes
the `[x]` lines from the live file. Nothing is ever deleted without a backup.

## Development

```sh
cargo test      # unit tests for the FVP state machine, storage, and key handling
cargo clippy --all-targets
cargo fmt
```

The FVP algorithm lives in `src/fvp.rs` as a pure, I/O-free state machine, which keeps it fully unit-testable independent of the terminal.

## License

Licensed under the [MIT License](LICENSE).
