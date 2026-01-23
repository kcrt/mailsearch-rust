# Apple Mail Storage Structure Comprehensive Review

  * This is AI generated, not human reviewed.

---

## 1. Directory Structure

### Location
```
~/Library/Mail/V10/
```

### Directory Layout
```
~/Library/Mail/V10/
├── MailData/                    # Database and index files
│   ├── Envelope Index           # Main SQLite database
│   ├── Envelope Index-shm       # Shared memory file
│   └── Envelope Index-wal       # Write-Ahead Log
├── {Account-ID}/                # Per-account directories
│   ├── {Mailbox}.mbox/          # Each mailbox folder
│   │   ├── {ROWID}.emlx        # Individual message files
│   │   ├── {ROWID}.emlx
│   │   └── ...
│   └── ...
└── ...
```

### Version History
- V2: Earlier macOS versions
- V5: macOS 10.11-10.14
- V6-V9: Intermediate versions
- **V10**: Current macOS (Sequoia, Sonoma, etc.)

---

## 2. Envelope Index Database (SQLite)

### Database Location
```
~/Library/Mail/V10/MailData/Envelope Index
```

### Access Command
```bash
sqlite3 ~/Library/Mail/V10/MailData/Envelope\ Index
```

### Main Tables

| Table | Purpose |
|-------|---------|
| `addresses` | Email addresses (sender/recipients) |
| `messages` | Message metadata and relationships |
| `recipients` | Message-to-address mappings |
| `subjects` | Email subjects (normalized) |
| `mailboxes` | Folder/inbox information |
| `attachments` | Attachment metadata |
| `threads` | Conversation threading |
| `associations` | Various associations |
| `properties` | Extended properties |
| `alarms` | Reminder/alarms |
| `calendars` | Calendar integration |
| `feeds` | RSS feed data |
| `todo_notes` | Notes integration |
| `todos` | Task/todo items |

### Messages Table Schema (Key Columns)

| Column | Type | Description |
|--------|------|-------------|
| `ROWID` | INTEGER | Primary key, used as .emlx filename |
| `sender` | INTEGER | Foreign key to addresses table |
| `subject` | INTEGER | Foreign key to subjects table |
| `date` | INTEGER | Unix timestamp |
| `date_sent` | INTEGER | Unix timestamp (when sent) |
| `date_received` | INTEGER | Unix timestamp (when received) |
| `mailbox` | INTEGER | Foreign key to mailboxes table |
| `flags` | INTEGER | Message flags (read, flagged, etc.) |
| `type` | INTEGER | Message type |
| `conversation_id` | INTEGER | Thread/conversation grouping |
| `message_id` | TEXT | RFC 822 Message-ID header |
| `size` | INTEGER | Message size in bytes |

### Other Key Tables

#### Addresses Table
| Column | Description |
|--------|-------------|
| `ROWID` | Primary key |
| `comment` | Email address string |
| `name` | Display name |

#### Recipients Table
| Column | Description |
|--------|-------------|
| `message_id` | Foreign key to messages |
| `address_id` | Foreign key to addresses |
| `type` | To/Cc/Bcc |

---

## 3. EMLX File Format

### File Structure
Each `.emlx` file consists of **three parts**:

```
┌─────────────────────────────────────┐
│  1. Byte Count Header (ASCII)       │
│     - Line 1: Total bytes in message│
│     - Terminated by 0x0a (newline) │
├─────────────────────────────────────┤
│  2. Email Content (MIME/RFC 5322)   │
│     - Headers (From, To, Subject...)│
│     - Body (plain text and/or HTML) │
│     - Attachments                   │
├─────────────────────────────────────┤
│  3. Apple Property List (plist)     │
│     - Binary or XML plist format    │
│     - Mail.app metadata             │
└─────────────────────────────────────┘
```

### Part 1: Byte Count Header
```
5247
```
The first line is the byte count in ASCII decimal, followed by a newline.

### Part 2: MIME Message Content
Standard RFC 5322/RFC 822 format:
```
From: sender@example.com
To: recipient@example.com
Subject: Test Email
Content-Type: text/plain; charset=utf-8

This is the email body...
```

### Part 3: Apple Plist Metadata
Binary or XML plist containing:
```xml
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" ...>
<plist version="1.0">
<dict>
    <key>flags</key>
    <dict>
        <key>read</key>
        <true/>
        <key>answered</key>
        <false/>
        <key>flagged</key>
        <false/>
    </dict>
    <key>color</key>
    <string>000000</string>
    <key>conversation-id</key>
    <integer>12345</integer>
    <key>date-last-viewed</key>
    <integer>1580423184</integer>
</dict>
</plist>
```

---

## 4. Key Flags and Metadata

### Message Flags
| Flag | Description |
|------|-------------|
| `read` | Message has been opened |
| `answered` | Message has been replied to |
| `flagged` | Message is starred/flagged |
| `attachment_count` | Number of attachments |
| `forwarded` | Message has been forwarded |
| `junk` | Marked as spam |
| `deleted` | Marked for deletion |

---

## 5. Access Considerations

### macOS Permissions
The Mail directory is protected by **Full Disk Access**. To access:

1. **Grant Full Disk Access** to your terminal or IDE:
   - System Settings → Privacy & Security → Full Disk Access
   - Add Terminal, VSCode, or your application

2. **Alternative: Copy the database**:
   ```bash
   cp ~/Library/Mail/V10/MailData/Envelope\ Index ~/temp/
   sqlite3 ~/temp/Envelope\ Index
   ```