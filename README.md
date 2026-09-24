# Argus

**Argus** — personal desktop disk intelligence tool. Scan, browse, and monitor disk usage via TUI or CLI, with an optional daemon for continuous change tracking.

## Features

- **High-performance scanning** — recursive filesystem scan with hardlink dedup and progress reporting
- **Vim-style TUI browser** — navigate, sort, search, filter, and delete files/directories
- **Dual mode** — standalone (scan in-memory) or connect to `argusd` daemon for delta overlay
- **Daemon delta monitoring** — `argusd` watches directories via `notify`, debounces events, stores in SQLite, serves delta queries over UDS
- **Safety first** — trash-by-default, permanent delete requires explicit confirmation, protected system paths
- **AI plugin architecture** (future) — gated behind feature flags, zero impact on core

## Quick Start

```bash
# Build all crates
cargo build --release

# CLI: scan a directory
cargo run --release --bin argus -- scan --path ~/Downloads

# TUI: launch from current directory
cargo run --release --bin argus-tui

# Daemon: start background monitoring
cargo run --release --bin argusd -- --daemon
```

## Architecture

```
┌─────────────────────────────────────────────────────┐
│                   Clients                            │
│  ┌──────────┐  ┌──────────┐  ┌──────────┐          │
│  │ argus-cli │  │ argus-tui│  │ argus-gui│ (future) │
│  └─────┬────┘  └────┬─────┘  └──────────┘          │
│        │            │                                │
│        └──────┬─────┘                                │
│               │ depends on                           │
│        ┌──────▼──────┐                               │
│        │  argus-core  │  (pure logic library)        │
│        └──────┬──────┘                               │
│               │ UDS IPC                              │
│        ┌──────▼──────┐                               │
│        │   argusd     │  (daemon process)            │
│        └─────────────┘                               │
└─────────────────────────────────────────────────────┘
```

- **argus-core**: pure logic library. No client/UI deps. Data structures, scanner, SQLite storage, IPC protocol.
- **argus-cli**: CLI client for scripts and quick validation.
- **argus-tui**: TUI client with ratatui + crossterm. Vim keybindings, file tree browsing, search, delta overlay.
- **argusd**: Background daemon. Watches directories, debounces file events, stores in SQLite, serves clients via Unix domain socket.

## CLI Usage

### argus (CLI client)

```bash
# Scan a directory
argus scan --path ~/Downloads

# Query delta summary from daemon SQLite
argus delta-summary --path ~/Downloads
argus delta-summary --path ~/Downloads --from_ms 1700000000000 --to_ms 1700005000000

# Request daemon event consolidation
argus consolidate

# Talk to a daemon on a non-default socket (must match argusd's --uds-path / config)
argus --uds-path /tmp/argus.sock status

# Print help
argus help
```

### argusd (Daemon)

```bash
# Start in foreground
argusd

# Single instance: a second argusd (foreground or --daemon) refuses to start
# while the PID file names a live process. `argusd stop` releases it.

# Daemonize (fork to background)
argusd --daemon

# With custom config
argusd --config /path/to/config.toml

# Set log level
argusd --log-level debug

# Stop running daemon
argusd stop

# Generate service template
argusd --generate-service launchd  # macOS launchd plist
argusd --generate-service systemd  # Linux systemd unit

# Override UDS socket path
argusd --uds-path /tmp/argus.sock
```

## TUI Usage

Launch from any directory — the TUI starts with a pure filesystem tree view.

### Keyboard Shortcuts

| Key | Action |
|-----|--------|
| `j` / `k` | Move cursor up/down |
| `l` / Right | Expand directory / enter child |
| `h` / Left | Collapse / navigate to parent |
| `H` | Collapse all children |
| `u` | Go up one directory root |
| `g` `g` | Jump to top |
| `G` | Jump to bottom |
| `.` | Set current directory as tree root |
| `s` | Scan current root |
| `o` | Toggle sort mode (Name / Size / Delta) |
| `d` | Delete (move to trash) |
| `D` | Permanently delete (requires confirmation) |
| `i` | Show file info popup |
| `y` | Copy file path to clipboard |
| `?` | Toggle help overlay |
| `/` | Enter search mode |
| `n` / `N` | Next / previous search match |
| `f` | Focus filter pane |
| `c` | Clear delta filter |
| `t` | Cycle time preset (daemon mode) |
| `R` | Reconnect to daemon |
| `Tab` | Cycle filter pane fields |
| `:` | Enter command mode |
| `q` / `Ctrl+C` | Quit |

