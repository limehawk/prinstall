//! Microsoft Update Catalog scraper.
//!
//! This is a Rust port of the core HTTP/HTML flow used by the
//! [MSCatalogLTS](https://github.com/Marco-online/MSCatalogLTS) PowerShell
//! module, without the transitive dependency footprint of pulling in a PS
//! module at runtime. The public Windows Update Catalog has no official API,
//! so we scrape the classic ASP.NET search page and the download dialog.
//!
//! Two entrypoints:
//! - [`search`] — `GET https://www.catalog.update.microsoft.com/Search.aspx?q=<query>`
//!   and parse the result table (`#ctl00_catalogBody_updateMatches`) into a
//!   list of [`CatalogUpdate`] rows.
//! - [`download_urls`] — `POST https://www.catalog.update.microsoft.com/DownloadDialog.aspx`
//!   with an `UpdateIDs` form body and regex-extract the direct CDN URLs from
//!   the JavaScript response blob.
//!
//! Errors are returned as `Result<T, String>` to match the rest of the
//! `drivers/` modules.

use std::time::Duration;

use scraper::{Html, Selector};
use serde::{Deserialize, Serialize};

const SEARCH_URL: &str = "https://www.catalog.update.microsoft.com/Search.aspx";
const DOWNLOAD_URL: &str = "https://www.catalog.update.microsoft.com/DownloadDialog.aspx";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const USER_AGENT: &str = concat!("prinstall/", env!("CARGO_PKG_VERSION"));

/// One row from the Microsoft Update Catalog search results.
///
/// Fields mirror the eight `<td>` columns in the catalog's result table.
/// `guid` is the `<input id="...">` inside the "Download" column button —
/// that's what you pass to [`download_urls`] to resolve actual CDN links.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CatalogUpdate {
    pub title: String,
    pub products: String,
    pub classification: String,
    /// Raw date string as shown in the catalog (e.g. `"10/8/2024"`). Kept
    /// as a string because catalog date formatting is locale-dependent and
    /// date parsing is not load-bearing for driver selection.
    pub last_updated: String,
    pub version: String,
    /// Human-readable size (e.g. `"25.7 MB"`).
    pub size: String,
    /// Exact byte count from the hidden `<span>` in the size column.
    pub size_bytes: u64,
    /// Update GUID — the argument for [`download_urls`].
    pub guid: String,
}

/// Search the Microsoft Update Catalog. Returns an empty vec if the catalog
/// reports no results (not an error).
pub async fn search(query: &str) -> Result<Vec<CatalogUpdate>, String> {
    let client = build_client()?;

    let url = format!("{SEARCH_URL}?q={}", url_encode(query));
    let resp = client
        .get(&url)
        .header("Cache-Control", "no-cache")
        .header("Pragma", "no-cache")
        .send()
        .await
        .map_err(|e| format!("Catalog request failed: {e}"))?;

    if !resp.status().is_success() {
        return Err(format!(
            "Catalog returned HTTP {} for query '{query}'",
            resp.status()
        ));
    }

    let html = resp
        .text()
        .await
        .map_err(|e| format!("Failed to read catalog response body: {e}"))?;

    parse_search_html(&html)
}

/// Resolve actual download URLs for a catalog entry by GUID. Most driver
/// updates return one or two URLs (typically `.cab` files on
/// `download.windowsupdate.com`).
pub async fn download_urls(guid: &str) -> Result<Vec<String>, String> {
    if guid.is_empty() {
        return Err("Empty GUID passed to download_urls".into());
    }

    let client = build_client()?;

    // The catalog's DownloadDialog.aspx endpoint expects a JSON-encoded array
    // in an `UpdateIDs` form field. This is exactly the shape MSCatalogLTS
    // uses; changing it breaks the response.
    let update_ids = format!(r#"[{{"size":0,"UpdateID":"{guid}","UpdateIDInfo":"{guid}"}}]"#);
    // Manual form-urlencoding of the single `UpdateIDs` field. We avoid
    // enabling a reqwest feature flag just for .form() on a single call site.
    let body = format!("UpdateIDs={}", url_encode(&update_ids));

    let resp = client
        .post(DOWNLOAD_URL)
        .header("Content-Type", "application/x-www-form-urlencoded")
        .body(body)
        .send()
        .await
        .map_err(|e| format!("DownloadDialog request failed: {e}"))?;

    if !resp.status().is_success() {
        return Err(format!(
            "DownloadDialog returned HTTP {} for GUID {guid}",
            resp.status()
        ));
    }

    let body = resp
        .text()
        .await
        .map_err(|e| format!("Failed to read DownloadDialog body: {e}"))?;

    Ok(extract_download_urls(&body))
}

fn build_client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .timeout(REQUEST_TIMEOUT)
        .build()
        .map_err(|e| format!("HTTP client init failed: {e}"))
}

