//! Reader for Chromium's "simple cache" entry files.
//!
//! Claude Desktop stores its `claude.ai` API responses in the renderer's HTTP
//! disk cache. Each cache entry lives in a file named `<hash>_0` laid out as:
//!
//! ```text
//! | SimpleFileHeader | key | stream 1 (response body) | EOF(stream 1) |
//! | stream 0 (response headers) | [SHA-256 of key] | EOF(stream 0) |
//! ```
//!
//! The SHA-256 of the key is present only when stream 0's EOF record sets
//! `FLAG_HAS_KEY_SHA256`; it sits between stream 0's data and that record.
//!
//! Both EOF records carry a `stream_size`, but only stream 0's is dependable in
//! practice: Chromium writes `0` for stream 1 when the body was streamed in.
//! The body is therefore recovered by bounding it between the end of the key and
//! the start of stream 1's EOF record.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

const HEADER_MAGIC: u64 = 0xfcfb_6d1b_a772_5c30;
/// EOF magic used by current Chromium builds.
const EOF_MAGIC: u64 = 0xf4fa_6f45_970d_41d8;
/// EOF magic used by older builds; still seen in long-lived profiles.
const EOF_MAGIC_LEGACY: u64 = 0xf4fa_6f45_970d_41d5;
/// `SimpleFileHeader` is 20 bytes of fields padded to a u64 boundary.
const HEADER_LEN: usize = 24;
/// `SimpleFileEOF` is 20 bytes of fields padded to a u64 boundary.
const EOF_LEN: usize = 24;
const KEY_SHA256_LEN: usize = 32;
/// `SimpleFileEOF::FLAG_HAS_KEY_SHA256`.
const FLAG_HAS_KEY_SHA256: u32 = 2;
/// Cache keys are URLs with a small prefix; this bounds the cheap key read.
const MAX_KEY_LEN: usize = 64 * 1024;

#[derive(Debug)]
pub struct CacheEntry {
    pub key: String,
    /// Stream 0: the serialized HTTP response headers.
    pub headers: Vec<u8>,
    /// Stream 1: the response body, still in its transfer encoding.
    pub body: Vec<u8>,
}

/// Reads only the entry key, without pulling the body into memory.
///
/// Indexing a cache directory touches thousands of files, so this reads a single
/// small prefix per file.
pub fn read_entry_key(path: &Path) -> Option<String> {
    // Keys are URLs, so this prefix covers all but pathological ones; longer
    // keys fall through to the seek below. Keeping it small matters because
    // indexing reads a prefix from every entry in the profile.
    let mut file = File::open(path).ok()?;
    let mut prefix = [0u8; 1024];
    let read = file.read(&mut prefix).ok()?;
    let prefix = &prefix[..read];

    let key_len = parse_header(prefix)?;
    if key_len <= prefix.len() - HEADER_LEN {
        return decode_key(&prefix[HEADER_LEN..HEADER_LEN + key_len]);
    }

    let mut key_bytes = vec![0u8; key_len];
    file.seek(SeekFrom::Start(HEADER_LEN as u64)).ok()?;
    file.read_exact(&mut key_bytes).ok()?;
    decode_key(&key_bytes)
}

/// Reads only the response status, without decoding the body.
///
/// Stream 0 sits at the end of the file, so this reads a small tail rather than
/// pulling a multi-megabyte conversation into memory just to learn whether the
/// entry is a cached error.
pub fn read_entry_status(path: &Path) -> Option<u16> {
    const TAIL_BYTES: u64 = 128 * 1024;

    let mut file = File::open(path).ok()?;
    let len = file.metadata().ok()?.len();
    let tail_len = len.min(TAIL_BYTES);
    let tail_start = len - tail_len;

    file.seek(SeekFrom::Start(tail_start)).ok()?;
    let mut tail = vec![0u8; tail_len as usize];
    file.read_exact(&mut tail).ok()?;

    let stream0_eof = tail.len().checked_sub(EOF_LEN)?;
    let (flags, stream0_len) = parse_eof(&tail, stream0_eof)?;
    let key_sha256_len = if flags & FLAG_HAS_KEY_SHA256 != 0 {
        KEY_SHA256_LEN
    } else {
        0
    };
    let stream0_start = stream0_eof
        .checked_sub(key_sha256_len)?
        .checked_sub(stream0_len)?;

    status_code(&tail[stream0_start..stream0_eof - key_sha256_len])
}

