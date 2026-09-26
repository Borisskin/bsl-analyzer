use bsl_search::{EmbedderConfig, SearchConfig, SearchEngine};
use project_model::Project;
use std::path::PathBuf;
use std::time::Instant;

fn main() {
    tracing_subscriber::fmt().with_max_level(tracing_subscriber::filter::LevelFilter::INFO).init();

    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!("Usage: search_demo <config_dir> [query]");
        eprintln!();
        eprintln!("Examples:");
        eprintln!("  search_demo ~/src/niagara_ut/src \"обработка проведения\"");
        eprintln!("  search_demo ~/src/niagara_ut/src");
        std::process::exit(1);
    }

    let project_dir = PathBuf::from(&args[1]);
    let query = args.get(2).map(|s| s.as_str());

    if !project_dir.is_dir() {
        eprintln!("Error: {} is not a directory", project_dir.display());
        std::process::exit(1);
    }

    let project = match Project::new(&project_dir) {
        Ok(project) => project,
        Err(e) => {
            eprintln!("Error: invalid project at {}: {e}", project_dir.display());
            std::process::exit(1);
        }
    };
    let config_dir = project.source_path().to_path_buf();

    let base_url =
        std::env::var("EMBEDDING_URL").unwrap_or_else(|_| "http://localhost:11434".to_owned());
    let model = std::env::var("EMBEDDING_MODEL").unwrap_or_else(|_| "qwen3-embedding".to_owned());
    let dim: usize =
        std::env::var("EMBEDDING_DIM").ok().and_then(|s| s.parse().ok()).unwrap_or(1024);
    let batch_size: usize =
        std::env::var("EMBEDDING_BATCH_SIZE").ok().and_then(|s| s.parse().ok()).unwrap_or(32);

    let api_key = std::env::var("EMBEDDING_API_KEY").ok();

    let concurrency: usize =
        std::env::var("EMBEDDING_CONCURRENCY").ok().and_then(|s| s.parse().ok()).unwrap_or(10);
    let max_request_bytes = EmbedderConfig::request_bytes_from_env().unwrap_or_else(|error| {
        eprintln!("Error: {error}");
        std::process::exit(1);
    });

    let config = SearchConfig {
        embedder: EmbedderConfig {
            base_url,
            model: model.clone(),
            dim: Some(dim),
            api_key: api_key.clone(),
            provider: std::env::var("EMBEDDING_PROVIDER").ok(),
            max_request_bytes,
        },
        execution: bsl_search::EmbeddingExecutionPolicy {
            batch_size,
            concurrency,
            progress_interval: 20,
        },
    };

    let build_dir = project_dir.join(".build");
    std::fs::create_dir_all(&build_dir).ok();
    let db_path = build_dir.join("bsl-search.db");

    println!("=== BSL Search Demo ===");
    println!("Config dir:  {}", config_dir.display());
    println!("Database:    {}", db_path.display());
    println!("Model:       {model}");
    println!("Dimension:   {dim}");
    println!("Batch size:  {batch_size}");
    println!();

    let embedder = bsl_search::Embedder::new(config.embedder.clone());
    if let Err(e) = embedder.health_check() {
        eprintln!("Error: embedding service not available: {e}");
        eprintln!();
        eprintln!("Make sure ollama is running:");
        eprintln!("  ollama serve");
        eprintln!("  ollama pull qwen3-embedding");
        std::process::exit(1);
    }
    println!("Embedding service: OK");

    let mut engine = match SearchEngine::new(&db_path, config) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("Error creating search engine: {e}");
            std::process::exit(1);
        }
    };

    println!(
        "Existing index: {} files, {} chunks, {} vectors",
        engine.file_count().unwrap_or(0),
        engine.chunk_count().unwrap_or(0),
        engine.vector_count(),
    );
    println!();

    println!("Indexing...");
    let start = Instant::now();
    match engine.index_directory(&config_dir, None) {
        Ok(indexed) => {
            let elapsed = start.elapsed();
            println!(
                "Indexed {indexed} files in {:.1}s ({} total chunks, {} vectors)",
                elapsed.as_secs_f64(),
                engine.chunk_count().unwrap_or(0),
                engine.vector_count(),
            );
        }
        Err(e) => {
            eprintln!("Indexing error: {e}");
            std::process::exit(1);
        }
    }
    println!();

    if let Some(query) = query {
        let (mode, actual_query) = if let Some(q) = query.strip_prefix("fts:") {
            ("FTS", q.trim())
        } else {
            ("semantic", query)
        };

        println!("Searching ({mode}): \"{actual_query}\"");
        println!("{}", "─".repeat(60));

        let start = Instant::now();
        let result = if mode == "FTS" {
            engine.text_search(actual_query, 10, None)
        } else {
            engine.search(actual_query, 10, None)
        };

        match result {
            Ok(hits) => {
                let elapsed = start.elapsed();
                println!(
                    "Found {} results in {:.1}ms\n",
                    hits.len(),
                    elapsed.as_secs_f64() * 1000.0
                );

                for (i, hit) in hits.iter().enumerate() {
                    println!(
                        "#{} [{:.3}] {} :: {} ({})",
                        i + 1,
                        hit.score,
                        hit.file_path,
                        if hit.symbol_name.is_empty() { "<header>" } else { &hit.symbol_name },
                        hit.kind,
                    );
                    println!("   Lines {}-{}", hit.line_start + 1, hit.line_end);

                    let preview: String = hit
                        .text
                        .lines()
                        .take(3)
                        .map(|l| format!("   │ {l}"))
                        .collect::<Vec<_>>()
                        .join("\n");
                    println!("{preview}");
                    println!();
                }
            }
            Err(e) => {
                eprintln!("Search error: {e}");
            }
        }
    } else {
        println!("No query provided. Run with a query argument to search:");
        println!(
            "  cargo run -p bsl-search --example search_demo -- {} \"your query\"",
            config_dir.display()
        );
        println!("  # Prefix 'fts:' for full-text search:");
        println!(
            "  cargo run -p bsl-search --example search_demo -- {} \"fts:ОбработкаПроведения\"",
            config_dir.display()
        );
    }
}
