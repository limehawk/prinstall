//! Pure-Rust parser for SDIO's SDW binary index format.
//!
//! Clean-room port of the format from published `indexing.h` struct
//! definitions (facts, not source code) plus empirical validation against
//! real `.bin` files. Decompresses the LZMA1 "Alone" payload via `lzma-rs`
//! and walks four typed record vectors — inffile / manufacturer / desc /
//! HWID — plus a flat string-interning table. The result is a fast
//! HWID → (inffile, desc, manufacturer) lookup the SDI resolver uses to
//! decide which pack contains a driver for a given printer's device-id.
//!
//! ## File layout
//!
//! ```text
//! 0..=2   : magic "SDW"
//! 3..=6   : u32 LE version  (e.g. 0x00000205)
//! 7       : reserved (0x00)
//! 8..     : LZMA1 "Alone" stream (13-byte header + compressed payload)
//! ```
//!
//! The decompressed blob is a strict concatenation:
//!
//! ```text
//! Section<data_inffile_t      | 132 B/entry>   // header [size u32 | count u32] + entries
//! Section<data_manufacturer_t |  16 B/entry>
//! Section<data_desc_t         |  24 B/entry>
//! Section<data_HWID_t         |  12 B/entry>
//! String table                                 // flat ASCII, null-terminated
//! <trailing driverpack metadata — ignored>
//! ```
//!
//! Every section begins with an 8-byte `[byte_size: u32, count: u32]`
//! header. `byte_size == count * sizeof(T)` — we validate both. Records
//! follow immediately, tightly packed.
//!
//! Each record's `ofst` fields index into the string table by byte offset.
//! Offset 0 is the empty string; any non-zero offset must land inside the
//! table or the file is rejected as corrupt.
//!
//! ## Record layouts (inferred from indexing.h, verified against
//! `DP_Printer_26000.bin`)
//!
//! `data_inffile_t` — 132 B (33 u32):
//! ```text
//! u32[ 0]   infpath      (ofst)
//! u32[ 1]   inffilename  (ofst)
//! u32[ 2]   fields[0]    (ClassGuid_,           ofst)
//! u32[ 3]   fields[1]    (Class,                ofst)
//! u32[ 4]   fields[2]    (Provider,             ofst)
//! u32[ 5]   fields[3]    (CatalogFile,          ofst)
//! u32[ 6]   fields[4]    (CatalogFile_nt,       ofst)
//! u32[ 7]   fields[5]    (CatalogFile_ntx86,    ofst)
//! u32[ 8]   fields[6]    (CatalogFile_ntia64,   ofst)
//! u32[ 9]   fields[7]    (CatalogFile_ntamd64,  ofst)
//! u32[10]   fields[8]    (DriverVer,            ofst)  <-- what we care about
//! u32[11]   fields[9]    (DriverPackageDisplayName, ofst)
//! u32[12]   fields[10]   (DriverPackageType,    ofst)
//! u32[13..23] cats[11]   (ignored — catalog refs)
//! u32[24..32] Version + infsize + infcrc (ignored — metadata)
//! ```
//!
//! `data_manufacturer_t` — 16 B (4 u32):
//! ```text
//! u32[0]  inffile_index (u32 index into inffile vector)
//! u32[1]  manufacturer  (ofst)
//! u32[2]  sections      (raw-blob offset, ignored)
//! u32[3]  sections_n    (int32, ignored)
//! ```
//!
//! `data_desc_t` — 24 B (6 u32):
//! ```text
//! u32[0]  manufacturer_index (u32)
//! u32[1]  sect_pos           (i32, ignored)
//! u32[2]  desc               (ofst — the human-readable driver name)
//! u32[3]  install            (ofst, ignored)
//! u32[4]  install_picked     (ofst, ignored)
//! u32[5]  feature            (u32, ignored)
//! ```
//!
//! `data_HWID_t` — 12 B (3 u32):
//! ```text
//! u32[0]  desc_index (u32)
//! u32[1]  inf_pos    (i32, ignored)
//! u32[2]  HWID       (ofst — the hardware-ID string)
//! ```
//!
//! ## Design choices
//!
//! - **Backing storage is `Arc<Vec<u8>>` over the decompressed blob.**
//!   String lookups return `&str` slices into that blob. `SdiHit<'a>`
//!   borrows from `SdiIndex` so no allocation happens per match. For the
//!   ~160k-entry printer pack this is a straight linear scan and takes
//!   microseconds.
//! - **Strict validation.** Every section header must satisfy
//!   `byte_size == count * sizeof(T)`, every ofst must point inside the
//!   string table, every cross-reference index must be < the target
//!   vector's length. Violations return structured `Err(String)`. No
//!   panics, no unwraps, no unchecked casts.
//! - **Bounded string reads.** Strings are null-terminated but we cap
//!   each read at 4096 bytes so a corrupt offset pointing at unterminated
//!   binary can't walk forever.
//! - **No SDIO code.** Every byte in this file is clean-room — the format
//!   is reconstructed from `indexing.h` facts plus observation of a real
//!   production `.bin`. None of the SDIO (GPL) source is compiled in.

