# TUI Filter Feature Documentation

## Overview
The TUI now includes an interactive filtering feature that allows users to refine search results without re-running the entire search.

## Key Bindings

### Normal Mode
- `/` - Enter filter mode
- `Esc` - Clear active filter (or quit if no filter is active)
- `↑`/`↓` or `j`/`k` - Navigate results
- `Enter` - Open selected email
- `Space` - QuickLook preview
- `PgUp`/`PgDn` - Scroll content
- `q` - Quit

### Filter Input Mode
- `Esc` - Cancel filter mode without applying
- `Enter` - Apply the filter
- `Ctrl+U` - Clear entire filter input
- `Backspace` - Delete last character
- Any character - Add to filter input

## Filter Syntax

### Sender Filter
Filter by sender email or name (case-insensitive):
```
from:john
from:"John Doe"
```

### Subject Filter
Filter by subject line (case-insensitive):
```
subject:meeting
subject:"project update"
```

### Date Filters
Filter by date (format: YYYY-MM-DD):
```
after:2025-01-01
before:2026-01-20
```

### Combined Filters
Use multiple filters together (AND logic):
```
from:john subject:meeting
from:alice after:2025-01-01
subject:"project update" before:2026-01-01
```

## Usage Examples

1. **Filter by sender**
   - Press `/`
   - Type: `from:john`
   - Press `Enter`
   - Results now show only emails from senders matching "john"

2. **Filter by subject with spaces**
   - Press `/`
   - Type: `subject:"project update"`
   - Press `Enter`
   - Results now show only emails with "project update" in subject

3. **Filter by date range**
   - Press `/`
   - Type: `after:2025-01-01 before:2025-12-31`
   - Press `Enter`
   - Results now show only emails from 2025

4. **Complex filter**
   - Press `/`
   - Type: `from:alice subject:meeting after:2025-01-01`
   - Press `Enter`
   - Results show emails from Alice about meetings since Jan 1, 2025

5. **Clear filter**
   - Press `Esc` (while not in filter mode)
   - All original results are restored

## UI Changes

### Results List Title
When no filter is active:
```
 Results for: rust programming (150)
```

When filter is active:
```
 Results for: rust programming (150 filtered: 20)
```

### Help Bar (Normal Mode)
```
 ↑↓  Navigate  Space  QuickLook  Enter  Open  /  Filter  q  Quit
```

### Help Bar (Filter Input Mode)
```
 /  Filter  Esc  Cancel  Enter  Apply  Ctrl+U  Clear
Filter: from:john subject:meeting
```

## Implementation Details

### New App Fields
- `filter_input: String` - Current filter text being typed
- `filter_mode: bool` - Whether in filter input mode
- `filtered_indices: Option<Vec<usize>>` - Indices of filtered results (None = no filter)

### Filter Types Supported
- `from:pattern` - Sender filter
- `subject:pattern` - Subject filter
- `after:YYYY-MM-DD` - Date after filter
- `before:YYYY-MM-DD` - Date before filter

### Key Features
- Case-insensitive text matching
- Quoted string support for multi-word patterns
- Multiple filters with AND logic
- Maintains original results (filters are non-destructive)
- Navigation works correctly with filtered results
