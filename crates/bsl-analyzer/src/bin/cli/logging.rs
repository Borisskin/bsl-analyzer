use std::{env, fs, io, path::PathBuf, sync::Arc};

mod journal;
pub use journal::JournalGuard;

pub fn activate_vector_journal(source: Option<&std::path::Path>) {
    journal::activate(source);
}

pub fn setup_logging(
    log_file: Option<PathBuf>,
    append_file: bool,
    profile_filter: Option<String>,
    json_profile_filter: Option<String>,
) -> anyhow::Result<JournalGuard> {
    use tracing_subscriber::fmt::writer::BoxMakeWriter;

    // `append_file` is set for the daemon's default log: concurrent short-lived
    // daemon candidates (the broker's spawn race) must not truncate the winner's
    // live file. An explicit `BSL_LOG_FILE` keeps the historical truncate-on-start
    // behaviour.
    let writer: BoxMakeWriter = match log_file {
        Some(file) if append_file => BoxMakeWriter::new(Arc::new(
            fs::OpenOptions::new().create(true).append(true).open(&file)?,
        )),
        Some(file) => BoxMakeWriter::new(Arc::new(fs::File::create(&file)?)),
        None => BoxMakeWriter::new(io::stderr),
    };

    let user_filter = env::var("BSL_LOG").ok().unwrap_or_else(|| "warn".to_owned());
    // The graph-build heartbeat (target `bsl_graph`) stays visible at the default
    // `warn` level: it is the only trail that localises a wedged cold build. The
    // directive precedes the user filter, so an explicit `bsl_graph=off` wins.
    let filter = format!("bsl_graph=info,{},salsa=warn", user_filter);

    use tracing_subscriber::Layer;
    bsl_search::lifecycle::set_process_id(uuid::Uuid::new_v4().to_string());
    let (layer, guard) = journal::layer();
    bsl_analyzer::tracing::Config { writer, filter, profile_filter, json_profile_filter }
        .init_with_layer(layer.with_filter(journal::filter(&user_filter)))?;

    Ok(guard)
}
