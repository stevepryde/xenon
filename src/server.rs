use crate::browser::W3CCapabilities;
use crate::config::{load_config, XenonConfig};
use crate::error::{XenonError, XenonResult};
use crate::response::XenonResponse;
use crate::session::{Session, XenonSessionId};
use crate::state::XenonState;
use hyper::service::{make_service_fn, service_fn};
use hyper::{Body, Request, Response, Server, StatusCode};
use log::*;
use std::convert::Infallible;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use structopt::StructOpt;
use tokio::sync::RwLock;
use tokio::time::{delay_for, Duration};

#[derive(Debug, StructOpt)]
#[structopt(name = "Xenon", about = "A powerful WebDriver proxy")]
pub struct Opt {
    /// The port to listen on. Default is 4444.
    #[structopt(short, long, env = "XENON_PORT")]
    port: Option<u16>,

    /// The path to the YAML config file. Default is xenon.yml.
    #[structopt(short, long, parse(from_os_str), env = "XENON_CFG")]
    cfg: Option<PathBuf>,
}

pub async fn start_server() -> XenonResult<()> {
    let opt = Opt::from_args();

    // Prefer CLI arg, otherwise environment variable, otherwise 4444.
    let port: u16 = opt.port.unwrap_or(4444);
    if port < 1024 {
        return Err(XenonError::InvalidPort);
    }

    let addr: SocketAddr = format!("127.0.0.1:{}", port)
        .parse()
        .map_err(|_| XenonError::InvalidPort)?;

    // Read config.
    let config_filename = opt.cfg.unwrap_or_else(|| PathBuf::from("xenon.yml"));
    let config = load_config(&config_filename).unwrap_or_else(|e| {
        warn!(
            "Warning: {} (Default config will be used instead)",
            e.to_string()
        );

        // Use default config.
        XenonConfig::new()
    });
    debug!("Config loaded:\n{:#?}", config);
    let state = Arc::new(RwLock::new(XenonState::new(config)));

    let (tx_terminator, rx_terminator) = tokio::sync::oneshot::channel();

    // Spawn session timeout task.
    let state_clone = state.clone();
    tokio::spawn(async move {
        process_session_timeout(state_clone, rx_terminator).await;
    });

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
    info!("Server running at {}", addr);
    let server = Server::bind(&addr).serve(make_service);

    // And run forever...
    let result = server
        .await
        .map_err(|e| XenonError::ServerError(e.to_string()));

    if let Err(e) = tx_terminator.send(true) {
        error!("Error terminating timeout task: {:?}", e);
    }

    result
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
        "session" => handle_session(req, state, false).await,
        "wd" => handle_session(req, state, true).await,
        p => Err(XenonError::RespondWith(XenonResponse::EndpointNotFound(
            p.to_string(),
        ))),
    };

    match result {
        Ok(x) => Ok(x),
        Err(XenonError::RespondWith(r)) => {
            debug!("Xenon replied with error: {:#?}", r);
            Ok(Response::builder()
                .status(r.status())
                .body(r.into())
                .unwrap_or_else(|_| {
                    Response::builder()
                        .status(StatusCode::INTERNAL_SERVER_ERROR)
                        .body(Body::from("Xenon failed to serialize an error"))
                        .unwrap()
                }))
        }
        Err(e) => {
            // Coerce all errors into WebDriver-compatible response.
            error!("Internal Error: {:#?}", e);
            let r = XenonResponse::InternalServerError(e.to_string());

            Ok(Response::builder()
                .status(r.status())
                .body(r.into())
                .unwrap_or_else(|_| {
                    Response::builder()
                        .status(StatusCode::INTERNAL_SERVER_ERROR)
                        .body(Body::from("Xenon failed to serialize an error"))
                        .unwrap()
                }))
        }
    }
}

