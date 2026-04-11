//! Integration tests for `src/drivers/sdi/fetcher.rs`.
//!
//! Each test spawns a hand-rolled HTTP mock server on an ephemeral
//! localhost port, serves a small set of canned routes, then drives
//! `fetch_manifest` / `fetch_index` / `fetch_pack` against it.
//!
//! The mock is ~80 lines of raw `TcpListener` + HTTP/1.1 responder, on
//! purpose — it's simpler than pulling `wiremock` or `httpmock` into
//! `[dev-dependencies]` and keeps the test file self-contained.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;

use prinstall::drivers::sdi::fetcher::{
    fetch_index, fetch_manifest, fetch_pack, IndexBundleManifest, ManifestAsset,
};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::task::JoinHandle;

/// A canned response: (status_code, body_bytes).
type Response = (u16, Vec<u8>);

/// Spawn a tiny mock HTTP server on `127.0.0.1:0`. Returns the base
/// URL (e.g. `http://127.0.0.1:54321/`) and a `JoinHandle` for the
/// background task. The handle is dropped (and the server torn down)
/// at the end of each test.
async fn spawn_mock_mirror(routes: HashMap<String, Response>) -> (String, JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr: SocketAddr = listener.local_addr().unwrap();
    let routes = Arc::new(routes);

    let handle = tokio::spawn(async move {
        loop {
            let (mut stream, _peer) = match listener.accept().await {
                Ok(x) => x,
                Err(_) => return,
            };
            let routes = routes.clone();
            tokio::spawn(async move {
                let mut buf = vec![0u8; 8192];
                let n = match stream.read(&mut buf).await {
                    Ok(n) if n > 0 => n,
                    _ => return,
                };
                let req_str = String::from_utf8_lossy(&buf[..n]).into_owned();
                let first_line = req_str.lines().next().unwrap_or("");
                // "GET /path HTTP/1.1"
                let path = first_line.split_whitespace().nth(1).unwrap_or("/");

                let (status, body) = match routes.get(path) {
                    Some(r) => (r.0, r.1.clone()),
                    None => (404, b"not found".to_vec()),
                };

                let reason = match status {
                    200 => "OK",
                    404 => "Not Found",
                    500 => "Internal Server Error",
                    _ => "Other",
                };

                let header = format!(
                    "HTTP/1.1 {status} {reason}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = stream.write_all(header.as_bytes()).await;
                let _ = stream.write_all(&body).await;
                let _ = stream.shutdown().await;
            });
        }
    });

    (format!("http://{}/", addr), handle)
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(bytes);
    let out = h.finalize();
    let mut s = String::with_capacity(64);
    for b in out {
        s.push_str(&format!("{:02x}", b));
    }
    s
}

fn canned_manifest_json() -> Vec<u8> {
    let json = r#"{
        "version": "sdi-printer-v1",
        "generated_at": "2026-04-11T12:00:00Z",
        "indexes": [
            { "name": "DP_Printer_26000.bin", "size_bytes": 11, "sha256": "fakehash" }
        ],
        "packs": [
            { "name": "DP_Printer_26000.7z", "size_bytes": 22, "sha256": "fakehash" }
        ]
    }"#;
    json.as_bytes().to_vec()
}