/// Reads a full entry: key, response headers, and the raw response body.
pub fn read_entry(path: &Path) -> Option<CacheEntry> {
    let bytes = std::fs::read(path).ok()?;
    let key_len = parse_header(&bytes)?;
    let key_end = HEADER_LEN.checked_add(key_len)?;
    if bytes.len() < key_end + EOF_LEN {
        return None;
    }
    let key = decode_key(&bytes[HEADER_LEN..key_end])?;

    // Stream 0 is terminated by the final EOF record in the file, preceded by
    // the key's SHA-256 when that record says so.
    let stream0_eof = bytes.len().checked_sub(EOF_LEN)?;
    let (flags, stream0_len) = parse_eof(&bytes, stream0_eof)?;
    let key_sha256_len = if flags & FLAG_HAS_KEY_SHA256 != 0 {
        KEY_SHA256_LEN
    } else {
        0
    };
    let stream0_end = stream0_eof.checked_sub(key_sha256_len)?;
    let stream0_start = stream0_end.checked_sub(stream0_len)?;
    if stream0_start < key_end {
        return None;
    }
    let headers = bytes[stream0_start..stream0_end].to_vec();

    // Stream 1's EOF record sits immediately before stream 0's data.
    let body_end = stream0_start.checked_sub(EOF_LEN)?;
    if body_end < key_end || parse_eof(&bytes, body_end).is_none() {
        return None;
    }
    let body = bytes[key_end..body_end].to_vec();

    Some(CacheEntry { key, headers, body })
}

/// Returns the key length if `bytes` starts with a valid `SimpleFileHeader`.
fn parse_header(bytes: &[u8]) -> Option<usize> {
    if bytes.len() < HEADER_LEN || read_u64(bytes, 0)? != HEADER_MAGIC {
        return None;
    }

    let key_len = read_u32(bytes, 12)? as usize;
    (key_len > 0 && key_len <= MAX_KEY_LEN).then_some(key_len)
}

/// Returns `(flags, stream_size)` if a valid `SimpleFileEOF` starts at `offset`.
fn parse_eof(bytes: &[u8], offset: usize) -> Option<(u32, usize)> {
    let magic = read_u64(bytes, offset)?;
    if magic != EOF_MAGIC && magic != EOF_MAGIC_LEGACY {
        return None;
    }

    let flags = read_u32(bytes, offset + 8)?;
    let stream_size = read_i32(bytes, offset + 16)?;
    if stream_size < 0 || stream_size as usize > offset {
        return None;
    }

    Some((flags, stream_size as usize))
}

fn decode_key(bytes: &[u8]) -> Option<String> {
    std::str::from_utf8(bytes).ok().map(str::to_string)
}

fn read_u64(bytes: &[u8], offset: usize) -> Option<u64> {
    bytes
        .get(offset..offset + 8)?
        .try_into()
        .ok()
        .map(u64::from_le_bytes)
}

fn read_u32(bytes: &[u8], offset: usize) -> Option<u32> {
    bytes
        .get(offset..offset + 4)?
        .try_into()
        .ok()
        .map(u32::from_le_bytes)
}

fn read_i32(bytes: &[u8], offset: usize) -> Option<i32> {
    bytes
        .get(offset..offset + 4)?
        .try_into()
        .ok()
        .map(i32::from_le_bytes)
}

/// Extracts the HTTP status code from the serialized stream 0 blob.
///
/// Chromium caches error responses too — a conversation deleted from the account
/// leaves a cached 404 behind — so the status has to be checked before treating
/// a body as a transcript.
pub fn status_code(headers: &[u8]) -> Option<u16> {
    let text = String::from_utf8_lossy(headers);
    text.split('\0')
        .find(|field| field.starts_with("HTTP/"))?
        .split_whitespace()
        .nth(1)?
        .parse()
        .ok()
}

