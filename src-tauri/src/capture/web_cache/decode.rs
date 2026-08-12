//! Decodes a cached response body out of its HTTP transfer encoding.
//!
//! Every decoder here is pure Rust, so building the app needs no C toolchain
//! beyond what Tauri itself requires.

use std::io::Read;

/// Guards against a corrupt length field turning into an unbounded allocation.
const MAX_DECODED_BYTES: u64 = 512 * 1024 * 1024;

pub fn decode_body(body: &[u8], content_encoding: Option<&str>) -> Result<Vec<u8>, String> {
    match content_encoding
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "" | "identity" => Ok(body.to_vec()),
        "zstd" => {
            let decoder =
                ruzstd::StreamingDecoder::new(body).map_err(|error| format!("zstd: {error}"))?;
            read_all(decoder, "zstd")
        }
        "gzip" | "x-gzip" => read_all(flate2::read::MultiGzDecoder::new(body), "gzip"),
        "deflate" => read_all(flate2::read::ZlibDecoder::new(body), "deflate"),
        "br" => read_all(brotli::Decompressor::new(body, 8192), "brotli"),
        other => Err(format!("unsupported content-encoding `{other}`")),
    }
}

fn read_all(reader: impl Read, label: &str) -> Result<Vec<u8>, String> {
    let mut decoded = Vec::new();
    reader
        .take(MAX_DECODED_BYTES)
        .read_to_end(&mut decoded)
        .map_err(|error| format!("{label}: {error}"))?;
    Ok(decoded)
}

#[cfg(test)]
mod tests {
    use super::decode_body;
    use std::io::Write;

    /// `{"uuid":"x"}` as a zstd frame, the encoding claude.ai actually serves.
    const ZSTD_FRAME: &[u8] = &[
        0x28, 0xb5, 0x2f, 0xfd, 0x20, 0x0c, 0x61, 0x00, 0x00, 0x7b, 0x22, 0x75, 0x75, 0x69, 0x64,
        0x22, 0x3a, 0x22, 0x78, 0x22, 0x7d,
    ];

    #[test]
    fn passes_through_identity_bodies() {
        assert_eq!(decode_body(b"{}", None).unwrap(), b"{}");
        assert_eq!(decode_body(b"{}", Some("identity")).unwrap(), b"{}");
    }

    #[test]
    fn decodes_zstd_bodies() {
        assert_eq!(
            decode_body(ZSTD_FRAME, Some("zstd")).unwrap(),
            br#"{"uuid":"x"}"#
        );
    }

    #[test]
    fn decodes_gzip_bodies_case_insensitively() {
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(b"hello").unwrap();
        let encoded = encoder.finish().unwrap();
        assert_eq!(decode_body(&encoded, Some("GZIP")).unwrap(), b"hello");
    }

    #[test]
    fn reports_unsupported_encodings() {
        let error = decode_body(b"x", Some("snappy")).unwrap_err();
        assert!(error.contains("snappy"), "{error}");
    }

    #[test]
    fn reports_corrupt_bodies_rather_than_panicking() {
        assert!(decode_body(b"not a zstd frame", Some("zstd")).is_err());
        assert!(decode_body(b"not gzip", Some("gzip")).is_err());
    }
}