/// Parse the search results HTML into a list of catalog updates.
///
/// Also detects the catalog's "no results" and error page markers so that
/// an empty result set is distinguishable from a transport failure.
fn parse_search_html(html: &str) -> Result<Vec<CatalogUpdate>, String> {
    let doc = Html::parse_document(html);

    // Catalog's own error markers. Selectors built from literals can't fail,
    // so unwrap is safe here.
    let no_results_sel = Selector::parse("#ctl00_catalogBody_noResultText").unwrap();
    if doc.select(&no_results_sel).next().is_some() {
        return Ok(Vec::new());
    }

    let error_sel = Selector::parse("#errorPageDisplayedError").unwrap();
    if let Some(err) = doc.select(&error_sel).next() {
        let text = err.text().collect::<String>().trim().to_string();
        return Err(format!("Catalog error page: {text}"));
    }

    let row_sel = Selector::parse("#ctl00_catalogBody_updateMatches tr").unwrap();
    let cell_sel = Selector::parse("td").unwrap();
    let span_sel = Selector::parse("span").unwrap();
    let input_sel = Selector::parse("input").unwrap();

    let mut updates = Vec::new();
    for row in doc.select(&row_sel) {
        if row.value().attr("id") == Some("headerRow") {
            continue;
        }

        let cells: Vec<_> = row.select(&cell_sel).collect();
        if cells.len() < 8 {
            continue;
        }

        let title = clean_text(&cells[1].text().collect::<String>());
        let products = clean_text(&cells[2].text().collect::<String>());
        let classification = clean_text(&cells[3].text().collect::<String>());
        let last_updated = clean_text(&cells[4].text().collect::<String>());
        let version = clean_text(&cells[5].text().collect::<String>());

        // Size column: <span>display</span><span>bytes</span>
        let size_spans: Vec<_> = cells[6].select(&span_sel).collect();
        let size = size_spans
            .first()
            .map(|s| clean_text(&s.text().collect::<String>()))
            .unwrap_or_default();
        let size_bytes = size_spans
            .get(1)
            .and_then(|s| s.text().collect::<String>().trim().parse::<u64>().ok())
            .unwrap_or(0);

        // Last column has an <input id="<guid>"> used as the "Download" button.
        let guid = cells[7]
            .select(&input_sel)
            .next()
            .and_then(|i| i.value().attr("id"))
            .unwrap_or_default()
            .to_string();

        if !title.is_empty() && !guid.is_empty() {
            updates.push(CatalogUpdate {
                title,
                products,
                classification,
                last_updated,
                version,
                size,
                size_bytes,
                guid,
            });
        }
    }

    Ok(updates)
}

/// Extract direct CDN URLs from a DownloadDialog.aspx JavaScript response.
///
/// The response is an HTML page with an inline `<script>` that assigns URLs
/// to a `downloadInformation` array. We match every
/// `downloadInformation[N].files[M].url = '…'` and return the unique set,
/// preserving the order of first appearance.
fn extract_download_urls(body: &str) -> Vec<String> {
    // Lazy-initialised because regex compilation isn't free. Using a OnceLock
    // avoids pulling in `once_cell` as a dep.
    use std::sync::OnceLock;
    static RE: OnceLock<regex::Regex> = OnceLock::new();
    let re = RE.get_or_init(|| {
        regex::Regex::new(r"downloadInformation\[\d+\]\.files\[\d+\]\.url\s*=\s*'([^']*)'")
            .expect("static regex is valid")
    });

    let mut urls: Vec<String> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for cap in re.captures_iter(body) {
        if let Some(m) = cap.get(1) {
            // MSCatalogLTS normalizes `www.download.windowsupdate` to the
            // canonical `download.windowsupdate` host. We mirror the same.
            let url = m
                .as_str()
                .replace("www.download.windowsupdate", "download.windowsupdate");
            if seen.insert(url.clone()) {
                urls.push(url);
            }
        }
    }
    urls
}

