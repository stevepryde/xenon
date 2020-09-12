use crate::server::start_server;
use env_logger::Env;

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
    env_logger::from_env(Env::default().default_filter_or("xenon=debug")).init();

    if let Err(e) = start_server().await {
        println!("Xenon server stopped.\nERROR: {:?}", e);
        std::process::exit(1);
    }
}
