// use warp::Filter;
use hyper::service::{make_service_fn, service_fn};
use std::convert::Infallible;
use std::net::SocketAddr;
mod session;
mod error;
use hyper::{Body, Request, Response, Server};

#[tokio::main]
async fn main() {
    pretty_env_logger::init();

    // TODO: use port when https://github.com/seanmonstar/warp/issues/570 is resolved.
    let port: i32 = std::option_env!("XENON_PORT").unwrap_or("4444").parse().expect("Invalid port");
    let addr: SocketAddr = format!("127.0.0.1:{}", port).parse().expect("Invalid server address");

    // And a MakeService to handle each connection...
    let make_service = make_service_fn(|_conn| async {
        Ok::<_, Infallible>(service_fn(handle))
    });

    // Then bind and serve...
    let server = Server::bind(&addr).serve(make_service);

    // And run forever...
    if let Err(e) = server.await {
        eprintln!("server error: {}", e);
    }
}

async fn handle(_req: Request<Body>) -> Result<Response<Body>, Infallible> {
    Ok(Response::new(Body::from("Hello World")))
}
