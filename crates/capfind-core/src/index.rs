//! On-disk index file I/O: `<repo>/.capfind/index.cfi`.
//!
//! # Format
//!
//! ```text
//! ┌─ MAGIC   8B  b"CAPFIND\0"
//! ├─ VERSION 2B  u16, little-endian
//! ├─ CREATED 8B  i64 unix seconds (may be negative; used for `stats` only)
//! ├─ REPO_FP 32B blake3 of sorted roots + git HEAD (optional — all-zero when unknown)
//! ├─ FLAGS   2B  u16; bit 0 = body is zstd-compressed
//! ├─ BODYLEN 4B  u32, little-endian — length of the payload that follows
//! └─ BODY        bincode(IndexBody), optionally wrapped in zstd
//! ```
//!
//! A fixed 56-byte header keeps `stats` cheap: we can read the first chunk and
//! get version/created/fingerprint without decompressing the payload.
//!
//! [`write_index`] always compresses; [`load_index`] understands both.

use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::{IndexError, Result};
use crate::model::{Capability, FileStat};

/// Magic prefix. Not `0xdeadbeef`-style — a human-greppable string is friendlier
/// when someone `xxd`s the file.
pub const MAGIC: &[u8; 8] = b"CAPFIND\0";

/// Bump when the on-disk schema breaks. `load_index` rejects older versions
/// with a clear error rather than silently producing garbage.
pub const INDEX_VERSION: u16 = 1;

/// The size of the fixed header section, in bytes. Computed at compile time
/// from the field layout above.
pub const HEADER_LEN: usize = 8 /*magic*/ + 2 /*ver*/ + 8 /*created*/ + 32 /*fp*/ + 2 /*flags*/ + 4 /*bodylen*/;

const FLAG_ZSTD: u16 = 1 << 0;

/// Zstd compression level. Level 3 balances speed and ratio; we're writing a
/// 5-MB body in under 100 ms on commodity hardware.
const ZSTD_LEVEL: i32 = 3;

/// One posting: (capability id, which scoring field, term frequency).
///
/// A flat `Vec<Posting>` keyed by `term_id` is the fastest shape to iterate
/// during BM25 scoring — we pay one `HashMap::get` per query token and then
/// stream a tight array.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Posting {
    pub cap_id: u32,
    pub field: crate::model::Field,
    pub tf: u16,
}

/// The entire payload of an index file.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct IndexBody {
    pub capabilities: Vec<Capability>,
    /// String pool: `term_id` indexes into this.
    pub vocab: Vec<String>,
    /// `term_id -> postings`. `HashMap` because term distribution is sparse
    /// relative to total vocab size.
    pub postings: HashMap<u32, Vec<Posting>>,
    /// `file path -> stat` for incremental rebuilds.
    pub file_stats: HashMap<String, FileStat>,
    /// BM25's average document length across all capabilities (weighted by
    /// the field-weight table).
    pub avgdl: f32,
}

/// The header fields we expose to callers. Kept together so `capfind stats`
/// can print it without touching the payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IndexHeader {
    pub version: u16,
    pub created_unix: i64,
    pub repo_fp: [u8; 32],
    pub flags: u16,
    pub body_len: u32,
}

/// Write the complete index to `path` atomically via a `.tmp` sibling rename.
///
/// Atomic write matters because `capfind index` can be Ctrl-C'd mid-build;
/// we don't want to leave a half-written `.cfi` that fails to load next time.
pub fn write_index(
    path: &Path,
    body: &IndexBody,
    created_unix: i64,
    repo_fp: [u8; 32],
) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("cfi.tmp");

    // 1. bincode body
    let raw = bincode::serialize(body)?;

    // 2. zstd wrap
    let compressed = zstd::bulk::compress(&raw, ZSTD_LEVEL)
        .map_err(|e| IndexError::Decompress(format!("compress failed: {e}")))?;
    let flags = FLAG_ZSTD;
    let body_len = u32::try_from(compressed.len()).map_err(|_| {
        IndexError::Decompress(format!(
            "compressed body too large for u32 body_len: {} bytes",
            compressed.len()
        ))
    })?;

    // 3. write header + body
    let mut f = File::create(&tmp)?;
    f.write_all(MAGIC)?;
    f.write_all(&INDEX_VERSION.to_le_bytes())?;
    f.write_all(&created_unix.to_le_bytes())?;
    f.write_all(&repo_fp)?;
    f.write_all(&flags.to_le_bytes())?;
    f.write_all(&body_len.to_le_bytes())?;
    f.write_all(&compressed)?;
    f.sync_all()?;
    drop(f);

    // 4. atomic rename
    fs::rename(&tmp, path)?;
    Ok(())
}

/// Read just the fixed header. Cheap — exposes version/created/fingerprint
/// without decompressing the body.
pub fn read_header(path: &Path) -> Result<IndexHeader> {
    let mut f = File::open(path)?;
    let mut buf = [0u8; HEADER_LEN];
    read_exact_or_truncated(&mut f, &mut buf, 0)?;
    parse_header(&buf)
}

