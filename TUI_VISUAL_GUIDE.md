# TUI Filter Feature - Visual Guide

## Normal Mode (No Filter Active)

```
┌─ Results for: rust programming (150) ──────────────────────────────────────┐
│ alice@example.com [2025-01-15 10:30] Rust Conference 2025                  │
│ bob@company.org [2025-01-14 14:20] RE: Rust project update                 │
│ charlie@dev.io [2025-01-13 09:15] Async Rust tutorial                      │
│ → david@test.com [2025-01-12 16:45] Memory safety in Rust                  │
│ eve@email.net [2025-01-11 11:00] Rust weekly newsletter                    │
│ frank@rust.org [2025-01-10 13:30] Cargo tips and tricks                    │
│ grace@dev.com [2025-01-09 15:20] Rust ownership explained                  │
│ henry@example.org [2025-01-08 10:00] Error handling best practices         │
│ ...                                                                         │
└─────────────────────────────────────────────────────────────────────────────┘
┌─ Content ───────────────────────────────────────────────────────────────────┐
│ From: david@test.com                                                        │
│ Date: 2025-01-12 16:45                                                      │
│ Subject: Memory safety in Rust                                              │
│                                                                             │
│ Hello team,                                                                 │
│                                                                             │
│ I wanted to share some insights about **memory safety** in **Rust**...     │
│ The ownership system ensures that memory is managed safely without a       │
│ garbage collector. This is one of the key features that makes **Rust**     │
│ unique among systems programming languages.                                 │
│                                                                             │
│ Key points:                                                                 │
│ - Ownership rules prevent data races                                        │
│ - Borrowing allows safe references                                          │
│ - Lifetime annotations ensure validity                                      │
│ ...                                                                         │
└─────────────────────────────────────────────────────────────────────────────┘
┌─────────────────────────────────────────────────────────────────────────────┐
│      ↑↓  Navigate  Space  QuickLook  Enter  Open  /  Filter  q  Quit       │
└─────────────────────────────────────────────────────────────────────────────┘
```

## Filter Input Mode (Typing Filter)

```
┌─ Results for: rust programming (150) ──────────────────────────────────────┐
│ alice@example.com [2025-01-15 10:30] Rust Conference 2025                  │
│ bob@company.org [2025-01-14 14:20] RE: Rust project update                 │
│ charlie@dev.io [2025-01-13 09:15] Async Rust tutorial                      │
│ → david@test.com [2025-01-12 16:45] Memory safety in Rust                  │
│ eve@email.net [2025-01-11 11:00] Rust weekly newsletter                    │
│ frank@rust.org [2025-01-10 13:30] Cargo tips and tricks                    │
│ grace@dev.com [2025-01-09 15:20] Rust ownership explained                  │
│ henry@example.org [2025-01-08 10:00] Error handling best practices         │
│ ...                                                                         │
└─────────────────────────────────────────────────────────────────────────────┘
┌─ Content ───────────────────────────────────────────────────────────────────┐
│ From: david@test.com                                                        │
│ Date: 2025-01-12 16:45                                                      │
│ Subject: Memory safety in Rust                                              │
│                                                                             │
│ Hello team,                                                                 │
│                                                                             │
│ I wanted to share some insights about **memory safety** in **Rust**...     │
│ The ownership system ensures that memory is managed safely without a       │
│ garbage collector. This is one of the key features that makes **Rust**     │
│ unique among systems programming languages.                                 │
│                                                                             │
│ Key points:                                                                 │
│ - Ownership rules prevent data races                                        │
│ - Borrowing allows safe references                                          │
│ - Lifetime annotations ensure validity                                      │
│ ...                                                                         │
└─────────────────────────────────────────────────────────────────────────────┘
┌─────────────────────────────────────────────────────────────────────────────┐
│         /  Filter  Esc  Cancel  Enter  Apply  Ctrl+U  Clear                │
│ Filter: from:alice subject:conference█                                      │
└─────────────────────────────────────────────────────────────────────────────┘
```

## Normal Mode (With Active Filter)

```
┌─ Results for: rust programming (150 filtered: 12) ─────────────────────────┐
│ → alice@example.com [2025-01-15 10:30] Rust Conference 2025                │
│ alice@example.com [2025-01-10 09:00] Conference registration deadline      │
│ alice@example.com [2025-01-05 14:15] Call for papers - Rust Conference     │
│ ...                                                                         │
│                                                                             │
│                                                                             │
│                                                                             │
│                                                                             │
│                                                                             │
└─────────────────────────────────────────────────────────────────────────────┘
┌─ Content ───────────────────────────────────────────────────────────────────┐
│ From: alice@example.com                                                     │
│ Date: 2025-01-15 10:30                                                      │
│ Subject: Rust Conference 2025                                               │
│                                                                             │
│ Hi everyone,                                                                │
│                                                                             │
│ I'm excited to announce that the **Rust Conference 2025** will be held     │
│ on March 15-17 in San Francisco. This is a great opportunity for the       │
│ **Rust** community to gather and share knowledge.                           │
│                                                                             │
│ The **conference** will feature:                                            │
│ - Keynotes from core team members                                           │
│ - Technical workshops                                                        │
│ - Networking sessions                                                        │
│ - Community projects showcase                                               │
│                                                                             │
│ Registration is now open at conference.rust-lang.org                        │
│ ...                                                                         │
└─────────────────────────────────────────────────────────────────────────────┘
┌─────────────────────────────────────────────────────────────────────────────┐
│      ↑↓  Navigate  Space  QuickLook  Enter  Open  /  Filter  q  Quit       │
└─────────────────────────────────────────────────────────────────────────────┘
```

## Filter Examples

### Example 1: Filter by sender
Press `/`, type `from:alice`, press Enter
- Shows only emails from senders containing "alice"

### Example 2: Filter by subject with spaces
Press `/`, type `subject:"project update"`, press Enter
- Shows only emails with "project update" in subject

### Example 3: Filter by date range
Press `/`, type `after:2025-01-10 before:2025-01-15`, press Enter
- Shows only emails between Jan 10-15, 2025

### Example 4: Combined filters
Press `/`, type `from:alice subject:conference after:2025-01-01`, press Enter
- Shows emails from Alice about conference since Jan 1, 2025

### Example 5: Clear filter
Press `Esc` (when not in filter mode)
- Returns to showing all 150 results

## Key Features

✅ **Real-time filtering** - Filter applied instantly on Enter
✅ **Non-destructive** - Original results preserved, can clear filter anytime
✅ **Case-insensitive** - Matching ignores case for text filters
✅ **Quoted strings** - Support multi-word patterns with quotes
✅ **Multiple filters** - Combine filters with AND logic
✅ **Date filtering** - Filter by date ranges using YYYY-MM-DD format
✅ **Visual feedback** - Shows filtered count in title bar
✅ **Navigation** - Selection and scrolling work correctly with filtered results