/// Minimal application/x-www-form-urlencoded encoder for a single value.
/// We only ever feed this the `UpdateIDs` JSON blob, which contains
/// ASCII letters, digits, and a fixed set of punctuation — encode anything
/// that isn't an unreserved char per RFC 3986. Not a general-purpose
/// encoder; don't reach for it outside this module.
fn url_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 3);
    for byte in s.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char);
            }
            _ => {
                out.push('%');
                out.push_str(&format!("{byte:02X}"));
            }
        }
    }
    out
}

/// Collapse whitespace and trim — catalog HTML is full of &nbsp; and
/// indentation from the ASP.NET rendering layer.
fn clean_text(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut last_was_space = true; // trims leading whitespace
    for ch in s.chars() {
        if ch.is_whitespace() {
            if !last_was_space {
                out.push(' ');
                last_was_space = true;
            }
        } else {
            out.push(ch);
            last_was_space = false;
        }
    }
    if out.ends_with(' ') {
        out.pop();
    }
    out
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// Minimal HTML fixture matching the real catalog structure. Two result
    /// rows plus the header row. Columns: checkbox, title, products,
    /// classification, date, version, size (two spans), download input.
    fn two_row_fixture() -> &'static str {
        r##"
<html><body>
<table id="ctl00_catalogBody_updateMatches">
  <tr id="headerRow">
    <th></th><th>Title</th><th>Products</th><th>Classification</th>
    <th>Last Updated</th><th>Version</th><th>Size</th><th>Download</th>
  </tr>
  <tr>
    <td><input type="checkbox"/></td>
    <td>  Brother - Printers - 1.0.0.0  </td>
    <td>Windows 11 Client, version 22H2 and later, Servicing Drivers</td>
    <td>Drivers (Printers)</td>
    <td>  10/8/2024  </td>
    <td>1.0.0.0</td>
    <td><span>25.7 MB</span><span>26943488</span></td>
    <td><input id="aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee" type="button"/></td>
  </tr>
  <tr>
    <td><input type="checkbox"/></td>
    <td>Brother
      MFC-L2750DW series</td>
    <td>Windows 10</td>
    <td>Drivers (Printers)</td>
    <td>3/15/2023</td>
    <td>2.1</td>
    <td><span>15.2 MB</span><span>15938355</span></td>
    <td><input id="11111111-2222-3333-4444-555555555555"/></td>
  </tr>
</table>
</body></html>
        "##
    }

    #[test]
    fn parse_search_returns_both_rows() {
        let updates = parse_search_html(two_row_fixture()).expect("parse ok");
        assert_eq!(updates.len(), 2);
    }

    #[test]
    fn parse_search_extracts_title_and_guid() {
        let updates = parse_search_html(two_row_fixture()).unwrap();
        assert_eq!(updates[0].title, "Brother - Printers - 1.0.0.0");
        assert_eq!(updates[0].guid, "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee");
        assert_eq!(updates[1].title, "Brother MFC-L2750DW series");
        assert_eq!(updates[1].guid, "11111111-2222-3333-4444-555555555555");
    }

    #[test]
    fn parse_search_extracts_size_bytes() {
        let updates = parse_search_html(two_row_fixture()).unwrap();
        assert_eq!(updates[0].size, "25.7 MB");
        assert_eq!(updates[0].size_bytes, 26943488);
        assert_eq!(updates[1].size_bytes, 15938355);
    }

    #[test]
    fn parse_search_extracts_classification_and_date() {
        let updates = parse_search_html(two_row_fixture()).unwrap();
        assert_eq!(updates[0].classification, "Drivers (Printers)");
        assert_eq!(updates[0].last_updated, "10/8/2024");
    }

    #[test]
    fn parse_search_handles_no_results_page() {
        let html = r#"<html><body>
            <span id="ctl00_catalogBody_noResultText">No results found</span>
        </body></html>"#;
        let updates = parse_search_html(html).unwrap();
        assert!(updates.is_empty());
    }

    #[test]
    fn parse_search_reports_catalog_error_page() {
        let html = r#"<html><body>
            <div id="errorPageDisplayedError">Error code 8DDD0010</div>
        </body></html>"#;
        let result = parse_search_html(html);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.contains("8DDD0010"), "error should mention code: {err}");
    }

    #[test]
    fn parse_search_skips_rows_with_too_few_cells() {
        let html = r#"<html><body>
            <table id="ctl00_catalogBody_updateMatches">
              <tr><td>only one cell</td></tr>
            </table>
        </body></html>"#;
        let updates = parse_search_html(html).unwrap();
        assert!(updates.is_empty());
    }

    #[test]
    fn parse_search_empty_table_returns_empty_vec() {
        let html = r#"<html><body>
            <table id="ctl00_catalogBody_updateMatches"></table>
        </body></html>"#;
        let updates = parse_search_html(html).unwrap();
        assert!(updates.is_empty());
    }

    #[test]
    fn extract_download_urls_finds_single_url() {
        let body = r#"
            <script>
            downloadInformation[0].files[0].url = 'http://download.windowsupdate.com/c/msdownload/update/driver/drvs/2024/10/foo.cab';
            </script>
        "#;
        let urls = extract_download_urls(body);
        assert_eq!(urls.len(), 1);
        assert!(urls[0].ends_with("/foo.cab"));
    }

    #[test]
    fn extract_download_urls_normalizes_www_host() {
        let body = r#"downloadInformation[0].files[0].url = 'http://www.download.windowsupdate.com/d/driver.cab';"#;
        let urls = extract_download_urls(body);
        assert_eq!(urls.len(), 1);
        assert_eq!(urls[0], "http://download.windowsupdate.com/d/driver.cab");
    }

    #[test]
    fn extract_download_urls_deduplicates() {
        let body = r#"
            downloadInformation[0].files[0].url = 'http://download.windowsupdate.com/same.cab';
            downloadInformation[0].files[1].url = 'http://download.windowsupdate.com/same.cab';
            downloadInformation[1].files[0].url = 'http://download.windowsupdate.com/other.cab';
        "#;
        let urls = extract_download_urls(body);
        assert_eq!(urls.len(), 2);
        assert!(urls[0].ends_with("/same.cab"));
        assert!(urls[1].ends_with("/other.cab"));
    }

    #[test]
    fn extract_download_urls_returns_empty_on_no_match() {
        let body = "<html><body>nothing here</body></html>";
        let urls = extract_download_urls(body);
        assert!(urls.is_empty());
    }

    #[test]
    fn url_encode_handles_json_blob() {
        // The real payload shape we pass to DownloadDialog.aspx.
        let input = r#"[{"size":0,"UpdateID":"abc-123","UpdateIDInfo":"abc-123"}]"#;
        let out = url_encode(input);
        // Letters/digits/dashes untouched
        assert!(out.contains("size"));
        assert!(out.contains("abc-123"));
        // Special chars encoded
        assert!(out.contains("%5B")); // [
        assert!(out.contains("%7B")); // {
        assert!(out.contains("%22")); // "
        assert!(out.contains("%3A")); // :
        assert!(out.contains("%2C")); // ,
    }

    #[test]
    fn clean_text_collapses_whitespace() {
        assert_eq!(clean_text("  hello   world  "), "hello world");
        assert_eq!(clean_text("\n\tfoo\n\tbar\n"), "foo bar");
        assert_eq!(clean_text(""), "");
        assert_eq!(clean_text("   "), "");
    }
}
