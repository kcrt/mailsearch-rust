# Filter Feature - Implementation Diagram

## Architecture Overview

```
┌─────────────────────────────────────────────────────────────┐
│                         User Input                          │
│                   (Keyboard Events)                         │
└────────────────────┬────────────────────────────────────────┘
                     │
                     ▼
┌─────────────────────────────────────────────────────────────┐
│                    Event Handler                            │
│                                                             │
│  Normal Mode:              Filter Mode:                     │
│  • '/' → enter_filter_mode  • Esc → exit_filter_mode       │
│  • 'q' → quit               • Enter → apply_filter          │
│  • Esc → clear_filter       • Backspace → delete_char       │
│  • j/k → navigate           • Ctrl+U → clear_input         │
│  • Enter → open             • char → add_char               │
└────────────────────┬───────────────────┬────────────────────┘
                     │                   │
                     ▼                   ▼
┌──────────────────────────┐   ┌──────────────────────────┐
│   Navigation Logic       │   │   Filter Logic           │
│                          │   │                          │
│  • next()                │   │  • parse_filter()        │
│  • previous()            │   │  • apply_filter()        │
│  • visible_results_count()│   │  • match_filter()        │
│  • selected_result()     │   │                          │
└────────────┬─────────────┘   └────────────┬─────────────┘
             │                               │
             │         ┌─────────────────────┘
             │         │
             ▼         ▼
┌─────────────────────────────────────────────────────────────┐
│                        App State                            │
│                                                             │
│  • results: Vec<SearchResult>     (original results)        │
│  • query: String                   (search query)           │
│  • selected: usize                 (current selection)      │
│  • filter_input: String            (current filter text)    │
│  • filter_mode: bool               (in filter mode?)        │
│  • filtered_indices: Option<Vec<usize>>  (filter results)   │
└────────────────────┬────────────────────────────────────────┘
                     │
                     ▼
┌─────────────────────────────────────────────────────────────┐
│                      UI Renderer                            │
│                      (draw_ui)                              │
│                                                             │
│  ┌───────────────────────────────────────────────────────┐ │
│  │  Results List                                         │ │
│  │  - Shows filtered or all results                      │ │
│  │  - Title shows filtered count                         │ │
│  └───────────────────────────────────────────────────────┘ │
│  ┌───────────────────────────────────────────────────────┐ │
│  │  Content Preview                                      │ │
│  │  - Shows selected result content                      │ │
│  └───────────────────────────────────────────────────────┘ │
│  ┌───────────────────────────────────────────────────────┐ │
│  │  Help/Filter Bar                                      │ │
│  │  - Normal mode: shows help                            │ │
│  │  - Filter mode: shows filter input                    │ │
│  └───────────────────────────────────────────────────────┘ │
└─────────────────────────────────────────────────────────────┘
```

## Filter Pipeline

```
User Input: "from:alice subject:meeting after:2025-01-01"
    │
    ▼
┌─────────────────────────────────────────┐
│   parse_filter()                        │
│   Split by spaces, handle quotes        │
└────────────┬────────────────────────────┘
             │
             ▼
┌─────────────────────────────────────────┐
│   parse_single_filter() for each token │
│   Extract type and value                │
└────────────┬────────────────────────────┘
             │
             ▼
    ┌────────────────────────┐
    │  FilterType::From      │
    │  value: "alice"        │
    └────────────────────────┘
    ┌────────────────────────┐
    │  FilterType::Subject   │
    │  value: "meeting"      │
    └────────────────────────┘
    ┌────────────────────────┐
    │  FilterType::After     │
    │  value: 2025-01-01     │
    └────────────────────────┘
             │
             ▼
┌─────────────────────────────────────────┐
│   Apply filters to results              │
│   For each result:                      │
│     if all filters match:               │
│       add index to filtered_indices     │
└────────────┬────────────────────────────┘
             │
             ▼
┌─────────────────────────────────────────┐
│   filtered_indices: Some([2, 5, 7, 12]) │
└─────────────────────────────────────────┘
```

