use crate::config::{load_config, XenonConfig};
use crate::error::{XenonError, XenonResult};
use crate::response::XenonResponse;
use crate::session::{Capabilities, XenonSessionId};
use crate::state::XenonState;
use bytes::buf::ext::BufExt;
use hyper::service::{make_service_fn, service_fn};
use hyper::{Body, Request, Response, Server, StatusCode};
use log::*;
use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::RwLock;

pub async fn start_server() -> XenonResult<()> {
    let port: i32 = std::option_env!("XENON_PORT")
        .unwrap_or("4444")
        .parse()
        .map_err(|_| XenonError::InvalidPort)?;
    let addr: SocketAddr = format!("127.0.0.1:{}", port)
        .parse()
        .map_err(|_| XenonError::InvalidPort)?;

    // Read config.
    let config_filename = std::option_env!("XENON_CFG").unwrap_or("xenon.yml");
    let config = load_config(config_filename).unwrap_or_else(|e| {
        warn!("Warning: {}", e.to_string());

        // Use default config.
        XenonConfig::new()
    });
    let state = Arc::new(RwLock::new(XenonState::new(config)));

    // And a MakeService to handle each connection...
    let make_service = make_service_fn(move |_conn| {
        // Clone state.
        let state = state.clone();
        async move {
            let state = state.clone();
            Ok::<_, Infallible>(service_fn(move |req| {
                let state = state.clone();
                handle(req, state)
            }))
        }
    });

    // Then bind and serve...
    let server = Server::bind(&addr).serve(make_service);

    // And run forever...
    server
        .await
        .map_err(|e| XenonError::ServerError(e.to_string()))
}

async fn handle(
    req: Request<Body>,
    state: Arc<RwLock<XenonState>>,
) -> Result<Response<Body>, Infallible> {
    let top_level_path: &str = req
        .uri()
        .path()
        .trim_matches('/')
        .split('/')
        .next()
        .unwrap_or_else(|| "");

    // Routing for top-level path.
    let result = match top_level_path {
        x if x.is_empty() => Ok(Response::new(Body::from("TODO: show status page"))),
        "session" => handle_session(req, state).await,
        p => Err(XenonError::RespondWith(XenonResponse::PathNotFound(
            p.to_string(),
        ))),
    };

    match result {
        Ok(x) => Ok(x),
        Err(XenonError::RespondWith(r)) => Ok(Response::builder()
            .status(r.status())
            .body(r.into())
            .unwrap_or_else(|_| {
                Response::builder()
                    .status(StatusCode::INTERNAL_SERVER_ERROR)
                    .body(Body::from("Xenon failed to serialize an error"))
                    .unwrap()
            })),
        Err(e) => Ok(Response::builder()
            .status(StatusCode::INTERNAL_SERVER_ERROR)
            .body(XenonResponse::InternalServerError(e.to_string()).into())
            .unwrap_or_else(|_| {
                Response::builder()
                    .status(StatusCode::INTERNAL_SERVER_ERROR)
                    .body(Body::from("Xenon experienced an internal error"))
                    .unwrap()
            })),
    }
}

async fn handle_session(
    req: Request<Body>,
    state: Arc<RwLock<XenonState>>,
) -> XenonResult<Response<Body>> {
    let path_elements: Vec<&str> = req.uri().path().trim_matches('/').split('/').collect();

    match path_elements.len() {
        0 => unreachable!(),
        1 => match req.method() {
            &hyper::Method::POST => handle_create_session(req, state).await,
            e => Err(XenonError::RespondWith(XenonResponse::MethodNotFound(
                path_elements.join("/"),
            ))),
        },
        _ => {
            let session_id = XenonSessionId::from(path_elements[1]);
            let remaining_path: String = path_elements[2..].join("/");

            // Forward to session.
            match state.read().await.get_session(&session_id) {
                Some(session) => session.handle_request(req, &remaining_path).await,
                None => Err(XenonError::RespondWith(XenonResponse::SessionNotFound(
                    session_id.to_string(),
                ))),
            }
        }
    }
}

pub async fn handle_create_session(
    req: Request<Body>,
    state: Arc<RwLock<XenonState>>,
) -> XenonResult<Response<Body>> {
    let whole_body = hyper::body::aggregate(req)
        .await
        .map_err(|e| XenonError::RespondWith(XenonResponse::ErrorCreatingSession(e.to_string())))?;
    let capabilities: Capabilities = serde_json::from_reader(whole_body.reader())
        .map_err(|e| XenonError::RespondWith(XenonResponse::ErrorCreatingSession(e.to_string())))?;

    // TODO: use read lock here and then use write lock later once session created.
    let s = state.write().await;
    match s.config.match_capabilities(&capabilities) {
        Some(browser) => {
            // TODO: Create session.
            Ok(Response::new(Body::from("TEST")))
        }
        None => Err(XenonError::RespondWith(
            XenonResponse::ErrorCreatingSession("No browser matching capabilities".to_string()),
        )),
    }
}

pub async fn handle_delete_session(
    req: Request<Body>,
    state: Arc<RwLock<XenonState>>,
    session_id: &XenonSessionId,
) -> XenonResult<Response<Body>> {
    let res = {
        let s = state.read().await;
        let session = match s.get_session(&session_id) {
            Some(x) => x,
            None => {
                return Err(XenonError::RespondWith(XenonResponse::SessionNotFound(
                    session_id.to_string(),
                )))
            }
        };

        session.handle_request(req, "").await?
    };
    if !state.write().await.delete_session(session_id) {
        Err(XenonError::RespondWith(XenonResponse::SessionNotFound(
            session_id.to_string(),
        )))
    } else {
        Ok(res)
    }
}