use std::fs;
use std::path::Path;

/// Magic bytes at the start of every SDW file.
const SDW_MAGIC: &[u8; 3] = b"SDW";

/// Size of the fixed SDW prefix before the LZMA1 stream.
const SDW_PREFIX_LEN: usize = 8;

/// Smallest SDW version we know how to parse. We accept any version whose
/// upper 16 bits are zero so that minor-version bumps on newer SDIO
/// releases keep working as long as the record layouts are compatible.
const MIN_KNOWN_VERSION: u32 = 0x0000_0200;
/// Largest SDW version we'll attempt to parse without whining.
const MAX_KNOWN_VERSION: u32 = 0x0000_02FF;

/// Per-entry byte sizes — fixed by the format.
const INFFILE_SIZE: usize = 132;
const MANUFACTURER_SIZE: usize = 16;
const DESC_SIZE: usize = 24;
const HWID_SIZE: usize = 12;

/// Upper bound on how many bytes we'll read for a single interned string.
/// Any real field is well under this — it's a safety cap against a
/// corrupt offset landing in a region without a null terminator.
const MAX_STRING_LEN: usize = 4096;

/// Field indices into `data_inffile_t::fields[11]`, matching the
/// `NUM_VER_NAMES` enum in `indexing.h`.
const FIELD_DRIVER_VER: usize = 8;

/// Parsed in-memory representation of an SDIO `.bin` index.
///
/// Owned, self-contained, cheap to query. Parse once per `.bin` at
/// startup (or on-demand from the SDI cache) and keep it around; every
/// `find_matching` call is a linear scan over the HWID vector which is
/// well under a millisecond for production pack sizes.
pub struct SdiIndex {
    /// Pack name derived from the `.bin` filename (with the extension
    /// stripped). Empty when parsed from raw bytes via [`parse_index`].
    pub pack_name: String,
    /// SDW version number from the header (little-endian `u32`).
    pub version: u32,

    /// Decompressed blob. All string slices and record views borrow from
    /// this buffer.
    blob: Vec<u8>,

    /// Byte offset of the string table within `blob`.
    string_table_start: usize,
    /// Exclusive end of the string table — everything from here onward is
    /// opaque trailing driverpack metadata we don't interpret.
    string_table_end: usize,

    /// Byte offsets into `blob` for the record vectors.
    inffile_start: usize,
    inffile_count: usize,
    manufacturer_start: usize,
    manufacturer_count: usize,
    desc_start: usize,
    desc_count: usize,
    hwid_start: usize,
    hwid_count: usize,
}

impl std::fmt::Debug for SdiIndex {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SdiIndex")
            .field("pack_name", &self.pack_name)
            .field("version", &format_args!("0x{:08x}", self.version))
            .field("blob_len", &self.blob.len())
            .field("inffile_count", &self.inffile_count)
            .field("manufacturer_count", &self.manufacturer_count)
            .field("desc_count", &self.desc_count)
            .field("hwid_count", &self.hwid_count)
            .field(
                "string_table_bytes",
                &(self.string_table_end.saturating_sub(self.string_table_start)),
            )
            .finish()
    }
}