/// Extracts a header value from the serialized stream 0 blob.
///
/// Stream 0 is a Chromium pickle whose payload holds the status line and the
/// headers as NUL-separated `name:value` pairs. Rather than decoding the pickle
/// framing, this scans for the header directly, which is stable across versions.
pub fn header_value(headers: &[u8], name: &str) -> Option<String> {
    let text = String::from_utf8_lossy(headers);
    let wanted = name.to_ascii_lowercase();

    text.split('\0').find_map(|field| {
        let (key, value) = field.split_once(':')?;
        (key.trim().to_ascii_lowercase() == wanted).then(|| value.trim().to_string())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds a cache entry file in the on-disk layout.
    ///
    /// The key SHA-256 goes after stream 0's data and before its EOF record —
    /// verified against real entries by hashing the key and comparing those 32
    /// bytes. Getting this backwards shifts the header slice by 32 bytes.
    fn build_entry(key: &str, body: &[u8], headers: &[u8], with_key_sha256: bool) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&HEADER_MAGIC.to_le_bytes());
        bytes.extend_from_slice(&6u32.to_le_bytes()); // version
        bytes.extend_from_slice(&(key.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes()); // key hash
        bytes.extend_from_slice(&0u32.to_le_bytes()); // padding
        bytes.extend_from_slice(key.as_bytes());
        bytes.extend_from_slice(body);
        // Chromium writes 0 here when the body was streamed in.
        bytes.extend_from_slice(&eof_record(0, 1));
        bytes.extend_from_slice(headers);
        let stream0_flags = if with_key_sha256 {
            if let Some(digest) = fake_key_sha256(key) {
                bytes.extend_from_slice(&digest);
            }
            1 | FLAG_HAS_KEY_SHA256
        } else {
            1
        };
        bytes.extend_from_slice(&eof_record(headers.len() as i32, stream0_flags));
        bytes
    }

    /// Stand-in for the real digest; only its length matters to the parser.
    fn fake_key_sha256(key: &str) -> Option<[u8; KEY_SHA256_LEN]> {
        let mut digest = [0u8; KEY_SHA256_LEN];
        for (index, slot) in digest.iter_mut().enumerate() {
            *slot = key.as_bytes()[index % key.len()];
        }
        Some(digest)
    }

    fn eof_record(stream_size: i32, flags: u32) -> [u8; EOF_LEN] {
        let mut record = [0u8; EOF_LEN];
        record[0..8].copy_from_slice(&EOF_MAGIC.to_le_bytes());
        record[8..12].copy_from_slice(&flags.to_le_bytes());
        record[12..16].copy_from_slice(&0u32.to_le_bytes()); // crc32
        record[16..20].copy_from_slice(&stream_size.to_le_bytes());
        record
    }

    fn headers_blob() -> Vec<u8> {
        let mut blob = vec![0u8; 12]; // pickle framing we intentionally skip
        blob.extend_from_slice(b"HTTP/1.1 200\0content-type:application/json\0content-encoding:zstd\0");
        blob
    }

    fn write_temp(name: &str, bytes: &[u8]) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(name);
        std::fs::write(&path, bytes).unwrap();
        path
    }

    #[test]
    fn reads_key_headers_and_body() {
        let key = "1/0/https://claude.ai/api/organizations/org/chat_conversations/uuid";
        let bytes = build_entry(key, b"compressed-body-bytes", &headers_blob(), false);
        let path = write_temp("cse-simple-cache-plain_0", &bytes);

        let entry = read_entry(&path).unwrap();
        assert_eq!(entry.key, key);
        assert_eq!(entry.body, b"compressed-body-bytes");
        // Exact, not "contains": a slice shifted by the SHA-256 length would
        // still satisfy a scan for a header value.
        assert_eq!(entry.headers, headers_blob());
        assert_eq!(
            header_value(&entry.headers, "content-encoding").as_deref(),
            Some("zstd")
        );
        assert_eq!(read_entry_key(&path).as_deref(), Some(key));
    }

    /// Most real entries carry a key SHA-256 between stream 0 and its EOF
    /// record. Both slices must come back byte-exact regardless.
    #[test]
    fn reads_streams_when_key_sha256_is_present() {
        let key = "1/0/https://claude.ai/api/x";
        let bytes = build_entry(key, b"body", &headers_blob(), true);
        let path = write_temp("cse-simple-cache-sha_0", &bytes);

        let entry = read_entry(&path).unwrap();
        assert_eq!(entry.body, b"body");
        assert_eq!(entry.headers, headers_blob());
        assert_eq!(status_code(&entry.headers), Some(200));
    }

    #[test]
    fn rejects_files_that_are_not_cache_entries() {
        let path = write_temp("cse-simple-cache-bogus_0", b"not a cache entry at all");
        assert!(read_entry(&path).is_none());
        assert!(read_entry_key(&path).is_none());
    }

    #[test]
    fn header_lookup_is_case_insensitive_and_misses_cleanly() {
        let headers = headers_blob();
        assert_eq!(
            header_value(&headers, "Content-Type").as_deref(),
            Some("application/json")
        );
        assert!(header_value(&headers, "content-length").is_none());
    }

    #[test]
    fn reads_status_code_including_cached_errors() {
        assert_eq!(status_code(&headers_blob()), Some(200));

        let mut not_found = vec![0u8; 12];
        not_found.extend_from_slice(b"HTTP/1.1 404\0content-type:application/json\0");
        assert_eq!(status_code(&not_found), Some(404));

        assert_eq!(status_code(b"no status line here"), None);
    }
}
