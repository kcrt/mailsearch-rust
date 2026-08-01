# mailsearch

A fast full-text search tool for Apple Mail `.emlx` files with an interactive terminal UI.

![Screenshot](materials/screenshot.webp)

## Features

- **Fast full-text search** - Search through email content (subject, body, headers) for multiple terms with AND logic
- **Interactive TUI** - Browse and view results with a rich terminal interface built with Ratatui
- **Search highlighting** - Matching search terms are highlighted in yellow bold text
- **Recent-mail search** - `--this-week` / `--days N` restrict the search to a recent period, skipping most files without reading them. On a 230k-message mailbox this cuts CPU time by roughly 8x. See [Date windows](#date-windows).
- **Flexible sorting** - Sort results by date (ascending/descending), subject, from, or to fields, interactively or via the `--sort` flag
- **JSON output** - `--json` prints results to stdout for scripting and integration with other tools
- **Advanced filtering** - Filter results by sender, recipient, subject, date range, or full-text search
- **macOS integration** - QuickLook preview and open emails with default system applications
- **Performance optimized** - Parallel processing for unlimited searches, sequential with early termination for limited results
- **Smart email parsing** - Handles both plain text and HTML emails, strips HTML/CSS/JavaScript, preserves embedded newlines
- **Comprehensive metadata** - Displays From, To, Cc, Subject, and Date for each email; `--json` also emits `message_id` and a `timestamp`, so downstream tools can address a message without re-parsing the file

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

- `<QUERY>` - Search terms (space-separated, AND logic applied). Combine with `--or` for OR search (see below).

  Terms are matched against the `Subject`, `From`, `To`, `Cc` and `Reply-To` headers as well as the body, so an address that never appears in the text is still findable (`mailsearch noreply@example.com`). A single AND-group may take one term from a header and another from the body.

### Options

- `-o, --or <TERMS>` - Add an OR group (repeatable). Each group is AND-matched internally, and groups are OR-combined. The whole search still runs in a single scan. See [OR search](#or-search).
- `-r, --mail-root <DIR>` - Path to Apple Mail directory (default: `~/Library/Mail/V10`)
- `-l, --limit <N>` - Limit number of results (default: unlimited)
- `--this-week` - Only search mail from the last 7 days. Shorthand for `--days 7`. See [Date windows](#date-windows).
- `--days <N>` - Only search mail from the last N days (max 36500). `--days 0` means today only. Cannot be combined with `--this-week`.
- `--sort <ORDER>` - Sort results before display/output. One of `none` (default), `date-asc`, `date-desc`, `subject`, `from`, `to`. When combined with `--limit`, results are sorted first and then truncated (i.e. the top-N).
- `--json` - Print results to stdout as a JSON array instead of launching the TUI. Each entry contains `subject`, `from`, `to`, `cc`, `date`, `timestamp` (Unix epoch seconds, `null` if the date could not be determined), `message_id` (angle brackets included, `null` if the message has none — e.g. an unsent draft), and `path`. The message body is omitted.

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
mailsearch -r ~/Library/Mail/V2 "project update"
```

Output the 10 most recent matches as JSON (for scripting):

```bash
mailsearch --json --sort date-desc --limit 10 "receipt invoice" | jq '.[].subject'
```

Search only recent mail (much faster on a large mailbox):

```bash
mailsearch --this-week meeting
mailsearch --days 30 --sort date-desc invoice
```

### Date windows

`--this-week` and `--days N` restrict the search to mail from a recent period. Because most files can be ruled out without being read, this is also the single biggest speedup available — on a 230,000-message mailbox, `--this-week` reduced CPU time from ~67s to ~8s.

Filtering happens in two stages:

1. **File modification time**, checked while walking the directory. For Apple Mail's `.emlx` files the file is written at or after the message date, and later edits only move its mtime forward, so a file older than the cutoff cannot hold a message inside the window and is skipped unread. This is what makes the window fast.
2. **The `Date:` header**, checked after parsing. This stage is required for correctness: changing a flag or moving a message rewrites the file, so a good fraction of the surviving candidates are old mail with a freshly bumped mtime. In practice roughly a third of stage-1 survivors are rejected here.

Things worth knowing:

- **The window is deliberately a little wider than requested.** The cutoff is midnight at the start of the day, so `--days 7` run in the afternoon covers 7 days plus that afternoon. The file-side cutoff (stage 1) is loosened by a further 2 days, so mail from a sender whose clock runs fast is not dropped unread.
- **`--days 0` means today**, not "no window". Omitting both flags is how you search everything.
- **Mail with a future date always matches**, since the window has no upper bound. Some spam sets its `Date:` header years ahead.
- **Mail with no readable `Date:` header falls back to the file's mtime**, so it is included when the file itself is recent rather than being silently dropped.
- **`--limit` alone does not give you the newest N.** It truncates in filesystem walk order. For the most recent matches, combine it with `--sort date-desc`.
- **Dates are displayed in UTC**, including the `since ...` line printed at startup, so they may differ from your wall clock.

### OR search

By default, multiple words in the query are combined with **AND** (`"Hello world"` matches messages containing both `Hello` and `world`). To search with **OR**, add one or more `--or` (`-o`) groups. Everything is evaluated in a single scan, so OR search costs the same as one AND search — no need to run the command multiple times.

The logic is a disjunction of conjunctions (DNF): spaces mean AND *within* a group, and groups are joined by OR.

```bash
# hello AND world
mailsearch "hello world"

# hello OR world OR today
mailsearch hello --or world --or today

# (urgent AND invoice) OR (至急 AND 請求)
mailsearch "urgent invoice" --or "至急 請求"
```

All matched terms across every group are highlighted in the TUI.

## TUI Controls

| Key | Action |
| :--- | :--- |
| `↑` / `↓` or `j` / `k` | Navigate results |
| `s` | Cycle sort order (no sort / date ↑ / date ↓ / subject / from / to) |
| `Enter` | Open email in default application |
| `Space` | QuickLook preview |
| `/` | Enter filter mode |
| `Esc` | Exit filter mode or clear active filter |
| `PgUp` / `PgDn` | Scroll content preview |
| `q` | Quit |

### Filter Mode

Press `/` to enter filter mode and type filters to refine results:

| Filter | Example | Description |
| :--- | :--- | :--- |
| (plain text) | `Hello` | Search across all fields (from, subject, content) |
| `from:` | `from:alice` | Filter by sender |
| `subject:` | `subject:meeting` | Filter by subject |
| `to:` | `to:bob@example.com` | Filter by recipient |
| `after:` | `after:2025-01-01` | Filter by date after (inclusive) |
| `before:` | `before:2025-12-31` | Filter by date before (inclusive) |

**Filter Tips:**

- Use quotes for multi-word values: `subject:"project update"`
- Combine multiple filters: `from:alice subject:meeting after:2025-01-01`
- Plain text searches across sender, subject, AND content
- Press `Enter` to apply, `Esc` to cancel
- Text filters are case-insensitive
- `after:` / `before:` compare against the date shown in the results list, which is UTC
- Clear active filter by pressing `Esc` in normal mode

## Requirements

- macOS (Apple Mail stores emails in macOS-specific format)
- Full Disk Access permission for Terminal or your terminal emulator

### Granting Full Disk Access

If you see permission errors, grant Full Disk Access:

1. Open **System Settings** > **Privacy & Security** > **Full Disk Access**
2. Add your terminal application (Terminal.app, iTerm2, etc.)
3. Restart your terminal

## How It Works

1. **Discovery** - Recursively finds all `.emlx` files in the mail directory. With `--this-week` / `--days N`, files whose modification time predates the window are dropped here without being read (see [Date windows](#date-windows))
2. **Parsing** - Extracts headers and body content from each email, handling both plain text and HTML. The `Date:` header is resolved first, so mail outside the window is discarded before the expensive body extraction
3. **Search** - Searches extracted content for the query terms (AND within a group, OR across `--or` groups)
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

### Key Dependencies

- `clap` - CLI argument parsing
- `mailparse` - `.emlx` / MIME parsing
- `chrono` - date parsing, formatting, and the `--this-week` / `--days` windows
- `walkdir` - recursive mail directory traversal
- `ratatui` + `crossterm` - interactive terminal UI
- `rayon` - parallel search
- `indicatif` - progress bars and spinners
- `serde` + `serde_json` - JSON output (`--json`)

## TODO

- [ ] Improve search performance further (date windows help a lot, but an unrestricted search still reads every file)
- [ ] Add more sort options (e.g., by attachment count)
- [ ] Export search results to file
- [ ] Save and load search queries

## License

MIT
