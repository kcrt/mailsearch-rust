# mailsearch

A fast full-text search tool for Apple Mail `.emlx` files with an interactive terminal UI.

## Features

- **Fast full-text search** - Search through email content (subject, body, headers) for multiple terms with AND logic
- **Interactive TUI** - Browse and view results with a rich terminal interface built with Ratatui
- **Search highlighting** - Matching search terms are highlighted in yellow bold text
- **macOS integration** - QuickLook preview and open emails with default system applications
- **Performance optimized** - Parallel processing for unlimited searches, sequential with early termination for limited results
- **Smart email parsing** - Handles both plain text and HTML emails, strips HTML tags, preserves embedded newlines

## Installation

```bash
cargo install --path .
```

Or build from source:

```bash
cargo build --release
```

The release binary will be at `target/release/mailsearch`.

## Usage

```bash
mailsearch [OPTIONS] <QUERY>
```

### Arguments

- `<QUERY>` - Search terms (space-separated, AND logic applied)

### Options

- `-d, --maildir <DIR>` - Path to Apple Mail directory (default: `~/Library/Mail`)
- `-l, --limit <N>` - Limit number of results (default: unlimited)

### Examples

Search with unlimited results:

```bash
mailsearch "rust programming"
```

Search with limited results:

```bash
mailsearch -l 20 "receipt invoice"
```

Search a custom mail directory:

```bash
mailsearch -d ~/Library/Mail/V2 "project update"
```

## TUI Controls

| Key | Action |
|-----|--------|
| `↑` / `↓` or `j` / `k` | Navigate results |
| `Enter` | Open email in default application |
| `Space` | QuickLook preview |
| `/` | Enter filter mode |
| `Esc` | Exit filter mode or clear active filter |
| `PgUp` / `PgDn` | Scroll content preview |
| `q` | Quit |

### Filter Mode

Press `/` to enter filter mode and type filters to refine results:

| Filter | Example | Description |
|--------|---------|-------------|
| `from:` | `from:alice` | Filter by sender |
| `subject:` | `subject:meeting` | Filter by subject |
| `after:` | `after:2025-01-01` | Filter by date after |
| `before:` | `before:2025-12-31` | Filter by date before |

**Filter Tips:**
- Use quotes for multi-word values: `subject:"project update"`
- Combine multiple filters: `from:alice subject:meeting after:2025-01-01`
- Press `Enter` to apply, `Esc` to cancel
- Text filters are case-insensitive
- Clear active filter by pressing `Esc` in normal mode

See [FILTER_FEATURE.md](FILTER_FEATURE.md) for detailed filter documentation.

## Requirements

- macOS (Apple Mail stores emails in macOS-specific format)
- Full Disk Access permission for Terminal or your terminal emulator

### Granting Full Disk Access

If you see permission errors, grant Full Disk Access:

1. Open **System Settings** > **Privacy & Security** > **Full Disk Access**
2. Add your terminal application (Terminal.app, iTerm2, etc.)
3. Restart your terminal

## How It Works

1. **Discovery** - Recursively finds all `.emlx` files in the mail directory
2. **Parsing** - Extracts headers and body content from each email, handling both plain text and HTML
3. **Search** - Searches extracted content for all query terms (AND logic)
4. **Display** - Shows results in interactive TUI with highlighted matches

## Development

```bash
# Run tests
cargo test

# Run with debug output
cargo run -- "query"

# Build optimized release
cargo build --release
```

## TODO

- [ ] Show spinner during initial emlx file search
- [ ] Improve search performance
- [x] Add filtering functionality in TUI ✅
- [ ] Display date in content pane (currently shown only in upper listing pane)
- [ ] Fix JIS character encoding display
- [ ] Strip CSS from HTML content
- [x] Implement tests ✅ (9 tests for filter functionality)

## License

MIT