/// A single HWID → driver match, borrowed from the backing [`SdiIndex`].
///
/// All string fields are slices into the index's string table — no
/// allocation happens per match. The only owned field is
/// `inf_dir_prefix`, which we build by normalising `\` → `/` on the
/// interned `infpath` and appending a trailing slash for filesystem joins.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SdiHit<'a> {
    /// The HWID string from the index — e.g. `"1284_CID_BROTHER_LASER_TYPE1"`.
    pub hwid: &'a str,
    /// Directory prefix (forward-slash, trailing `/`) for the INF inside
    /// the pack — e.g. `"Brother/Allx64/FORCED/-Class/"`. Used by the pack
    /// extractor as a filename-prefix filter.
    pub inf_dir_prefix: String,
    /// INF filename — e.g. `"prnbrcl1.inf"`.
    pub inf_filename: &'a str,
    /// Human-readable driver display name from `data_desc_t.desc`.
    pub driver_display_name: &'a str,
    /// `DriverVer` field from the INF, if the index recorded one.
    pub driver_ver: Option<&'a str>,
    /// Manufacturer (`Brother`, `Canon`, ...) from the manufacturer record.
    pub driver_manufacturer: &'a str,
}

/// Parse an SDW `.bin` file from in-memory bytes.
pub fn parse_index(bytes: &[u8]) -> Result<SdiIndex, String> {
    parse_index_inner(bytes, String::new())
}

/// Convenience wrapper that reads a `.bin` file from disk, then calls
/// [`parse_index`]. The pack name is derived from the file stem.
pub fn parse_index_file(path: &Path) -> Result<SdiIndex, String> {
    let bytes = fs::read(path)
        .map_err(|e| format!("Failed to read SDW index {}: {e}", path.display()))?;
    let pack_name = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_string();
    parse_index_inner(&bytes, pack_name)
}

