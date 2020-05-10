use crate::server::start_server;

mod config;
mod error;
mod response;
mod server;
mod session;
mod state;

#[tokio::main]
async fn main() {
    pretty_env_logger::init();

    if let Err(e) = start_server().await {
        println!("Xenon server stopped.\nERROR: {}", e);
        std::process::exit(1);
    }
}
