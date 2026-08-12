# Claude Home Transcript Source

This is the source of truth for regular Claude Home chat export. Cowork uses
`local-agent-mode-sessions` metadata correlated to session-nested JSONL
transcripts instead. It
replaces the earlier approach of scanning Local Storage LevelDB files for
`chat_messages` string fragments, which never worked reliably: Claude compacts
and rotates those files, and the messages are not there in the first place.

## Where the transcripts actually are

Claude Desktop loads a conversation with:

```text
GET https://claude.ai/api/organizations/<org-uuid>/chat_conversations/<conversation-uuid>
    ?tree=True&rendering_mode=messages&render_all_tools=true&consistency=eventual
```

The renderer's Chromium HTTP disk cache keeps the full JSON response. That cached
response is the transcript. Nothing is fetched over the network and nothing is
uploaded — the app only reads files the user's own Claude Desktop already wrote.

Verified on macOS, 2026-08-11, Claude Desktop Electron profile at
`~/Library/Application Support/Claude`: 10,690 cache entries indexed, 43 unique
conversations recovered, 42 decoded end to end (the 43rd is a cached 404 for a
deleted conversation).

Checked and ruled out: Local Storage (`dframe-store` shell state only),
Session Storage, IndexedDB (`keyval-store` holds the react-query cache of account
and settings data, not messages).

## Profile locations

| Platform | Root |
| --- | --- |
| macOS | `~/Library/Application Support/Claude` |
| Windows | `%APPDATA%\Claude`, plus `%LOCALAPPDATA%\Packages\Claude_*\LocalCache\Roaming\Claude` for Store installs |
| Linux | `$XDG_CONFIG_HOME/Claude` or `~/.config/Claude` |

Underneath the root the layout is expected to match on every platform, because
it is the same Electron/Chromium profile: `Cache/Cache_Data` for entries and
`Local Storage/leveldb` for the shell mode. `Partitions/*/Cache` is also scanned.

> **Only macOS is verified.** Everything below was confirmed against a real macOS
> profile. Chromium ships two HTTP cache backends — simple cache (implemented
> here) and blockfile (not implemented) — and which one a Windows Claude
> installation uses has not been checked. A profile using blockfile will produce
> a failure message naming the backend rather than a misleading "no transcript
> found"; see `paths::detect_backend`. Confirm against a real Windows install
> before treating Windows Home export as working.

## Cache entry format

Each entry is a file named `<hash>_0` in Chromium's "simple cache" format:

```text
| SimpleFileHeader | key | stream 1 (response body) | EOF(stream 1) |
| stream 0 (response headers) | [SHA-256 of key] | EOF(stream 0) |
```

- Header magic `0xfcfb6d1ba7725c30`, 24 bytes (20 bytes of fields, padded).
- EOF magic `0xf4fa6f45970d41d8` on current builds, `…41d5` on older ones.
- Only stream 0's `stream_size` is dependable. Chromium writes `0` for stream 1
  when the body was streamed in, so the body is bounded between the end of the
  key and the start of stream 1's EOF record.
- The 32-byte SHA-256 of the key sits between stream 0's data and stream 0's EOF
  record, and only when that record sets `FLAG_HAS_KEY_SHA256` (2). Most real
  entries set it. Placing it anywhere else shifts the header slice by 32 bytes —
  which still *looks* fine, because the status and `content-encoding` are found
  by scanning and survive the shift. Verified the real placement by hashing the
  key and comparing: `sha256(key) == bytes[eof0 - 32 .. eof0]`.
- Stream 0 is a Chromium pickle. Rather than decoding its framing, the status
  line and headers are read directly from the NUL-separated fields, which is
  stable across Chromium versions.

Bodies observed from claude.ai are `content-encoding: zstd`. `gzip`, `deflate`,
`br`, and `identity` are also handled; anything else fails with a named error.

## Selecting a conversation