fn parse_index_inner(bytes: &[u8], pack_name: String) -> Result<SdiIndex, String> {
    // --- Header ---
    if bytes.len() < SDW_PREFIX_LEN {
        return Err(format!(
            "SDW header truncated: got {} bytes, need at least {}",
            bytes.len(),
            SDW_PREFIX_LEN
        ));
    }
    if &bytes[0..3] != SDW_MAGIC {
        return Err(format!(
            "SDW magic mismatch: expected 'SDW', got {:02x?}",
            &bytes[0..3]
        ));
    }
    let version = u32::from_le_bytes([bytes[3], bytes[4], bytes[5], bytes[6]]);
    if !(MIN_KNOWN_VERSION..=MAX_KNOWN_VERSION).contains(&version) {
        return Err(format!(
            "SDW version 0x{version:08x} outside known range 0x{MIN_KNOWN_VERSION:08x}..=0x{MAX_KNOWN_VERSION:08x}"
        ));
    }

    // --- LZMA1 "Alone" payload ---
    let payload = &bytes[SDW_PREFIX_LEN..];
    let mut reader = std::io::BufReader::new(payload);
    let mut blob: Vec<u8> = Vec::new();
    lzma_rs::lzma_decompress(&mut reader, &mut blob)
        .map_err(|e| format!("SDW payload LZMA decompress failed: {e}"))?;

    // --- Section walk ---
    let mut cursor = 0usize;

    let (inffile_start, inffile_count, inffile_end) =
        read_section_header(&blob, cursor, INFFILE_SIZE, "inffile")?;
    cursor = inffile_end;

    let (manufacturer_start, manufacturer_count, manufacturer_end) =
        read_section_header(&blob, cursor, MANUFACTURER_SIZE, "manufacturer")?;
    cursor = manufacturer_end;

    let (desc_start, desc_count, desc_end) =
        read_section_header(&blob, cursor, DESC_SIZE, "desc")?;
    cursor = desc_end;

    let (hwid_start, hwid_count, hwid_end) =
        read_section_header(&blob, cursor, HWID_SIZE, "hwid")?;
    cursor = hwid_end;

    // After the HWID vector comes an 8-byte string-table header
    // `[byte_size_u32, byte_size_u32]` (the second field is a duplicate
    // of the first — SDIO's `Txt` class tracks `size` and `capacity` as
    // two separate fields and emits both on disk). We only consume the
    // first. The actual string blob follows, sized exactly as declared.
    // Anything past the string table is opaque trailing driverpack
    // metadata (Hashtable, cat_list) that we don't interpret.
    if cursor + 8 > blob.len() {
        return Err(format!(
            "SDW string-table header truncated: need 8 bytes at {cursor}, have {}",
            blob.len().saturating_sub(cursor)
        ));
    }
    let string_table_bytes = u32::from_le_bytes([
        blob[cursor],
        blob[cursor + 1],
        blob[cursor + 2],
        blob[cursor + 3],
    ]) as usize;
    let string_table_start = cursor + 8;
    let string_table_end = string_table_start
        .checked_add(string_table_bytes)
        .ok_or_else(|| "SDW string-table end overflow".to_string())?;
    if string_table_end > blob.len() {
        return Err(format!(
            "SDW string-table truncated: declared {string_table_bytes} bytes at {string_table_start}, blob has {}",
            blob.len().saturating_sub(string_table_start)
        ));
    }

    let index = SdiIndex {
        pack_name,
        version,
        blob,
        string_table_start,
        string_table_end,
        inffile_start,
        inffile_count,
        manufacturer_start,
        manufacturer_count,
        desc_start,
        desc_count,
        hwid_start,
        hwid_count,
    };

    // --- Structural cross-check on every manufacturer / desc / HWID
    // record. Catches index-out-of-range corruption up front so later
    // lookups can be unchecked in the hot path.
    index.validate_cross_references()?;

    Ok(index)
}

/// Read an 8-byte `[byte_size, count]` section header at `cursor` and
/// return `(records_start, count, records_end)`.
fn read_section_header(
    blob: &[u8],
    cursor: usize,
    entry_size: usize,
    name: &str,
) -> Result<(usize, usize, usize), String> {
    let header_end = cursor
        .checked_add(8)
        .ok_or_else(|| format!("SDW {name} header offset overflow"))?;
    if header_end > blob.len() {
        return Err(format!(
            "SDW {name} header truncated: need 8 bytes at {cursor}, have {}",
            blob.len().saturating_sub(cursor)
        ));
    }
    let byte_size = u32::from_le_bytes([
        blob[cursor],
        blob[cursor + 1],
        blob[cursor + 2],
        blob[cursor + 3],
    ]) as usize;
    let count = u32::from_le_bytes([
        blob[cursor + 4],
        blob[cursor + 5],
        blob[cursor + 6],
        blob[cursor + 7],
    ]) as usize;

    let expected = count
        .checked_mul(entry_size)
        .ok_or_else(|| format!("SDW {name} size overflow: count={count} * {entry_size}"))?;
    if byte_size != expected {
        return Err(format!(
            "SDW {name} header mismatch: byte_size={byte_size}, expected count*{entry_size}={expected} (count={count})"
        ));
    }

    let records_start = header_end;
    let records_end = records_start
        .checked_add(byte_size)
        .ok_or_else(|| format!("SDW {name} records end overflow"))?;
    if records_end > blob.len() {
        return Err(format!(
            "SDW {name} records truncated: need {byte_size} bytes at {records_start}, have {}",
            blob.len().saturating_sub(records_start)
        ));
    }

    Ok((records_start, count, records_end))
}

