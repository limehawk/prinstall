use std::path::{Path, PathBuf};
use std::time::Duration;
use reqwest::Client;

use crate::drivers::manifest::UniversalDriver;

const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(60);
const MAX_FILE_SIZE: u64 = 500 * 1024 * 1024; // 500 MB

/// Download and extract a driver package. Returns path to the directory
/// containing the INF file(s), or an error message.
pub async fn download_and_stage(driver: &UniversalDriver, verbose: bool) -> Result<PathBuf, String> {
    if driver.url.is_empty() {
        return Err(format!(
            "No download URL available for '{}'. Install this driver manually.",
            driver.name
        ));
    }

    let staging = crate::paths::staging_dir();
    std::fs::create_dir_all(&staging)
        .map_err(|e| format!("Failed to create staging directory: {e}"))?;

    if verbose {
        eprintln!("[download] {} → {}", driver.url, staging.display());
    }

    // Download
    let client = Client::builder()
        .timeout(DOWNLOAD_TIMEOUT)
        .build()
        .map_err(|e| format!("HTTP client error: {e}"))?;

    let response = client
        .get(&driver.url)
        .send()
        .await
        .map_err(|e| format!("Download failed: {e}"))?;

    if !response.status().is_success() {
        return Err(format!(
            "HTTP {} for {}. Download manually: {}",
            response.status(),
            driver.name,
            driver.url
        ));
    }

    // Check content length
    if let Some(len) = response.content_length()
        && len > MAX_FILE_SIZE
    {
        return Err(format!(
            "Driver package is {} MB (max {} MB). Download manually: {}",
            len / 1024 / 1024,
            MAX_FILE_SIZE / 1024 / 1024,
            driver.url
        ));
    }

    let bytes = response
        .bytes()
        .await
        .map_err(|e| format!("Failed to read download: {e}"))?;

    // Extract based on format
    let extract_dir = staging.join(sanitize_name(&driver.name));
    std::fs::create_dir_all(&extract_dir)
        .map_err(|e| format!("Failed to create extract directory: {e}"))?;

    match driver.format.as_str() {
        "zip" => extract_zip(&bytes, &extract_dir, verbose)?,
        "cab" => extract_cab(&bytes, &extract_dir, verbose)?,
        other => return Err(format!("Unsupported format: {other}. Only zip and cab are supported.")),
    }

    if verbose {
        eprintln!("[extracted] → {}", extract_dir.display());
    }

    Ok(extract_dir)
}

