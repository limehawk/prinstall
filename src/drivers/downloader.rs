use std::path::{Path, PathBuf};
use std::time::Duration;
use reqwest::Client;

use crate::drivers::manifest::UniversalDriver;

const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(60);
const MAX_FILE_SIZE: u64 = 500 * 1024 * 1024; // 500 MB
const STAGING_DIR: &str = r"C:\ProgramData\prinstall\staging";

/// Download and extract a driver package. Returns path to the directory
/// containing the INF file(s), or an error message.
pub async fn download_and_stage(driver: &UniversalDriver, verbose: bool) -> Result<PathBuf, String> {
    if driver.url.is_empty() {
        return Err(format!(
            "No download URL available for '{}'. Install this driver manually.",
            driver.name
        ));
    }

    let staging = PathBuf::from(STAGING_DIR);
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
    // Write CAB to temp file, then use expand.exe (Windows built-in)
    let cab_path = dest.join("__temp.cab");
    std::fs::write(&cab_path, bytes)
        .map_err(|e| format!("Failed to write CAB file: {e}"))?;

    if verbose {
        eprintln!("[cab] expanding {} → {}", cab_path.display(), dest.display());
    }

    let output = std::process::Command::new("expand")
        .args([
            cab_path.to_str().unwrap(),
            "-F:*",
            dest.to_str().unwrap(),
        ])
        .output()
        .map_err(|e| format!("Failed to run expand.exe: {e}"))?;

    // Clean up temp CAB
    std::fs::remove_file(&cab_path).ok();

    if !output.status.success() {
        return Err(format!(
            "CAB extraction failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
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
