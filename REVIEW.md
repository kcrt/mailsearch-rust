# Apple Mail Storage Structure Comprehensive Review

## Executive Summary

Apple Mail stores messages in a hybrid system combining a SQLite database for metadata/indexing and individual `.emlx` files for actual message content. This review covers the V10 storage format and provides guidance for building full-text search and text embedding applications.

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

---

## 6. Python Tools for Parsing

### emlx Library
**Repository**: [mikez/emlx](https://github.com/mikez/emlx)

```bash
pip install emlx
```

```python
import emlx

m = emlx.read("12345.emlx")

# Access headers
m.headers
# {'Subject': 'Re: Test', 'From': '...', 'Date': '...'}

# Access content
m.text   # Plain text body
m.html   # HTML body

# Access metadata
m.plist  # Full plist as dict
m.flags  # {'read': True, 'answered': True, ...}
```

### Using Standard Library (email module)
```python
import email
import plistlib

def read_emlx(path: str) -> dict:
    with open(path, 'rb') as f:
        bytecount = int(f.readline())
        message_data = f.read(bytecount)
        plist_data = plistlib.load(f)

    msg = email.message_from_bytes(message_data)

    return {
        'headers': dict(msg.items()),
        'subject': msg['Subject'],
        'from': msg['From'],
        'to': msg['To'],
        'body': msg.get_payload(),
        'plist': plist_data
    }
```

---

## 7. MCP Servers for Apple Mail

### 1. patrickfreyer/apple-mail-mcp
**Repository**: [patrickfreyer/apple-mail-mcp](https://github.com/patrickfreyer/apple-mail-mcp)

Features:
- **Unified search** - both semantic and full-text
- **Batch operations**
- **Conversation management**
- Built with FastMCP
- AppleScript-based integration

### 2. s-morgan-jeffries/apple-mail-mcp
**Repository**: [s-morgan-jeffries/apple-mail-mcp](https://github.com/s-morgan-jeffries/apple-mail-mcp)

Features:
- Programmatic access via Model Context Protocol
- AI assistant integration

### 3. sweetrb/apple-mail-mcp
**Repository**: [sweetrb/apple-mail-mcp](https://github.com/sweetrb/apple-mail-mcp)

Features:
- Read, search, send, and manage emails

---

## 8. Building Full-Text Search

### Architecture Options

#### Option A: Direct SQLite Query
```sql
SELECT m.ROWID, s.subject, a.comment, m.date
FROM messages m
JOIN subjects s ON m.subject = s.ROWID
JOIN addresses a ON m.sender = a.ROWID
WHERE s.subject LIKE '%search term%'
   OR m.message_id LIKE '%search term%';
```

#### Option B: SQLite FTS5 Extension
```sql
CREATE VIRTUAL TABLE messages_fts USING fts5(
    subject,
    sender,
    body_content,
    content=messages,
    content_rowid=ROWID
);

-- Insert triggers for auto-indexing
```

#### Option C: External Search Engine
- **Elasticsearch** - Powerful full-text search
- **Meilisearch** - Lightweight, fast
- **Typesense** - Typo-tolerant search
- **Whoosh** - Pure Python (good for local use)

### Python Example (Whoosh)
```python
from whoosh.index import create_in, open_dir
from whoosh.fields import Schema, TEXT, ID, DATETIME
from whoosh.qparser import QueryParser

schema = Schema(
    path=ID(stored=True),
    title=TEXT(stored=True),
    content=TEXT,
    date=DATETIME
)

ix = create_in("indexdir", schema)
writer = ix.writer()

# Index emails
for emlx_file in glob("*.emlx"):
    msg = parse_emlx(emlx_file)
    writer.add_document(
        path=emlx_file,
        title=msg['subject'],
        content=msg['body'],
        date=msg['date']
    )
writer.commit()

# Search
with ix.searcher() as searcher:
    query = QueryParser("content", ix.schema).parse("search term")
    results = searcher.search(query)
```

---

## 9. Building Text Embeddings

### Architecture for Semantic Search

```
┌─────────────────┐
│   .emlx Files   │
└────────┬────────┘
         │
         ▼
┌─────────────────┐
│  Parser/Loader  │ → Extract text, headers
└────────┬────────┘
         │
         ▼
┌─────────────────┐
│  Chunking       │ → Split into manageable pieces
└────────┬────────┘
         │
         ▼
┌─────────────────┐
│  Embedding      │ → OpenAI, Cohere, local models
│  Model          │
└────────┬────────┘
         │
         ▼
┌─────────────────┐
│  Vector DB      │ → Chroma, Weaviate, Qdrant, pgvector
└─────────────────┘
```

### Python Example (ChromaDB)
```python
import chromadb
from chromadb.config import Settings
import emlx

# Initialize client
client = chromadb.PersistentClient(path="./chroma_db")
collection = client.get_or_create_collection(
    name="emails",
    metadata={"hnsw:space": "cosine"}
)

# Index emails
def index_email(emlx_path: str, embedding_function):
    m = emlx.read(emlx_path)

    # Combine subject and body for search
    text_content = f"{m.headers.get('Subject', '')}\n\n{m.text or ''}"

    collection.add(
        documents=[text_content],
        metadatas=[{
            "path": emlx_path,
            "subject": m.headers.get("Subject", ""),
            "from": m.headers.get("From", ""),
            "date": m.headers.get("Date", "")
        }],
        ids=[emlx_path]
    )

# Semantic search
def search_emails(query: str, n_results: int = 5):
    results = collection.query(
        query_texts=[query],
        n_results=n_results
    )
    return results
```

### Vector Database Options

| Database | Pros | Cons |
|----------|------|------|
| **ChromaDB** | Simple, pure Python, embedded | Less scalable |
| **Qdrant** | Fast, filtering, hybrid search | Requires separate service |
| **Weaviate** | GraphQL, modular | More complex setup |
| **pgvector** | PostgreSQL extension | Requires Postgres |
| **Pinecone** | Managed, scalable | Paid service |

---

## 10. Recommended Architecture for Your Application

### Data Pipeline
```
┌──────────────────────────────────────────────────────┐
│                     Apple Mail                       │
│  ~/Library/Mail/V10/MailData/Envelope Index          │
│  ~/Library/Mail/V10/{Account}/{Mailbox}.mbox/*.emlx  │
└──────────────────────┬───────────────────────────────┘
                       │
                       ▼
┌──────────────────────────────────────────────────────┐
│              Ingestion/Watcher Service               │
│  - Watch for new/modified .emlx files                │
│  - Parse using emlx or email module                  │
│  - Extract: headers, body, attachments               │
└──────────────────────┬───────────────────────────────┘
                       │
                       ▼
┌──────────────────────────────────────────────────────┐
│                   Processing                         │
│  - Text cleaning & normalization                     │
│  - Chunking for embeddings (if needed)               │
│  - Duplicate detection                              │
└──────────────────────┬───────────────────────────────┘
                       │
           ┌───────────┴───────────┐
           ▼                       ▼
┌────────────────────┐    ┌────────────────────┐
│  Full-Text Search  │    │  Embedding Index   │
│  (Whoosh/Elastic)  │    │  (Chroma/Qdrant)   │
└─────────┬──────────┘    └─────────┬──────────┘
          │                        │
          └────────────┬───────────┘
                       ▼
┌──────────────────────────────────────────────────────┐
│                  Query Service                       │
│  - Hybrid search (keyword + semantic)               │
│  - Reranking (RRF, CoT, etc.)                       │
│  - Filtering (date, sender, mailbox, flags)        │
└──────────────────────────────────────────────────────┘
```

### Key Considerations

1. **Permissions**: Handle Full Disk Access requirements gracefully
2. **Performance**:
   - SQLite for fast metadata queries
   - Separate indexes for full-text and vector search
   - Incremental updates (watch for new emails)
3. **Security**:
   - Don't expose credentials in embeddings
   - Sanitize content before sending to external APIs
4. **Data freshness**:
   - Implement file watching (FSEvents, watchdog)
   - Rebuild index on Mail.app restart

---

## 11. Sources

- [Mail.app Database Schema - Word to the Wise Labs](https://labs.wordtothewise.com/mailapp/)
- [Build your own mail analyzer for Mac Mail.app - JavaRants](http://www.javarants.com/build-your-own-mail-analyzer-for-mac-mail-app-747143e94ccc)
- [Apple Mail Email Format - Library of Congress](https://www.loc.gov/preservation/digital/formats//fdd/fdd000615.shtml)
- [EMLX - Apple Mail File Format - FileFormat.com](https://docs.fileformat.com/email/emlx/)
- [mikez/emlx - Python EMLX Parser](https://github.com/mikez/emlx)
- [patrickfreyer/apple-mail-mcp - MCP Server](https://github.com/patrickfreyer/apple-mail-mcp)
- [SQLite FTS5 Extension](https://www.sqlite.org/fts5.html)

---