/// Download a URL, sniff the bytes, extract, return the folder with INFs.
pub async fn download_and_extract_url(url: &str, verbose: bool) -> Result<PathBuf, String> {
    let staging = crate::paths::staging_dir();
    std::fs::create_dir_all(&staging)
        .map_err(|e| format!("Failed to create staging directory: {e}"))?;

    if verbose {
        eprintln!("[download] {url}");
    }

    let client = Client::builder()
        .timeout(DOWNLOAD_TIMEOUT)
        .build()
        .map_err(|e| format!("HTTP client error: {e}"))?;

    let response = client
        .get(url)
        .send()
        .await
        .map_err(|e| format!("Download failed: {e}"))?;

    if !response.status().is_success() {
        return Err(format!("HTTP {} for {url}", response.status()));
    }

    if let Some(len) = response.content_length()
        && len > MAX_FILE_SIZE
    {
        return Err(format!(
            "Driver package is {} MB (max {} MB)",
            len / 1024 / 1024,
            MAX_FILE_SIZE / 1024 / 1024
        ));
    }

    let bytes = response
        .bytes()
        .await
        .map_err(|e| format!("Failed to read download: {e}"))?;

    let extract_dir = staging.join(sanitize_name(url));
    let _ = std::fs::remove_dir_all(&extract_dir);
    std::fs::create_dir_all(&extract_dir)
        .map_err(|e| format!("Failed to create extract directory: {e}"))?;

    extract_bytes(&bytes, &extract_dir, verbose)?;

    if find_inf_files(&extract_dir).is_empty() {
        return Err(
            "no INF in this pack. Extract it yourself, then run prinstall driver add <folder>"
                .into(),
        );
    }

    if verbose {
        eprintln!("[extracted] → {}", extract_dir.display());
    }
    Ok(extract_dir)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PackFormat {
    Zip,
    Cab,
    SevenZ,
    Exe,
    Unknown,
}

fn sniff(bytes: &[u8]) -> PackFormat {
    if bytes.len() >= 4 && bytes.starts_with(b"PK\x03\x04") {
        PackFormat::Zip
    } else if bytes.len() >= 4 && bytes.starts_with(b"MSCF") {
        PackFormat::Cab
    } else if bytes.len() >= 6 && bytes[..6] == [0x37, 0x7A, 0xBC, 0xAF, 0x27, 0x1C] {
        PackFormat::SevenZ
    } else if bytes.len() >= 2 && bytes.starts_with(b"MZ") {
        PackFormat::Exe
    } else {
        PackFormat::Unknown
    }
}

fn extract_bytes(bytes: &[u8], dest: &Path, verbose: bool) -> Result<(), String> {
    std::fs::create_dir_all(dest)
        .map_err(|e| format!("Failed to create extract directory: {e}"))?;
    match sniff(bytes) {
        PackFormat::Zip => extract_zip(bytes, dest, verbose),
        PackFormat::Cab => extract_cab(bytes, dest, verbose),
        PackFormat::SevenZ => extract_7z(bytes, dest, verbose),
        PackFormat::Exe => extract_exe(bytes, dest, verbose),
        PackFormat::Unknown => extract_zip(bytes, dest, verbose)
            .or_else(|_| extract_cab(bytes, dest, verbose))
            .or_else(|_| extract_7z(bytes, dest, verbose)),
    }
}

fn extract_exe(bytes: &[u8], dest: &Path, verbose: bool) -> Result<(), String> {
    if extract_zip(bytes, dest, verbose).is_ok() && !find_inf_files(dest).is_empty() {
        return Ok(());
    }
    let _ = std::fs::remove_dir_all(dest);
    let _ = std::fs::create_dir_all(dest);
    if extract_7z(bytes, dest, verbose).is_ok() && !find_inf_files(dest).is_empty() {
        return Ok(());
    }
    Err(
        "this installer does not unpack. Extract it yourself, then run prinstall driver add <folder>"
            .into(),
    )
}

fn extract_7z(bytes: &[u8], dest: &Path, verbose: bool) -> Result<(), String> {
    #[cfg(not(feature = "sdi"))]
    {
        let _ = (bytes, dest, verbose);
        Err("7z extract is not in this build".into())
    }
    #[cfg(feature = "sdi")]
    {
        extract_7z_sdi(bytes, dest, verbose)
    }
}

#[cfg(feature = "sdi")]
fn extract_7z_sdi(bytes: &[u8], dest: &Path, verbose: bool) -> Result<(), String> {
    const MAGIC: &[u8] = &[0x37, 0x7A, 0xBC, 0xAF, 0x27, 0x1C];
    let payload = match bytes.windows(6).position(|w| w == MAGIC) {
        Some(n) => &bytes[n..],
        None => bytes,
    };
    let tmp = dest.join("_pack.7z");
    std::fs::write(&tmp, payload).map_err(|e| format!("Failed to write temp 7z: {e}"))?;
    if verbose {
        eprintln!("[7z] extracting {} bytes → {}", payload.len(), dest.display());
    }
    use sevenz_rust2::{ArchiveReader, Error as SzError, Password};
    let mut reader = ArchiveReader::open(&tmp, Password::empty())
        .map_err(|e| format!("7z open failed: {e}"))?;
    let dest_owned = dest.to_path_buf();
    let result = reader.for_each_entries(|entry, rdr| {
        if entry.is_directory() {
            return Ok(true);
        }
        let name = entry.name().replace('\\', "/");
        if name.split('/').any(|s| s == "..") {
            return Err(SzError::Other(std::borrow::Cow::Owned(format!(
                "7z entry '{name}' contains '..'"
            ))));
        }
        let mut outpath = dest_owned.clone();
        for seg in name.split('/').filter(|s| !s.is_empty() && *s != ".") {
            outpath.push(seg);
        }
        if let Some(parent) = outpath.parent() {
            std::fs::create_dir_all(parent).ok();
        }
        let mut outfile = std::fs::File::create(&outpath).map_err(|e| {
            SzError::Other(std::borrow::Cow::Owned(e.to_string()))
        })?;
        std::io::copy(rdr, &mut outfile)
            .map_err(|e| SzError::Other(std::borrow::Cow::Owned(e.to_string())))?;
        Ok(true)
    });
    let _ = std::fs::remove_file(&tmp);
    result.map_err(|e| format!("7z extract failed: {e}"))?;
    Ok(())
}

fn extract_zip(bytes: &[u8], dest: &Path, verbose: bool) -> Result<(), String> {
    let cursor = std::io::Cursor::new(bytes);
    let mut archive = zip::ZipArchive::new(cursor)
        .map_err(|e| format!("Invalid ZIP archive: {e}"))?;

    for i in 0..archive.len() {
        let mut file = archive.by_index(i)
            .map_err(|e| format!("ZIP read error: {e}"))?;

        let outpath = match file.enclosed_name() {
            Some(p) => dest.join(p),
            None => continue,
        };

        if file.is_dir() {
            std::fs::create_dir_all(&outpath).ok();
        } else {
            if let Some(parent) = outpath.parent() {
                std::fs::create_dir_all(parent).ok();
            }
            let mut outfile = std::fs::File::create(&outpath)
                .map_err(|e| format!("Failed to create {}: {e}", outpath.display()))?;
            std::io::copy(&mut file, &mut outfile)
                .map_err(|e| format!("Failed to write {}: {e}", outpath.display()))?;
        }

        if verbose {
            eprintln!("[zip] {}", outpath.display());
        }
    }

    Ok(())
}

fn extract_cab(bytes: &[u8], dest: &Path, verbose: bool) -> Result<(), String> {
    if verbose {
        eprintln!(
            "[cab] extracting {} bytes → {}",
            bytes.len(),
            dest.display()
        );
    }

    // Pure-Rust CAB extraction via the `cab` crate. Replaces the earlier
    // `expand.exe` subprocess — see src/drivers/cab.rs for the rationale.
    // Linux-testable, no Windows-only dependencies.
    let written = crate::drivers::cab::extract_cab_to_dir(bytes, dest)?;

    if verbose {
        for path in &written {
            eprintln!("[cab] {}", path.display());
        }
    }

    Ok(())
}

/// Find INF files in a directory (recursively).
pub fn find_inf_files(dir: &Path) -> Vec<PathBuf> {
    let mut results = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                results.extend(find_inf_files(&path));
            } else if path.extension().is_some_and(|e| e.eq_ignore_ascii_case("inf")) {
                results.push(path);
            }
        }
    }
    results
}