Opening a conversation in Claude Desktop refreshes its cache entry, so the
most recently written entry is the conversation the user last had open. Entries
are indexed by reading only the key (a small prefix read per file) and sorted
newest first, ties broken on size so a truncated response loses to a full one.

Cached error responses are excluded when choosing the target — a deleted
conversation leaves a 404 behind, and letting it become the target would hide
every conversation behind it. Status is read from a small tail read, not by
decoding the body.

An export targets **exactly one conversation** — the newest usable one, or the
one named by `conversation_id` — and retries only that conversation's own stored versions if
the newest will not decode. It never falls through to the next conversation in
the list: Chromium's live cache can hold a half-written entry, and turning that
into a successful export of a *different* chat is the worst failure this reader
can have. If every stored version fails, the export fails and says so.

The app indexes every usable cached conversation for its searchable picker.
Choosing a Home row passes its exact conversation UUID to export. "Newest" is
used only when no explicit Home conversation was selected.

A key counts as a transcript only if it carries the full
`https://claude.ai/api/organizations/<org>/chat_conversations/<uuid>` route. The
renderer cache holds third-party resources too, and matching the bare
`/chat_conversations/` fragment would let a foreign URL with conversation-shaped
JSON be picked as the newest transcript.

The profile holds tens of thousands of entries — over 10,000 on the machine this
was developed against — so the scan is spread across cores. It is deliberately
*not* cached between calls: a stale index would mean exporting the wrong
conversation. Measured at ~290 ms for 10,690 entries.

The decoded payload's `uuid` must equal the one the cache key promised; a
mismatch is treated as a failed read rather than exported under the wrong name.

Only the resource URL is matched, anchored at its start. Searching the key for
the route would accept a foreign URL that merely embeds it, such as
`https://evil.example/?next=https://claude.ai/api/organizations/...`.

## Payload shape

```jsonc
{
  "uuid": "...", "name": "...", "model": "...", "created_at": "...",
  "chat_messages": [
    {
      "sender": "human" | "assistant",
      "text": "",              // empty on current payloads
      "created_at": "...",
      "content": [ /* text | thinking | tool_use | tool_result */ ],
      "attachments": [ { "file_name", "file_size", "file_type", "extracted_content" } ],
      "files": [ { "file_name", "file_kind", "path" } ]
    }
  ]
}
```

Block-specific fields:

| `type` | Carries |
| --- | --- |
| `text` | `text` |
| `thinking` | `thinking` (not `text`) |
| `tool_use` | `name`, `input`, `message` (the status line shown on the card) |
| `tool_result` | `name`, `is_error`, `content[]` of `text` / `knowledge` / `local_resource` / `image` items |

Each entry of `content`, `attachments`, and `files` is parsed independently and
best-effort. A block whose shape changed upstream — `tool_result.content` arriving
as an object rather than an array, say — becomes an `unknown:` block carrying its
complete original JSON, rather than failing deserialization of the whole
conversation. `raw` holds the entire block, including fields the typed structs
name, so nothing is lost to a schema change.

The whole message-level `text` field is empty on current payloads; everything
lives in `content`. It is still used as a fallback whenever `content` yields no
prose — not only when it yields nothing at all — so a transitional payload
carrying flat text beside an attachment does not lose the text.

Parsing treats every field as optional. An unrecognized block type becomes
`unknown:<type>` carrying both whatever text it has and its raw JSON payload, so
a payload change costs formatting rather than content.

## Limits

The cache is best-effort, and this is the main caveat to communicate in the UI:

- A conversation never opened on this machine is not cached.
- Chromium evicts entries under pressure; old conversations disappear over time.
- The cached copy is as fresh as the last time the conversation was opened.

## Testing

`cargo test` covers the format parsing, decoding, normalization, and Markdown
rendering against synthetic fixtures. The end-to-end path against the real
profile is a manual test, since it depends on the machine's Claude Desktop data:

```bash
cd src-tauri && cargo test -- --ignored --nocapture real_profile
```