## State Transitions

```
┌──────────────┐
│ Normal Mode  │
│ No Filter    │
└──────┬───────┘
       │ Press '/'
       ▼
┌──────────────┐
│ Filter Mode  │
│ Typing...    │
└──────┬───────┘
       │ Press Enter
       ▼
┌──────────────┐     Press '/'
│ Normal Mode  ├───────────────┐
│ With Filter  │               │
└──────┬───────┘               │
       │ Press Esc             │
       ▼                       ▼
┌──────────────┐        ┌──────────────┐
│ Normal Mode  │        │ Filter Mode  │
│ No Filter    │        │ Editing...   │
└──────────────┘        └──────────────┘
```

## Data Flow

```
SearchResults (Original)
    │
    │   [0] alice@... "Meeting notes" 2025-01-15
    │   [1] bob@... "Project update" 2025-01-14
    │   [2] alice@... "Team meeting" 2025-01-13
    │   [3] charlie@... "Vacation" 2025-01-12
    │   [4] alice@... "Weekly meeting" 2025-01-11
    │   ...
    │
    ▼ Apply filter: "from:alice subject:meeting"
    │
FilteredIndices
    │
    │   Some([0, 2, 4])  ─┐
    │                     │
    └─────────────────────┼───► Display Only These
                          │
    ┌─────────────────────┘
    │
    ▼
Displayed Results
    │
    │   [0] alice@... "Meeting notes" 2025-01-15
    │   [1] alice@... "Team meeting" 2025-01-13
    │   [2] alice@... "Weekly meeting" 2025-01-11
    │
    └──► User navigates and views these
```

## Key Methods

### Filter Parsing
```rust
fn parse_filter(input: &str) -> Vec<FilterType>
    ├── Handles quoted strings: "project update"
    ├── Splits by whitespace
    └── Calls parse_single_filter() for each token

fn parse_single_filter(token: &str) -> Option<FilterType>
    ├── Splits on ':' to get type and value
    ├── Matches type: from, subject, after, before
    └── Returns appropriate FilterType variant
```

### Filter Matching
```rust
fn match_filter(filter: &FilterType, result: &SearchResult) -> bool
    ├── From: case-insensitive contains
    ├── Subject: case-insensitive contains
    ├── After: date >= filter_date
    └── Before: date <= filter_date
```

### App Methods
```rust
impl App {
    fn apply_filter(&mut self)
        ├── Parse filter_input
        ├── For each result, check all filters
        ├── Store matching indices
        └── Reset selection to 0
    
    fn selected_result(&self) -> Option<&SearchResult>
        ├── If filtered: use filtered_indices[selected]
        └── Else: use results[selected]
    
    fn visible_results_count(&self) -> usize
        ├── If filtered: return filtered_indices.len()
        └── Else: return results.len()
}
```

## Filter Types

```rust
enum FilterType {
    From(String),      // from:alice
    Subject(String),   // subject:meeting
    After(NaiveDate),  // after:2025-01-01
    Before(NaiveDate), // before:2025-12-31
}
```

## Testing Coverage

```
✅ test_parse_filter_from          - Parse "from:john"
✅ test_parse_filter_subject       - Parse "subject:meeting"
✅ test_parse_filter_with_quotes   - Parse "subject:\"project update\""
✅ test_parse_filter_multiple      - Parse "from:john subject:meeting"
✅ test_parse_filter_date          - Parse "after:2025-01-01"
✅ test_match_filter_from          - Match sender
✅ test_match_filter_subject       - Match subject
✅ test_match_filter_after         - Match date after
✅ test_match_filter_before        - Match date before
```

## Performance Characteristics

- **Filter Parsing**: O(n) where n = length of filter string
- **Filter Application**: O(m * f) where m = number of results, f = number of filters
- **Navigation**: O(1) - uses indices
- **Memory**: O(k) where k = number of matching results (stores indices only)
- **Non-destructive**: Original results always preserved in memory