/// Sanitize a driver name for use as a directory name.
fn sanitize_name(name: &str) -> String {
    name.chars()
        .map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn zip_with_inf() -> Vec<u8> {
        let mut buf = Vec::new();
        {
            let mut zip = zip::ZipWriter::new(std::io::Cursor::new(&mut buf));
            let opts = zip::write::SimpleFileOptions::default();
            zip.start_file("Driver/oemsetup.inf", opts).unwrap();
            zip.write_all(b"[Version]\r\nSignature=\"$Windows NT$\"\r\n").unwrap();
            zip.finish().unwrap();
        }
        buf
    }

    #[test]
    fn sniff_zip_cab_7z_mz() {
        assert_eq!(sniff(b"PK\x03\x04rest"), PackFormat::Zip);
        assert_eq!(sniff(b"MSCFrest"), PackFormat::Cab);
        assert_eq!(
            sniff(&[0x37, 0x7A, 0xBC, 0xAF, 0x27, 0x1C, 0x00]),
            PackFormat::SevenZ
        );
        assert_eq!(sniff(b"MZ\x90\x00"), PackFormat::Exe);
        assert_eq!(sniff(b"????"), PackFormat::Unknown);
    }

    #[test]
    fn extract_bytes_unpacks_zip() {
        let dest = std::env::temp_dir().join(format!(
            "prinstall-zip-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dest);
        extract_bytes(&zip_with_inf(), &dest, false).unwrap();
        assert_eq!(find_inf_files(&dest).len(), 1);
        let _ = std::fs::remove_dir_all(&dest);
    }

    #[test]
    fn extract_bytes_unpacks_zip_sfx_exe() {
        let mut sfx = b"MZ this is a fake stub".to_vec();
        sfx.extend_from_slice(&zip_with_inf());
        let dest = std::env::temp_dir().join(format!(
            "prinstall-sfx-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dest);
        extract_bytes(&sfx, &dest, false).unwrap();
        assert_eq!(find_inf_files(&dest).len(), 1);
        let _ = std::fs::remove_dir_all(&dest);
    }
}
