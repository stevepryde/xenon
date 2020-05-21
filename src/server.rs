use crate::browser::Capabilities;
use crate::config::{load_config, XenonConfig};
use crate::error::{XenonError, XenonResult};
use crate::response::XenonResponse;
use crate::session::{Session, XenonSessionId};
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
    debug!("Config loaded:\n{:#?}", config);
    let state = Arc::new(RwLock::new(XenonState::new(config)));

    // TODO: Start timer for performing cleanup / session timeout.

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
        1 => match *req.method() {
            hyper::Method::POST => handle_create_session(req, state).await,
            _ => Err(XenonError::RespondWith(XenonResponse::MethodNotFound(
                path_elements.join("/"),
            ))),
        },
        _ => {
            let session_id = XenonSessionId::from(path_elements[1]);
            let remaining_path: String = path_elements[2..].join("/");

            // Forward to session.
            let mutex_session = {
                let s = state.read().await;
                match s.get_session(&session_id) {
                    Some(x) => x,
                    None => {
                        return Err(XenonError::RespondWith(XenonResponse::SessionNotFound(
                            session_id.to_string(),
                        )))
                    }
                }
            };

            let session = mutex_session.lock().await;
            let response = session.forward_request(req, &remaining_path).await?;
            // TODO: handle session deletion here.

            Ok(response)
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

    let (xsession_id, port) = {
        let s = state.read().await;
        let rwlock_groups = s.service_groups();

        // We can do the capability matching under a read lock.
        let group_name = {
            let groups = rwlock_groups.read().await;

            let group = match groups
                .values()
                .find(|v| v.matches_capabilities(&capabilities))
            {
                Some(x) => x,
                None => {
                    return Err(XenonError::RespondWith(XenonResponse::NoMatchingBrowser));
                }
            };

            // Found a service group. Do we have capacity for a new session?
            // Note that this is a preliminary check only and is not a guarantee.
            // We use this to return early if there are no sessions available,
            // but even if this suggests we have capacity we still need to
            // check again while holding a write lock on service_groups.
            if !group.has_capacity() {
                return Err(XenonError::RespondWith(XenonResponse::NoSessionsAvailable));
            }

            group.name().to_string()
        };

        // Now we get a write lock to add the new session/service.
        let rwlock_port_manager = s.port_manager();
        let (mut port_manager, mut groups) =
            tokio::join!(rwlock_port_manager.write(), rwlock_groups.write());

        let group = match groups.get_mut(&group_name) {
            Some(g) => g,
            None => {
                return Err(XenonError::RespondWith(XenonResponse::NoMatchingBrowser));
            }
        };

        // This only holds a write lock on the port manager and service groups,
        // so it only blocks the creation or deletion of other services or sessions.
        // This will not block any in-progress sessions.
        let service = group.get_or_start_service(&mut port_manager)?;
        let xsession_id = XenonSessionId::new();
        service.add_session(xsession_id.clone());
        (xsession_id, service.port())
    };

    // Create the session. No locks are held at all here.
    match Session::create(port, &capabilities, xsession_id.clone()).await {
        Ok((session, response)) => {
            // Add session to pool.
            let mut s = state.write().await;
            s.add_session(xsession_id, session);
            // Forward the response back to the client.
            Ok(response)
        }
        Err(XenonError::ResponsePassThrough(response)) => {
            // Delete session from service.
            let s = state.read().await;
            let rwlock_groups = s.service_groups();
            // TODO: need to store the group name/port with the session.
            Ok(response)
        }
        Err(e) => {
            // Delete session from service.

            Err(e)
        }
    }
}