/// Load the entire index (header + body).
pub fn load_index(path: &Path) -> Result<(IndexHeader, IndexBody)> {
    let mut f = File::open(path)?;
    let mut hdr_buf = [0u8; HEADER_LEN];
    read_exact_or_truncated(&mut f, &mut hdr_buf, 0)?;
    let header = parse_header(&hdr_buf)?;

    let mut body_buf = vec![0u8; header.body_len as usize];
    read_exact_or_truncated(&mut f, &mut body_buf, HEADER_LEN)?;

    let raw = if header.flags & FLAG_ZSTD != 0 {
        // Decompress. We don't know the uncompressed size up-front; zstd crate
        // handles this for us via the streaming decoder.
        zstd::bulk::decompress(&body_buf, 256 * 1024 * 1024 /* 256 MB ceiling */)
            .map_err(|e| IndexError::Decompress(format!("decompress failed: {e}")))?
    } else {
        body_buf
    };

    let body: IndexBody = bincode::deserialize(&raw)?;
    Ok((header, body))
}

fn parse_header(buf: &[u8; HEADER_LEN]) -> Result<IndexHeader> {
    let mut magic = [0u8; 8];
    magic.copy_from_slice(&buf[0..8]);
    if &magic != MAGIC {
        return Err(IndexError::BadMagic { got: magic });
    }
    let version = u16::from_le_bytes(buf[8..10].try_into().unwrap());
    if version != INDEX_VERSION {
        return Err(IndexError::UnsupportedVersion {
            found: version,
            supported: INDEX_VERSION,
        });
    }
    let created_unix = i64::from_le_bytes(buf[10..18].try_into().unwrap());
    let mut repo_fp = [0u8; 32];
    repo_fp.copy_from_slice(&buf[18..50]);
    let flags = u16::from_le_bytes(buf[50..52].try_into().unwrap());
    let body_len = u32::from_le_bytes(buf[52..56].try_into().unwrap());
    Ok(IndexHeader {
        version,
        created_unix,
        repo_fp,
        flags,
        body_len,
    })
}

fn read_exact_or_truncated<R: Read>(r: &mut R, buf: &mut [u8], offset: usize) -> Result<()> {
    let needed = buf.len();
    let mut got = 0;
    while got < needed {
        match r.read(&mut buf[got..])? {
            0 => {
                return Err(IndexError::Truncated {
                    offset: offset + got,
                    needed,
                    got,
                });
            }
            n => got += n,
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Field, HttpInfo, Kind, Lang, TermRef};
    use pretty_assertions::assert_eq;
    use tempfile::tempdir;

    fn sample_body() -> IndexBody {
        let cap = Capability {
            id: 0,
            kind: Kind::HttpEndpoint,
            lang: Lang::Java,
            module: "biz".into(),
            package: "com.demo".into(),
            class: Some("MdmController".into()),
            method: "queryMdm".into(),
            signature: "ApiResult<X> queryMdm(MdmQueryRequest)".into(),
            annotations: vec!["@PostMapping(\"/query\")".into()],
            http: Some(HttpInfo {
                method: "POST".into(),
                path: "/mdm/query".into(),
                consumes: None,
                produces: None,
            }),
            rpc: None,
            doc: Some("Query MDM by id.".into()),
            tags: vec![],
            file: "biz/src/main/java/com/demo/MdmController.java".into(),
            line: 42,
            byte_range: (100, 200),
            terms: vec![TermRef {
                term_id: 0,
                field: Field::MethodName,
                tf: 1,
            }],
        };
        let mut postings = HashMap::new();
        postings.insert(
            0,
            vec![Posting {
                cap_id: 0,
                field: Field::MethodName,
                tf: 1,
            }],
        );
        IndexBody {
            capabilities: vec![cap],
            vocab: vec!["mdm".into()],
            postings,
            file_stats: HashMap::new(),
            avgdl: 1.0,
        }
    }

    #[test]
    fn round_trip_is_stable() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("index.cfi");
        let body = sample_body();
        let fp = [7u8; 32];
        write_index(&path, &body, 1_700_000_000, fp).unwrap();

        let (hdr, loaded) = load_index(&path).unwrap();
        assert_eq!(hdr.version, INDEX_VERSION);
        assert_eq!(hdr.created_unix, 1_700_000_000);
        assert_eq!(hdr.repo_fp, fp);
        assert_eq!(hdr.flags & FLAG_ZSTD, FLAG_ZSTD);
        assert_eq!(loaded, body);
    }

    #[test]
    fn header_read_is_cheap_and_correct() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("index.cfi");
        write_index(&path, &sample_body(), 42, [1u8; 32]).unwrap();

        let hdr = read_header(&path).unwrap();
        assert_eq!(hdr.version, INDEX_VERSION);
        assert_eq!(hdr.created_unix, 42);
        assert_eq!(hdr.repo_fp, [1u8; 32]);
    }

    #[test]
    fn bad_magic_is_rejected() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("wrong.cfi");
        std::fs::write(&path, vec![0u8; HEADER_LEN + 4]).unwrap();

        let err = load_index(&path).unwrap_err();
        assert!(matches!(err, IndexError::BadMagic { .. }), "got: {err:?}");
    }

    #[test]
    fn truncated_file_errors_cleanly() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("trunc.cfi");
        // Only the magic; everything else missing.
        std::fs::write(&path, MAGIC).unwrap();

        let err = load_index(&path).unwrap_err();
        assert!(matches!(err, IndexError::Truncated { .. }), "got: {err:?}");
    }
}
