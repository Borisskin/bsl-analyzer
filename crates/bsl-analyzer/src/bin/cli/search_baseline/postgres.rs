use std::{env, error::Error, io, path::Path};

pub(super) fn resolve_project_url(
    postgres: &project_model::SearchPostgresConfig,
    mode: project_model::PostgresAccessMode,
) -> Result<project_model::ResolvedPostgresUrl, project_model::ResolvePostgresUrlError> {
    project_model::resolve_postgres_url(postgres, mode)
}

pub(super) fn build_project_adapter(
    source_dir: &Path,
    mode: project_model::PostgresAccessMode,
) -> Result<bsl_search::ExternalBaselineAdapter, Box<dyn Error + Send + Sync>> {
    let project = project_model::Project::new(source_dir)?;
    let resolved =
        resolve_project_url(&project.config.search.baseline.postgres, mode).map_err(|error| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("failed to resolve PostgreSQL {} credentials: {error}", mode.as_str()),
            )
        })?;
    build_adapter(&resolved.url, project.config.search.baseline.postgres.schema.as_deref())
}

pub(super) fn build_adapter(
    pg_url: &str,
    pg_schema: Option<&str>,
) -> Result<bsl_search::ExternalBaselineAdapter, Box<dyn Error + Send + Sync>> {
    let mut config = bsl_search::ExternalBaselineConfig::postgres(pg_url.to_owned());
    if let Some(schema) = pg_schema {
        config = config.with_schema(schema.to_owned());
    }
    Ok(bsl_search::ExternalBaselineAdapter::new(config)?)
}

pub(super) fn embedder_config(
    project: &project_model::Project,
) -> Result<Option<bsl_search::EmbedderConfig>, bsl_search::SearchError> {
    let emb = &project.config.search.baseline.embedding;

    let Some(model) = emb.model.clone().or_else(|| env::var("EMBEDDING_MODEL").ok()) else {
        return Ok(None);
    };
    let Some(base_url) = env::var("EMBEDDING_URL").ok().or_else(|| emb.url.clone()) else {
        return Ok(None);
    };
    let max_request_bytes = bsl_search::EmbedderConfig::request_bytes_from_env()?;
    let dim = emb
        .dimension
        .or_else(|| env::var("EMBEDDING_DIM").ok().and_then(|value| value.parse().ok()))
        .or(Some(1024));

    Ok(Some(bsl_search::EmbedderConfig {
        base_url,
        model,
        dim,
        api_key: env::var("EMBEDDING_API_KEY").ok(),
        provider: emb.provider.clone().or_else(|| env::var("EMBEDDING_PROVIDER").ok()),
        max_request_bytes,
    }))
}

pub(super) fn embedding_execution_policy_from_env() -> bsl_search::EmbeddingExecutionPolicy {
    bsl_search::EmbeddingExecutionPolicy {
        batch_size: env::var("EMBEDDING_BATCH_SIZE")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(32),
        concurrency: env::var("EMBEDDING_CONCURRENCY")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(10),
        progress_interval: env::var("EMBEDDING_PROGRESS_INTERVAL")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(20),
    }
}
