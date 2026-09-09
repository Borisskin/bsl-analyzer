use anyhow::Context;
use tracing_subscriber::{
    filter::Targets,
    fmt::{time::ChronoLocal, MakeWriter},
    layer::SubscriberExt,
    util::SubscriberInitExt,
    Layer, Registry,
};

use crate::tracing::{hprof, json};

#[derive(Debug)]
pub struct Config<T> {
    pub writer: T,
    pub filter: String,
    pub profile_filter: Option<String>,
    pub json_profile_filter: Option<String>,
}

impl<T> Config<T>
where
    T: for<'writer> MakeWriter<'writer> + Send + Sync + 'static,
{
    pub fn init(self) -> anyhow::Result<()> {
        self.init_with_layer(tracing_subscriber::layer::Identity::new())
    }

    pub fn init_with_layer<L>(self, layer: L) -> anyhow::Result<()>
    where
        L: Layer<Registry> + Send + Sync + 'static,
    {
        let targets_filter: Targets = self
            .filter
            .parse()
            .with_context(|| format!("invalid log filter: `{}`", self.filter))?;

        let writer = self.writer;

        let fmt_layer = tracing_subscriber::fmt::layer()
            .with_timer(ChronoLocal::rfc_3339())
            .with_target(false)
            .with_ansi(false)
            .with_writer(writer)
            .with_filter(targets_filter);

        let hprof_layer = self.profile_filter.as_ref().map(|spec| hprof::SpanTree::new(spec));

        let json_layer = self
            .json_profile_filter
            .as_ref()
            .map(|spec| json::TimingLayer::new(spec, std::io::stderr));

        Registry::default().with(layer).with(fmt_layer).with(hprof_layer).with(json_layer).init();

        Ok(())
    }
}