fn tmp_dir(tag: &str) -> std::path::PathBuf {
    let base = std::env::temp_dir().join(format!(
        "prinstall-sdi-fetcher-{}-{}",
        tag,
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&base).unwrap();
    base
}

// ---------------------------------------------------------------------

#[tokio::test]
async fn fetch_manifest_returns_parsed_structure() {
    let mut routes: HashMap<String, Response> = HashMap::new();
    routes.insert("/manifest.json".to_string(), (200, canned_manifest_json()));

    let (base_url, _handle) = spawn_mock_mirror(routes).await;

    let m: IndexBundleManifest = fetch_manifest(&base_url).await.unwrap();
    assert_eq!(m.version, "sdi-printer-v1");
    assert_eq!(m.indexes.len(), 1);
    assert_eq!(m.packs.len(), 1);
    assert_eq!(m.indexes[0].name, "DP_Printer_26000.bin");
    assert_eq!(m.packs[0].name, "DP_Printer_26000.7z");
}

#[tokio::test]
async fn fetch_manifest_rejects_bad_json() {
    let mut routes: HashMap<String, Response> = HashMap::new();
    routes.insert(
        "/manifest.json".to_string(),
        (200, b"not valid json {".to_vec()),
    );

    let (base_url, _handle) = spawn_mock_mirror(routes).await;

    let err = fetch_manifest(&base_url).await.unwrap_err();
    assert!(
        err.to_lowercase().contains("json"),
        "error should mention JSON parse failure, got: {err}"
    );
}

#[tokio::test]
async fn fetch_manifest_rejects_http_404() {
    // Empty route map — every path returns 404 by default.
    let routes: HashMap<String, Response> = HashMap::new();
    let (base_url, _handle) = spawn_mock_mirror(routes).await;

    let err = fetch_manifest(&base_url).await.unwrap_err();
    let low = err.to_lowercase();
    assert!(
        low.contains("404") || low.contains("not found"),
        "error should mention 404 or not found, got: {err}"
    );
}

#[tokio::test]
async fn fetch_index_verifies_sha256_success() {
    let content = b"index-file-body-bytes";
    let hash = sha256_hex(content);

    let mut routes: HashMap<String, Response> = HashMap::new();
    routes.insert("/DP_Printer.bin".to_string(), (200, content.to_vec()));

    let (base_url, _handle) = spawn_mock_mirror(routes).await;

    let asset = ManifestAsset {
        name: "DP_Printer.bin".to_string(),
        size_bytes: content.len() as u64,
        sha256: hash,
    };

    let dest = tmp_dir("idx-ok");
    let path = fetch_index(&base_url, &asset, &dest).await.unwrap();
    assert_eq!(path, dest.join("DP_Printer.bin"));

    let on_disk = std::fs::read(&path).unwrap();
    assert_eq!(on_disk, content);
}

#[tokio::test]
async fn fetch_index_rejects_sha256_mismatch() {
    let content = b"good content";

    let mut routes: HashMap<String, Response> = HashMap::new();
    routes.insert("/evil.bin".to_string(), (200, content.to_vec()));

    let (base_url, _handle) = spawn_mock_mirror(routes).await;

    let asset = ManifestAsset {
        name: "evil.bin".to_string(),
        size_bytes: content.len() as u64,
        // Wrong hash on purpose.
        sha256: "deadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef".to_string(),
    };

    let dest = tmp_dir("idx-mismatch");
    let err = fetch_index(&base_url, &asset, &dest).await.unwrap_err();
    let low = err.to_lowercase();
    assert!(
        low.contains("sha256") || low.contains("hash mismatch") || low.contains("mismatch"),
        "error should mention sha256/hash mismatch, got: {err}"
    );

    // Ensure no final file and no .downloading leftover.
    assert!(!dest.join("evil.bin").exists(), "final file should not exist");
    assert!(
        !dest.join("evil.bin.downloading").exists(),
        ".downloading temp should have been cleaned up"
    );
}

#[tokio::test]
async fn fetch_pack_writes_file_atomically() {
    // Synthetic ~100 KB pack.
    let content: Vec<u8> = (0..100 * 1024).map(|i| (i % 251) as u8).collect();
    let hash = sha256_hex(&content);

    let mut routes: HashMap<String, Response> = HashMap::new();
    routes.insert("/DP_Test.7z".to_string(), (200, content.clone()));

    let (base_url, _handle) = spawn_mock_mirror(routes).await;

    let asset = ManifestAsset {
        name: "DP_Test.7z".to_string(),
        size_bytes: content.len() as u64,
        sha256: hash,
    };

    let dest = tmp_dir("pack-atomic");
    let path = fetch_pack(&base_url, &asset, 10, &dest, false).await.unwrap();
    assert_eq!(path, dest.join("DP_Test.7z"));
    assert!(path.exists());
    assert!(!dest.join("DP_Test.7z.downloading").exists());

    let on_disk = std::fs::read(&path).unwrap();
    assert_eq!(on_disk.len(), content.len());
    assert_eq!(on_disk, content);
}

#[tokio::test]
async fn fetch_pack_enforces_max_size_mb() {
    // Claim the pack is 50 MB but cap at 10 MB.
    let asset = ManifestAsset {
        name: "Huge.7z".to_string(),
        size_bytes: 50 * 1024 * 1024,
        sha256: "0".repeat(64),
    };

    // No routes registered — if the size guard fails and we actually
    // fetch, the 404 error will surface instead of the size error.
    let routes: HashMap<String, Response> = HashMap::new();
    let (base_url, _handle) = spawn_mock_mirror(routes).await;

    let dest = tmp_dir("pack-too-big");
    let err = fetch_pack(&base_url, &asset, 10, &dest, false)
        .await
        .unwrap_err();
    let low = err.to_lowercase();
    assert!(
        low.contains("max_size_mb") || low.contains("exceeds") || low.contains("size"),
        "error should mention size guard, got: {err}"
    );

    // Confirm no file was written.
    assert!(!dest.join("Huge.7z").exists());
    assert!(!dest.join("Huge.7z.downloading").exists());
}

#[tokio::test]
async fn fetch_pack_streams_large_content() {
    // ~10 MB of deterministic content.
    let content: Vec<u8> = (0..10 * 1024 * 1024).map(|i| (i % 256) as u8).collect();
    let hash = sha256_hex(&content);

    let mut routes: HashMap<String, Response> = HashMap::new();
    routes.insert("/Big.7z".to_string(), (200, content.clone()));

    let (base_url, _handle) = spawn_mock_mirror(routes).await;

    let asset = ManifestAsset {
        name: "Big.7z".to_string(),
        size_bytes: content.len() as u64,
        sha256: hash,
    };

    let dest = tmp_dir("pack-streams");
    let path = fetch_pack(&base_url, &asset, 50, &dest, false).await.unwrap();
    let on_disk = std::fs::read(&path).unwrap();
    assert_eq!(on_disk.len(), content.len());
    // SHA match already checked internally, but verify equality just
    // to prove streaming didn't corrupt anything.
    assert_eq!(sha256_hex(&on_disk), sha256_hex(&content));
}

#[tokio::test]
async fn mirror_url_with_or_without_trailing_slash() {
    let content = b"trailing-slash-test";
    let hash = sha256_hex(content);

    let mut routes: HashMap<String, Response> = HashMap::new();
    routes.insert("/trailing.bin".to_string(), (200, content.to_vec()));

    let (base_url_with_slash, _h1) = spawn_mock_mirror(routes.clone()).await;
    let base_url_no_slash = base_url_with_slash
        .trim_end_matches('/')
        .to_string();

    let asset = ManifestAsset {
        name: "trailing.bin".to_string(),
        size_bytes: content.len() as u64,
        sha256: hash.clone(),
    };

    let dest1 = tmp_dir("slash-yes");
    let p1 = fetch_index(&base_url_with_slash, &asset, &dest1)
        .await
        .unwrap();
    assert!(p1.exists());

    // Start a second server for the no-slash case (each server dies
    // when its JoinHandle drops, so we can't reuse the first).
    let (base2, _h2) = spawn_mock_mirror(routes).await;
    let base2_no_slash = base2.trim_end_matches('/').to_string();
    let _ = base_url_no_slash; // capture for clarity, unused

    let dest2 = tmp_dir("slash-no");
    let p2 = fetch_index(&base2_no_slash, &asset, &dest2).await.unwrap();
    assert!(p2.exists());
}

#[tokio::test]
async fn rejects_asset_name_with_path_traversal() {
    let routes: HashMap<String, Response> = HashMap::new();
    let (base_url, _handle) = spawn_mock_mirror(routes).await;

    let asset = ManifestAsset {
        name: "../../etc/passwd".to_string(),
        size_bytes: 42,
        sha256: "0".repeat(64),
    };

    let dest = tmp_dir("traversal");
    let err = fetch_index(&base_url, &asset, &dest).await.unwrap_err();
    let low = err.to_lowercase();
    assert!(
        low.contains("traversal")
            || low.contains("refusing")
            || low.contains("separator")
            || low.contains("asset name"),
        "error should mention path traversal refusal, got: {err}"
    );

    // Verify nothing in the dest dir and nothing in the parent.
    assert!(!dest.join("passwd").exists());
    assert!(!dest.join("etc").exists());
    // And also walk up the chain that traversal would imply.
    let parent = dest.parent().unwrap();
    assert!(!parent.join("etc/passwd").exists());
}
