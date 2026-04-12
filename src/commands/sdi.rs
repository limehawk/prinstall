//! The `prinstall sdi` subcommand — cache management for the SDI
//! driver tier.
//!
//! ## Subcommands
//!
//! - `status` — show cache health, index count, pack count, total size
//! - `refresh` — re-download indexes from the configured mirror
//! - `list` — list cached indexes and packs with sizes
//! - `prefetch` — download all packs (or specific ones) so future installs are cache-local
//! - `clean` — drop cached packs past the size budget

use crate::config::AppConfig;
use crate::drivers::sdi::cache::SdiCache;

/// Run `prinstall sdi status`.
pub fn status(verbose: bool) {
    let cache = match SdiCache::load() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("SDI cache error: {e}");
            return;
        }
    };

    let indexes = cache.list_cached_indexes();
    let total_bytes = cache.total_cache_size_bytes();
    let pack_count = cache.metadata.packs.len();

    println!("SDI cache status:");
    println!("  Root:      {}", crate::paths::sdi_dir().display());
    println!("  Indexes:   {} file(s)", indexes.len());
    for idx in &indexes {
        if let Some(name) = idx.file_name() {
            println!("             {}", name.to_string_lossy());
        }
    }
    println!("  Packs:     {} cached", pack_count);
    for (name, meta) in &cache.metadata.packs {
        println!(
            "             {} ({:.1} MB, last used {})",
            name,
            meta.size_bytes as f64 / 1024.0 / 1024.0,
            meta.last_used.format("%Y-%m-%d %H:%M"),
        );
    }
    println!(
        "  Total:     {:.1} MB",
        total_bytes as f64 / 1024.0 / 1024.0
    );

    let config = AppConfig::load();
    if cache.is_stale(config.sdi.index_refresh_days) {
        match cache.metadata.last_refresh {
            Some(ts) => println!(
                "  Stale:     yes (last refresh {} — threshold {} days)",
                ts.format("%Y-%m-%d"),
                config.sdi.index_refresh_days,
            ),
            None => println!("  Stale:     yes (never refreshed)"),
        }
    } else if let Some(ts) = cache.metadata.last_refresh {
        println!("  Refresh:   {} (fresh)", ts.format("%Y-%m-%d %H:%M"));
    }

    println!("  Mirror:    {}", config.sdi.mirror_url);
    println!(
        "  Enabled:   {}",
        if config.sdi.enabled { "yes" } else { "no" }
    );
    if config.sdi.offline_mode {
        println!("  Offline:   yes");
    }

    if verbose {
        println!(
            "  Budget:    {} MB (max_cache_mb)",
            config.sdi.max_cache_mb
        );
        println!(
            "  Auto-fetch:{} (config.sdi.auto_fetch)",
            if config.sdi.auto_fetch { " yes" } else { " no" }
        );
    }
}

/// Run `prinstall sdi refresh`. Fetches manifest + indexes from the
/// configured mirror. Pack download is NOT triggered by refresh — that
/// happens lazily on HWID match or via `prinstall sdi prefetch`.
pub async fn refresh(verbose: bool) {
    let config = AppConfig::load();
    if config.sdi.offline_mode {
        eprintln!("SDI refresh skipped: offline mode is enabled in config.");
        return;
    }

    let mut cache = match SdiCache::load() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("SDI cache error: {e}");
            return;
        }
    };

    eprintln!("Fetching SDI manifest from {}...", config.sdi.mirror_url);
    let manifest = match crate::drivers::sdi::fetcher::fetch_manifest(&config.sdi.mirror_url).await
    {
        Ok(m) => m,
        Err(e) => {
            eprintln!("SDI refresh failed: {e}");
            eprintln!("The SDI mirror may not be published yet. Run `prinstall sdi status` to see cache state.");
            return;
        }
    };

    eprintln!(
        "Mirror version: {} ({} indexes, {} packs)",
        manifest.version,
        manifest.indexes.len(),
        manifest.packs.len()
    );

    let mut downloaded = 0;
    for asset in &manifest.indexes {
        if verbose {
            eprintln!("  Fetching index: {}", asset.name);
        }
        match crate::drivers::sdi::fetcher::fetch_index(
            &config.sdi.mirror_url,
            asset,
            &crate::paths::sdi_indexes_dir(),
        )
        .await
        {
            Ok(path) => {
                if verbose {
                    eprintln!("  → {}", path.display());
                }
                downloaded += 1;
            }
            Err(e) => {
                eprintln!("  Failed to fetch {}: {e}", asset.name);
            }
        }
    }

    if let Err(e) = cache.record_refresh(&manifest.version) {
        eprintln!("Warning: failed to update cache metadata: {e}");
    }

    eprintln!("SDI refresh complete: {downloaded} index(es) updated.");
}

