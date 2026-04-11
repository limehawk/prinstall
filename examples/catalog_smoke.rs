use prinstall::drivers::catalog;

#[tokio::main]
async fn main() {
    let query = std::env::args().nth(1).unwrap_or_else(|| "Brother MFC-L2750DW".to_string());
    println!("Searching catalog for: {query}");
    match catalog::search(&query).await {
        Ok(updates) => {
            println!("Found {} update(s)", updates.len());
            for (i, u) in updates.iter().enumerate().take(10) {
                println!("  #{}: {} [{}, {}]", i + 1, u.title, u.size, u.last_updated);
                println!("      guid={}", u.guid);
            }
            if let Some(first) = updates.first() {
                println!("\nResolving download URLs for {}...", first.guid);
                match catalog::download_urls(&first.guid).await {
                    Ok(urls) => {
                        println!("  {} URL(s):", urls.len());
                        for u in &urls {
                            println!("    {}", u);
                        }
                    }
                    Err(e) => eprintln!("  error: {e}"),
                }
            }
        }
        Err(e) => eprintln!("Search failed: {e}"),
    }
}
