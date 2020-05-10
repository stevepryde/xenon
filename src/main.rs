use crate::config::XenonConfig;
use crate::error::XenonError;
use crate::session::{Capabilities, Session, XenonSessionId};
use bytes::buf::ext::BufExt;
use hyper::service::{make_service_fn, service_fn};
use hyper::{Body, Request, Response, Server, StatusCode};
use serde::Serialize;
use std::collections::HashMap;
use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::RwLock;

mod config;
mod error;
mod session;

#[tokio::main]
async fn main() {
    pretty_env_logger::init();
    let port: i32 = std::option_env!("XENON_PORT")
        .unwrap_or("4444")
        .parse()
        .expect("Invalid port");
    let addr: SocketAddr = format!("127.0.0.1:{}", port)
        .parse()
        .expect("Invalid server address");

    // TODO: read from config file.
    let config = XenonConfig::new();

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
    if let Err(e) = server.await {
        eprintln!("server error: {}", e);
    }
}

#[derive(Debug)]
pub struct XenonState {
    sessions: HashMap<XenonSessionId, Session>,
    config: XenonConfig,
}

impl XenonState {
    pub fn new(config: XenonConfig) -> Self {
        Self {
            sessions: HashMap::new(),
            config,
        }
    }

    pub fn get_session(&self, session_id: &XenonSessionId) -> Option<&Session> {
        self.sessions.get(session_id)
    }

    pub fn delete_session(&mut self, session_id: &XenonSessionId) -> bool {
        self.sessions.remove(session_id).is_some()
    }
}

#[derive(Debug, Serialize)]
pub struct XenonResponse {
    pub code: String,
    pub message: String,
}

impl XenonResponse {
    pub fn new(code: &str, message: &str) -> Self {
        Self {
            code: code.to_string(),
            message: message.to_string(),
        }
    }
}

impl Into<Body> for XenonResponse {
    fn into(self) -> Body {
        Body::from(
            serde_json::to_string(&self)
                .unwrap_or_else(|e| format!("JSON conversion failed: {}", e)),
        )
    }
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
    match top_level_path {
        x if x.is_empty() => Ok(Response::new(Body::from("TODO: show status page"))),
        "session" => handle_session(req, state).await,
        _ => Ok(Response::builder()
            .status(StatusCode::NOT_FOUND)
            .body(
                XenonResponse::new("PATH_NOT_FOUND", "Please specify a valid session path").into(),
            )
            .unwrap()),
    }
}

async fn handle_session(
    req: Request<Body>,
    state: Arc<RwLock<XenonState>>,
) -> Result<Response<Body>, Infallible> {
    let path_elements: Vec<&str> = req.uri().path().trim_matches('/').split('/').collect();

    match path_elements.len() {
        0 => unreachable!(),
        1 => match req.method() {
            &hyper::Method::POST => match handle_create_session(req, state).await {
                Ok(x) => Ok(x),
                Err(e) => Ok(Response::builder()
                    .status(StatusCode::BAD_REQUEST)
                    .body(
                        XenonResponse::new("SESSION_NOT_STARTED", format!("{:?}", e).as_str())
                            .into(),
                    )
                    .unwrap()),
            },
            e => Ok(Response::builder()
                .status(StatusCode::NOT_FOUND)
                .body(
                    XenonResponse::new(
                        "METHOD_NOT_FOUND",
                        format!("Unknown method for /session: {}", e.to_string()).as_str(),
                    )
                    .into(),
                )
                .unwrap()),
        },
        _ => {
            let session_id = XenonSessionId::from(path_elements[1]);
            let remaining_path: String = path_elements[2..].join("/");
            // Forward to session.
            match state.read().await.get_session(&session_id) {
                Some(session) => match session.handle_request(req, &remaining_path).await {
                    Ok(x) => Ok(x),
                    Err(e) => Ok(Response::builder()
                        .status(StatusCode::BAD_REQUEST)
                        .body(
                            XenonResponse::new(
                                "REQUEST_FAILED",
                                format!("Request to target WebDriver failed: {:?}", e).as_str(),
                            )
                            .into(),
                        )
                        .unwrap()),
                },
                None => Ok(Response::builder()
                    .status(StatusCode::NOT_FOUND)
                    .body(
                        XenonResponse::new(
                            "SESSION_NOT_FOUND",
                            format!("Session '{:?}' not found", session_id).as_str(),
                        )
                        .into(),
                    )
                    .unwrap()),
            }
        }
    }
}

pub async fn handle_create_session(
    req: Request<Body>,
    state: Arc<RwLock<XenonState>>,
) -> Result<Response<Body>, XenonError> {
    let whole_body = hyper::body::aggregate(req)
        .await
        .map_err(|e| XenonError::NewSessionError(e.to_string()))?;
    let capabilities: Capabilities = serde_json::from_reader(whole_body.reader())
        .map_err(|e| XenonError::NewSessionError(e.to_string()))?;
    // TODO: use read lock here and then use write lock later once session created.
    let s = state.write().await;
    match s.config.match_capabilities(&capabilities) {
        Some(browser) => {
            // TODO: Create session.
            Ok(Response::new(Body::from("TEST")))
        }
        None => Err(XenonError::NewSessionError(
            "No browser matching capabilities".to_string(),
        )),
    }
}

pub async fn handle_delete_session(
    req: Request<Body>,
    state: Arc<RwLock<XenonState>>,
    session_id: &XenonSessionId,
) -> Result<Response<Body>, XenonError> {
    let res = {
        let s = state.read().await;
        let session = match s.get_session(&session_id) {
            Some(x) => x,
            None => return Err(XenonError::SessionNotFound),
        };

        match session.handle_request(req, "").await {
            Ok(x) => Ok(x),
            Err(e) => {
                return Ok(Response::builder()
                    .status(StatusCode::BAD_REQUEST)
                    .body(
                        XenonResponse::new(
                            "REQUEST_FAILED",
                            format!("Request to target WebDriver failed: {}", e).as_str(),
                        )
                        .into(),
                    )
                    .unwrap())
            }
        }
    };
    if !state.write().await.delete_session(session_id) {
        Err(XenonError::SessionNotFound)
    } else {
        res
    }
}
