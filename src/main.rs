use crate::server::start_server;
use tracing_subscriber::{EnvFilter, fmt};

mod browser;
mod config;
mod error;
mod nodes;
mod portmanager;
mod response;
mod server;
mod service;
mod session;
mod state;

#[tokio::main]
async fn main() {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("xenon=debug"));
    fmt().with_env_filter(filter).init();

    if let Err(e) = start_server().await {
        eprintln!("Xenon server stopped.\nERROR: {e:?}");
        std::process::exit(1);
    }
}