### Command Mode (`:` prefix)

| Command | Description |
|---------|-------------|
| `:Scan` | Scan current directory |
| `:Help` | Show help overlay |
| `:Sort n` / `:Sort s` / `:Sort d` | Sort by Name / Size / Delta |
| `:sd` / `:ss` / `:sn` | Quick sort: delta / size / name |
| `:Delta <N>[k\|m\|g]` | Set delta filter threshold (e.g., `:Delta 10m`) |
| `:Delta off` | Disable delta filter |
| `:Time <N>[h\|d\|w]` | Set relative time range (e.g., `:Time 2h`) |
| `:Time HH:MM` | Absolute time today |
| `:Time MM-DD [HH:MM]` | Absolute date |
| `:Time <from> to <to>` | Custom time range |
| `:FilterClear` | Clear all filters |
| `:FilterFocus` | Focus filter pane |
| `:Consolidate` | Request daemon event consolidation |

## Configuration

Config file: `~/.config/argus/config.toml`

Only implemented options are shown; unknown/legacy sections are ignored.

```toml
[theme]
# "system" (auto-detect) | "dark" | "light"
color_scheme = "system"

[browsing]
# Scan the working directory automatically on startup
auto_scan_on_start = false

# Shared by argusd (watch_dirs/debounce/retention/consolidation)
# and the TUI (uds_path/detection_interval_secs)
[daemon]
uds_path = "/tmp/argusd.sock"
detection_interval_secs = 5   # TUI daemon auto-detection interval
# Default watch_dirs when unset: $HOME/Downloads and $HOME/Desktop
watch_dirs = [
    "/Users/you/Downloads",
    { path = "/var/log", include = "*.log", exclude = "*.gz" },
]
debounce_seconds = 10
delta_retention_days = 30

[daemon.consolidation]
sibling_threshold = 500
interval_minutes = 60

[ai]
enabled = false
model = "gpt-4o"
api_url = ""
api_key = ""
language = "en-US"
max_tokens_per_request = 4096
```

## Data Model

- **Snapshots**: session-only, in-memory. Compact arena (`FileNode` + name blob + CSR children), serialized as bincode+gzip (v4).
- **Delta events**: daemon-persisted in SQLite (`~/.config/argus/argus.db`). Events have path, delta_size, event_type (create/modify/delete/agg), and timestamp.
- **IPC**: UDS with bincode-serialized `DaemonRequest`/`DaemonResponse` enums. Length-prefixed frames.
- **Double-count prevention**: `is_agg` rows consolidate child events; SQL queries use `NOT EXISTS` to exclude descendants covered by aggregate rows.

## Project Structure

```
argus/
├── argus-core/       Core library (model, scanner, db, ipc)
├── argus-cli/        CLI client
├── argus-tui/        TUI client (ratatui + crossterm)
│   ├── src/
│   │   ├── app.rs          Central state + message handling
│   │   ├── event.rs        Event polling (keyboard, resize, messages)
│   │   ├── types.rs        App types (AppMode, SortMode, DirEntry, TreeNode)
│   │   ├── handler/        Keyboard event dispatch (browsing, command, search, prompt, finder)
│   │   ├── components/     UI widget rendering
│   │   ├── tree_ops.rs     Tree expand/collapse/sort/delete
│   │   ├── delta.rs        Delta cache construction
│   │   ├── search.rs       Fuzzy match + jump-to-next
│   │   └── ipc_client.rs   Daemon UDS communication
│   └── tests/
├── argusd/           Background daemon
│   └── src/
│       ├── watcher.rs      Filesystem event monitoring
│       ├── debounce.rs     Event merge + delay flush
│       ├── retention.rs    Periodic purge + consolidation
│       ├── ipc_server.rs   UDS query handler
│       └── daemonize.rs    Fork + PID file management
├── docs/
│   ├── requirements/  Current specifications (Chinese)
│   ├── plans/         Active design documents
│   ├── notes/         Living behavior notes
│   └── archive/       Superseded / outdated documents
└── AGENTS.md          Agent development conventions
```

## Development

```bash
# Build all
cargo build

# Run all tests
cargo test

# Format & lint
cargo fmt
cargo clippy --all-targets -- -D warnings

# Run integration tests
cargo test --test integration
```

See [AGENTS.md](AGENTS.md) for development conventions and [docs/requirements/index.md](docs/requirements/index.md) for full specs.

## License

MIT