async fn handle_session(
    req: Request<Body>,
    state: Arc<RwLock<XenonState>>,
    selenium_compatibility: bool,
) -> XenonResult<Response<Body>> {
    let mut path_elements: Vec<&str> = req.uri().path().trim_matches('/').split('/').collect();

    // We can mimic selenium by ignoring the path /wd/hub if it exists.
    if selenium_compatibility
        && path_elements.len() > 2
        && path_elements[0] == "wd"
        && path_elements[1] == "hub"
    {
        // Selenium endpoint. Just remove these from the path.
        path_elements = path_elements.split_off(2);
    }

    match path_elements.len() {
        0 => unreachable!(),
        1 => match *req.method() {
            hyper::Method::POST => handle_create_session(req, state).await,
            _ => Err(XenonError::RespondWith(XenonResponse::MethodNotFound(
                path_elements.join("/"),
            ))),
        },
        _ => {
            if path_elements[0] != "session" {
                warn!("Unknown endpoint: {:?}", path_elements);
            }

            let xsession_id = XenonSessionId::from(path_elements[1]);
            let is_delete = if path_elements.len() == 2 {
                path_elements[0] == "session" && req.method() == hyper::Method::DELETE
            } else {
                false
            };

            // Forward to session.
            let mutex_session = {
                let s = state.read().await;
                match s.get_session(&xsession_id) {
                    Some(x) => x,
                    None => {
                        return Err(XenonError::RespondWith(XenonResponse::SessionNotFound(
                            xsession_id.to_string(),
                        )))
                    }
                }
            };

            let remaining_path: String = path_elements[2..].join("/");
            info!(
                "Session {:?} :: {} {}",
                xsession_id,
                req.method(),
                remaining_path
            );
            let mut session = mutex_session.lock().await;
            let response = session.forward_request(req, &remaining_path).await?;

            if is_delete && response.status().is_success() {
                info!(
                    "Session Delete {:?} :: port {}",
                    xsession_id,
                    session.port()
                );
                // Remove the actual session under write-lock. This should be fast.
                {
                    let mut s = state.write().await;
                    s.delete_session(&xsession_id);
                }

                // Remove the session reference under read-lock on state and write-lock on
                // service group. The service may self-destruct if this was the last connection
                // to it.
                {
                    let s = state.read().await;
                    let rwlock_groups = s.service_groups();
                    let rwlock_port_manager = s.port_manager();
                    let (mut port_manager, mut groups) =
                        tokio::join!(rwlock_port_manager.write(), rwlock_groups.write());
                    if let Some(group) = groups.get_mut(&session.service_group().to_string()) {
                        group.delete_session(session.port(), &xsession_id, &mut port_manager);
                    }
                }
            }

            Ok(response)
        }
    }
}

pub async fn handle_create_session(
    req: Request<Body>,
    state: Arc<RwLock<XenonState>>,
) -> XenonResult<Response<Body>> {
    let body_bytes = hyper::body::to_bytes(req)
        .await
        .map_err(|e| XenonError::RespondWith(XenonResponse::ErrorCreatingSession(e.to_string())))?;

    // NOTE: Capabilities are ONLY used to match within Xenon. They are not sent to the
    //       target webdriver.
    let w3c_capabilities: W3CCapabilities = serde_json::from_slice(&body_bytes)
        .map_err(|e| XenonError::RespondWith(XenonResponse::ErrorCreatingSession(e.to_string())))?;
    info!("Request new session :: {:#?}", &w3c_capabilities);
    let capabilities = serde_json::from_value(w3c_capabilities.capabilities.clone())
        .map_err(|e| XenonError::RespondWith(XenonResponse::ErrorCreatingSession(e.to_string())))?;

    let (xsession_id, port, group_name) = {
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
        let service = group.get_or_start_service(&mut port_manager).await?;
        let xsession_id = XenonSessionId::new();
        service.add_session(xsession_id.clone());
        (xsession_id, service.port(), group_name)
    };

    // Create the session. No locks are held at all here.
    info!("Session Create {:?} :: port {}", xsession_id, port);
    match Session::create(
        port,
        &group_name,
        &w3c_capabilities.capabilities,
        &w3c_capabilities.desired_capabilities,
        xsession_id.clone(),
    )
    .await
    {
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
            let rwlock_port_manager = s.port_manager();
            let (mut port_manager, mut groups) =
                tokio::join!(rwlock_port_manager.write(), rwlock_groups.write());
            if let Some(group) = groups.get_mut(&group_name) {
                group.delete_session(port, &xsession_id, &mut port_manager);
            }
            Ok(response)
        }
        Err(e) => {
            // Delete session from service.
            let s = state.read().await;
            let rwlock_groups = s.service_groups();
            let rwlock_port_manager = s.port_manager();
            let (mut port_manager, mut groups) =
                tokio::join!(rwlock_port_manager.write(), rwlock_groups.write());
            if let Some(group) = groups.get_mut(&group_name) {
                group.delete_session(port, &xsession_id, &mut port_manager);
            }
            Err(e)
        }
    }
}

async fn process_session_timeout(
    state: Arc<RwLock<XenonState>>,
    mut rx: tokio::sync::oneshot::Receiver<bool>,
) {
    while let Err(_) = rx.try_recv() {
        let timedout_sessions = {
            let s = state.read().await;
            s.get_timeout_sessions().await
        };

        if !timedout_sessions.is_empty() {
            let mut s = state.write().await;
            let rwlock_groups = s.service_groups();
            let rwlock_port_manager = s.port_manager();
            let (mut port_manager, mut groups) =
                tokio::join!(rwlock_port_manager.write(), rwlock_groups.write());

            for xsession_id in timedout_sessions {
                if let Some(mutex_session) = s.delete_session(&xsession_id) {
                    let session = mutex_session.lock().await;

                    info!(
                        "Session Timeout {:?} :: port {}",
                        xsession_id,
                        session.port()
                    );
                    if let Some(group) = groups.get_mut(session.service_group()) {
                        group.delete_session(session.port(), &xsession_id, &mut port_manager);
                    }
                }
            }
        }
        delay_for(Duration::new(60, 0)).await;
    }
}
