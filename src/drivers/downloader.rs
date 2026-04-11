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
