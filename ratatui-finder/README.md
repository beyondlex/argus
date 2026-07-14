# ratatui-finder

A macOS Finder-style "Go to Path" directory navigation component for [ratatui](https://ratatui.rs) TUI applications.

## Features

- **Interactive path browser** — type a path, get instant listings and fuzzy-matched results
- **Three modes**: listing (`/`-ending), auto-listing (existing directory), matching (partial name)
- **Fuzzy matching** with first-character filter and `*`-wildcard substring search
- **Tab completion** — autocomplete the selected item name
- **Parent navigation** — `Ctrl-w` jumps to parent directory
- **Fully self-contained** — no dependency on any framework, just pure data structures + `Widget` rendering

## Quick Start

```toml
[dependencies]
ratatui-finder = { path = "../ratatui-finder" }
```

```rust
use ratatui_finder::{FinderState, FinderConfig, FinderMode, render_finder_popup};

let mut state = FinderState::new(FinderConfig {
    mode: FinderMode::Dir,          // show directories only
    initial_path: "~".to_string(),
    extensions: None,
});

// In your event loop:
let action = state.handle_key(key_event);

// In your render function:
render_finder_popup(f, area, &mut state);
```

## Key Bindings

| Key | Action |
|---|---|
| Character input | Append to input, refresh results |
| `Enter` | Confirm selected path |
| `Esc` / `Ctrl-c` | Cancel |
| `Tab` | Complete selected item name |
| `Up` / `Down` or `Ctrl-p` / `Ctrl-n` | Navigate results |
| `Ctrl-w` | Go to parent directory |
| `Ctrl-u` | Clear input |
| `Home` / `End` or `Ctrl-a` / `Ctrl-e` | Cursor to start/end |
| `Left` / `Right` | Move cursor |
| `Backspace` / `Delete` | Delete character |

## API

### Types

- **`FinderState`** — main state machine holding input, cursor, results, and config
- **`FinderConfig`** — configuration: `mode` (Dir/File/Both), `initial_path`, `extensions`
- **`FinderMode`** — filter mode: `Dir`, `File`, `Both`
- **`FinderAction`** — feedback to host: `None`, `Confirm(String)`, `Cancel`, `Redraw`
- **`FinderItem`** — a result item with match positions for highlighting

### Functions

- `FinderState::new(config)` — create a new instance
- `state.handle_key(key)` — process a key event, returns `FinderAction`
- `state.refresh()` — force refresh after external input changes
- `render_finder_popup(f, area, &mut state)` — render the finder UI in a ratatui frame

## Integration Example

The popup is rendered as an overlay. Host applications should:

1. Create a `FinderState` with desired config
2. Route keyboard events to `state.handle_key()`
3. Match on `FinderAction` to confirm/cancel/nop
4. Call `render_finder_popup()` in the rendering pass over the full terminal area

See [argus-tui](../argus-tui/) for a complete integration example (`:finder` command).