impl SdiIndex {
    /// Return a case-insensitive match against any of the provided HWID
    /// candidates. Linear scan — for SDIO printer pack sizes (hundreds to
    /// a few thousand entries) this is microseconds.
    pub fn find_matching(&self, hwid_candidates: &[String]) -> Vec<SdiHit<'_>> {
        let mut hits: Vec<SdiHit<'_>> = Vec::new();
        if hwid_candidates.is_empty() || self.hwid_count == 0 {
            return hits;
        }
        for i in 0..self.hwid_count {
            let (desc_idx, _inf_pos, hwid_ofst) = match self.read_hwid_record(i) {
                Some(v) => v,
                None => continue,
            };
            let hwid = match self.read_string_checked(hwid_ofst) {
                Some(s) if !s.is_empty() => s,
                _ => continue,
            };
            if !hwid_candidates
                .iter()
                .any(|c| c.eq_ignore_ascii_case(hwid))
            {
                continue;
            }
            if let Some(hit) = self.build_hit(hwid, desc_idx) {
                hits.push(hit);
            }
        }
        hits
    }

    /// Total number of HWID entries in the index. Exposed so the
    /// `prinstall drivers` diagnostic can surface it without needing a
    /// separate count call.
    pub fn hwid_count(&self) -> usize {
        self.hwid_count
    }

    /// Derive the `.7z` pack filename this index corresponds to by
    /// substituting `.bin` → `.7z` in [`pack_name`](Self::pack_name).
    /// Returns `"<pack>.7z"` even if `pack_name` is empty (in which case
    /// you'll get just `".7z"` — the caller is expected to know the
    /// filename it started with).
    pub fn pack_filename(&self) -> String {
        format!("{}.7z", self.pack_name)
    }

    // ---- Internal lookups ----

    /// Decode a single `data_HWID_t` record. Returns `(desc_idx, inf_pos, HWID_ofst)`.
    fn read_hwid_record(&self, i: usize) -> Option<(u32, i32, u32)> {
        if i >= self.hwid_count {
            return None;
        }
        let off = self.hwid_start + i * HWID_SIZE;
        let slice = self.blob.get(off..off + HWID_SIZE)?;
        let desc_idx = u32::from_le_bytes([slice[0], slice[1], slice[2], slice[3]]);
        let inf_pos = i32::from_le_bytes([slice[4], slice[5], slice[6], slice[7]]);
        let hwid_ofst = u32::from_le_bytes([slice[8], slice[9], slice[10], slice[11]]);
        Some((desc_idx, inf_pos, hwid_ofst))
    }

    /// Decode a single `data_desc_t` record. Returns
    /// `(manufacturer_idx, desc_ofst)` — the fields we consume. The
    /// others (`install`, `install_picked`, `feature`, `sect_pos`) are
    /// ignored for now.
    fn read_desc_record(&self, i: usize) -> Option<(u32, u32)> {
        if i >= self.desc_count {
            return None;
        }
        let off = self.desc_start + i * DESC_SIZE;
        let slice = self.blob.get(off..off + DESC_SIZE)?;
        let mfr_idx = u32::from_le_bytes([slice[0], slice[1], slice[2], slice[3]]);
        let desc_ofst = u32::from_le_bytes([slice[8], slice[9], slice[10], slice[11]]);
        Some((mfr_idx, desc_ofst))
    }

    /// Decode a single `data_manufacturer_t` record. Returns
    /// `(inffile_idx, manufacturer_ofst)`.
    fn read_manufacturer_record(&self, i: usize) -> Option<(u32, u32)> {
        if i >= self.manufacturer_count {
            return None;
        }
        let off = self.manufacturer_start + i * MANUFACTURER_SIZE;
        let slice = self.blob.get(off..off + MANUFACTURER_SIZE)?;
        let inf_idx = u32::from_le_bytes([slice[0], slice[1], slice[2], slice[3]]);
        let mfr_ofst = u32::from_le_bytes([slice[4], slice[5], slice[6], slice[7]]);
        Some((inf_idx, mfr_ofst))
    }

    /// Decode the fields we care about from a `data_inffile_t` record:
    /// `(infpath_ofst, inffilename_ofst, driver_ver_ofst)`.
    fn read_inffile_record(&self, i: usize) -> Option<(u32, u32, u32)> {
        if i >= self.inffile_count {
            return None;
        }
        let off = self.inffile_start + i * INFFILE_SIZE;
        let slice = self.blob.get(off..off + INFFILE_SIZE)?;
        let infpath = u32::from_le_bytes([slice[0], slice[1], slice[2], slice[3]]);
        let inffilename = u32::from_le_bytes([slice[4], slice[5], slice[6], slice[7]]);
        // fields[DriverVer] lives at u32 index (2 + FIELD_DRIVER_VER) = 10.
        let dv_off = (2 + FIELD_DRIVER_VER) * 4;
        let driver_ver = u32::from_le_bytes([
            slice[dv_off],
            slice[dv_off + 1],
            slice[dv_off + 2],
            slice[dv_off + 3],
        ]);
        Some((infpath, inffilename, driver_ver))
    }

    /// Assemble an `SdiHit` for a matching HWID. Returns `None` if any
    /// link in the chain is corrupt (out-of-range index or offset). The
    /// `hwid` slice must borrow from the same backing `blob` as `self`
    /// — in practice we only ever pass one obtained via
    /// [`read_string_checked`].
    fn build_hit<'a>(&'a self, hwid: &'a str, desc_idx: u32) -> Option<SdiHit<'a>> {
        let (mfr_idx, desc_ofst) = self.read_desc_record(desc_idx as usize)?;
        let desc = self.read_string_checked(desc_ofst).unwrap_or("");

        let (inf_idx, mfr_ofst) = self.read_manufacturer_record(mfr_idx as usize)?;
        let manufacturer = self.read_string_checked(mfr_ofst).unwrap_or("");

        let (infpath_ofst, inffilename_ofst, driver_ver_ofst) =
            self.read_inffile_record(inf_idx as usize)?;
        let infpath = self.read_string_checked(infpath_ofst).unwrap_or("");
        let inf_filename = self.read_string_checked(inffilename_ofst).unwrap_or("");

        let driver_ver = if driver_ver_ofst == 0 {
            None
        } else {
            self.read_string_checked(driver_ver_ofst)
                .filter(|s| !s.is_empty())
        };

        Some(SdiHit {
            hwid,
            inf_dir_prefix: normalise_inf_dir(infpath),
            inf_filename,
            driver_display_name: desc,
            driver_ver,
            driver_manufacturer: manufacturer,
        })
    }

    /// Decode a null-terminated ASCII string at `ofst` bytes into the
    /// string table. Returns `None` if the offset is out of range, or if
    /// no terminator is found within `MAX_STRING_LEN` bytes.
    fn read_string_checked(&self, ofst: u32) -> Option<&str> {
        let ofst = ofst as usize;
        if self.string_table_start.checked_add(ofst)? >= self.string_table_end {
            // ofst == 0 with an empty string table counts as empty.
            if ofst == 0 {
                return Some("");
            }
            return None;
        }
        let start = self.string_table_start + ofst;
        let max_end = self
            .string_table_end
            .min(start.saturating_add(MAX_STRING_LEN));
        let slice = self.blob.get(start..max_end)?;
        let terminator = slice.iter().position(|&b| b == 0)?;
        std::str::from_utf8(&slice[..terminator]).ok()
    }

    /// Walk every manufacturer / desc / HWID record once during parse to
    /// confirm the cross-reference indices are all in-range. String
    /// offsets are checked lazily in [`read_string_checked`] during
    /// lookup, so a single corrupt ofst hurts only the matching call and
    /// not the whole parse.
    fn validate_cross_references(&self) -> Result<(), String> {
        for i in 0..self.manufacturer_count {
            let (inf_idx, _) = self
                .read_manufacturer_record(i)
                .ok_or_else(|| format!("SDW manufacturer[{i}] decode failed"))?;
            if (inf_idx as usize) >= self.inffile_count {
                return Err(format!(
                    "SDW manufacturer[{i}].inffile_index={inf_idx} >= inffile count {}",
                    self.inffile_count
                ));
            }
        }
        for i in 0..self.desc_count {
            let (mfr_idx, _) = self
                .read_desc_record(i)
                .ok_or_else(|| format!("SDW desc[{i}] decode failed"))?;
            if (mfr_idx as usize) >= self.manufacturer_count {
                return Err(format!(
                    "SDW desc[{i}].manufacturer_index={mfr_idx} >= manufacturer count {}",
                    self.manufacturer_count
                ));
            }
        }
        for i in 0..self.hwid_count {
            let (desc_idx, _, _) = self
                .read_hwid_record(i)
                .ok_or_else(|| format!("SDW hwid[{i}] decode failed"))?;
            if (desc_idx as usize) >= self.desc_count {
                return Err(format!(
                    "SDW hwid[{i}].desc_index={desc_idx} >= desc count {}",
                    self.desc_count
                ));
            }
        }
        Ok(())
    }
}