/// Run `prinstall sdi list`. Shows what's in the cache.
pub fn list(_verbose: bool) {
    status(true);
}

/// Run `prinstall sdi prefetch`. Downloads all packs from the mirror.
pub async fn prefetch(verbose: bool) {
    let config = AppConfig::load();
    if config.sdi.offline_mode {
        eprintln!("SDI prefetch skipped: offline mode is enabled.");
        return;
    }

    let mut cache = match SdiCache::load() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("SDI cache error: {e}");
            return;
        }
    };

    // Refresh indexes first if stale
    if cache.is_stale(config.sdi.index_refresh_days) {
        eprintln!("Indexes are stale — refreshing first...");
        refresh(verbose).await;
        // Reload cache after refresh
        cache = match SdiCache::load() {
            Ok(c) => c,
            Err(e) => {
                eprintln!("SDI cache error after refresh: {e}");
                return;
            }
        };
    }

    let manifest = match crate::drivers::sdi::fetcher::fetch_manifest(&config.sdi.mirror_url).await
    {
        Ok(m) => m,
        Err(e) => {
            eprintln!("Cannot fetch manifest for prefetch: {e}");
            return;
        }
    };

    for asset in &manifest.packs {
        if cache.has_pack(&asset.name) {
            if verbose {
                eprintln!("  {} — already cached", asset.name);
            }
            continue;
        }

        eprintln!(
            "  Downloading {} ({:.1} MB)...",
            asset.name,
            asset.size_bytes as f64 / 1024.0 / 1024.0
        );
        match crate::drivers::sdi::fetcher::fetch_pack(
            &config.sdi.mirror_url,
            asset,
            config.sdi.max_cache_mb,
            &crate::paths::sdi_drivers_dir(),
            true, // show progress bar
        )
        .await
        {
            Ok(path) => {
                if let Err(e) = cache.register_pack(&asset.name, &path) {
                    eprintln!("  Warning: failed to register {}: {e}", asset.name);
                } else if verbose {
                    eprintln!("  → {}", path.display());
                }
            }
            Err(e) => {
                eprintln!("  Failed to download {}: {e}", asset.name);
            }
        }
    }

    eprintln!("SDI prefetch complete.");
}

/// Run `prinstall sdi clean`. Prune packs past the budget.
pub fn clean(verbose: bool) {
    let config = AppConfig::load();
    let mut cache = match SdiCache::load() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("SDI cache error: {e}");
            return;
        }
    };

    let before = cache.total_cache_size_bytes();
    match cache.prune(config.sdi.max_cache_mb) {
        Ok(removed) => {
            if removed.is_empty() {
                eprintln!(
                    "SDI cache is within budget ({:.1} MB / {} MB). Nothing to clean.",
                    before as f64 / 1024.0 / 1024.0,
                    config.sdi.max_cache_mb
                );
            } else {
                let after = cache.total_cache_size_bytes();
                eprintln!(
                    "Cleaned {} pack(s): {:.1} MB → {:.1} MB",
                    removed.len(),
                    before as f64 / 1024.0 / 1024.0,
                    after as f64 / 1024.0 / 1024.0,
                );
                if verbose {
                    for name in &removed {
                        eprintln!("  removed: {name}");
                    }
                }
            }
        }
        Err(e) => eprintln!("SDI clean failed: {e}"),
    }
}
