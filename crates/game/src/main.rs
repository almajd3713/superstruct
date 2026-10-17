use engine::app;
use engine::settings::Settings;
use tracing_subscriber::EnvFilter;

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_env("SUPERSTRUCT_LOG")
                .unwrap_or_else(|_| EnvFilter::new("info,wgpu_hal=off,wgpu=warn,naga=warn")),
        )
        .init();
    #[cfg(feature = "tracy")]
    let _tracy = tracy_client::Client::start();
    let settings = Settings::load_or_create(&app::settings_path())?;
    app::run("Superstruct", settings)?;
    Ok(())
}