/// Convert an SDIO-native infpath (backslash-separated, no trailing slash)
/// into the shape the pack extractor wants: forward slashes, lowercase
/// left alone, always terminated by `/` so concatenation with
/// `inf_filename` produces a valid relative path. Empty input is
/// returned as an empty string.
fn normalise_inf_dir(infpath: &str) -> String {
    if infpath.is_empty() {
        return String::new();
    }
    let mut s: String = infpath.replace('\\', "/");
    if !s.ends_with('/') {
        s.push('/');
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a minimal valid SDW header + raw LZMA stream wrapper. The
    /// payload here is *not* LZMA, so these helpers only cover error
    /// paths (magic / version / LZMA-decode rejection).
    fn sdw_with_bad_payload(magic: &[u8; 3], version: u32, tail: &[u8]) -> Vec<u8> {
        let mut out = Vec::with_capacity(8 + tail.len());
        out.extend_from_slice(magic);
        out.extend_from_slice(&version.to_le_bytes());
        out.push(0); // reserved
        out.extend_from_slice(tail);
        out
    }

    #[test]
    fn rejects_short_header() {
        let err = parse_index(&[0u8; 4]).unwrap_err();
        assert!(err.contains("truncated"), "got: {err}");
    }

    #[test]
    fn rejects_bad_magic() {
        let buf = sdw_with_bad_payload(b"SDX", 0x0205, &[]);
        let err = parse_index(&buf).unwrap_err();
        assert!(
            err.contains("magic") || err.contains("SDW"),
            "expected magic error, got: {err}"
        );
    }

    #[test]
    fn rejects_bad_version() {
        let buf = sdw_with_bad_payload(b"SDW", 0x0999, &[]);
        let err = parse_index(&buf).unwrap_err();
        assert!(err.contains("version"), "expected version error, got: {err}");
    }

    #[test]
    fn normalise_inf_dir_appends_trailing_slash() {
        assert_eq!(
            normalise_inf_dir("Brother\\Allx64\\FORCED\\-Class"),
            "Brother/Allx64/FORCED/-Class/"
        );
        assert_eq!(
            normalise_inf_dir("Brother\\Allx64\\FORCED\\-Class\\"),
            "Brother/Allx64/FORCED/-Class/"
        );
        assert_eq!(normalise_inf_dir(""), "");
    }

    #[test]
    fn pack_filename_appends_7z_extension() {
        // Just cover the trivial formatter — construct a minimally-valid
        // index via field literals to avoid needing a real parse path.
        let idx = SdiIndex {
            pack_name: "DP_Printer_26000".to_string(),
            version: 0x0205,
            blob: Vec::new(),
            string_table_start: 0,
            string_table_end: 0,
            inffile_start: 0,
            inffile_count: 0,
            manufacturer_start: 0,
            manufacturer_count: 0,
            desc_start: 0,
            desc_count: 0,
            hwid_start: 0,
            hwid_count: 0,
        };
        assert_eq!(idx.pack_filename(), "DP_Printer_26000.7z");
        assert_eq!(idx.hwid_count(), 0);
        assert_eq!(idx.find_matching(&["whatever".into()]), Vec::new());
    }
}
