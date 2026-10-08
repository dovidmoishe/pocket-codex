use clap::Parser;
use pocket_codex::{
    auth::{Auth, load_key},
    config::Config,
    jobs::Queue,
    store::Store,
    web::{App, router},
};
use std::{sync::Arc, time::Duration};

#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "pocket_codex=info".into()),
        )
        .init();
    let mut config = Config::parse();
    config.validate()?;
    if config.show_key {
        // This command can run while the server holds the data-directory lock.
        std::fs::create_dir_all(&config.data)?;
        println!("{}", load_key(&config.data)?);
        return Ok(());
    }
    let store = Arc::new(Store::open(&config.data)?);
    let auth = Arc::new(Auth::new(&load_key(&store.root)?));
    // Bind before starting the worker: a port conflict must not leave a Codex child alive.
    let listener = tokio::net::TcpListener::bind(config.bind).await?;
    let (queue, worker) = Queue::start(store.clone(), config.clone());
    let app = router(App {
        store,
        auth,
        queue: queue.clone(),
        config: config.clone(),
    });
    tracing::info!("Pocket Codex listening at http://{}", config.bind);
    tracing::info!(
        "Run pocket-codex --show-key (with the same --data directory) to see your login key"
    );
    if config.mock {
        tracing::info!("Mock mode: no Codex requests or AI-generated artwork");
    }
    if config.public_origin.is_none() {
        tracing::info!(
            "Local mode. Set --public-origin to your HTTPS ngrok origin before remote use"
        );
    }
    let stop = queue.shutdown.clone();
    let server = axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            let _ = tokio::signal::ctrl_c().await;
            stop.cancel();
        })
        .await;
    queue.shutdown.cancel();
    let _ = tokio::time::timeout(Duration::from_secs(8), worker).await;
    server?;
    Ok(())
}
