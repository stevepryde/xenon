use std::convert::Infallible;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use hyper::http::uri::{Authority, Scheme};
use hyper::server::conn::AddrStream;
use hyper::service::{make_service_fn, service_fn};
use hyper::{Body, Client, Request, Response, Server, StatusCode};
use log::*;

use structopt::StructOpt;
use tokio::sync::RwLock;
use tokio::time::{delay_for, Duration};

use crate::browser::{Capabilities, W3CCapabilities};
use crate::config::load_config;
use crate::error::{XenonError, XenonResult};
use crate::nodes::{NodeId, RemoteNode, RemoteServiceGroup};
use crate::response::XenonResponse;
use crate::service::ServiceGroup;
use crate::session::{Session, XenonSessionId};
use crate::state::XenonState;
use indexmap::map::IndexMap;

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
    let config = load_config(&config_filename)?;
    debug!("Config loaded:\n{:#?}", config);
    let using_nodes = config.has_nodes();
    let state = Arc::new(RwLock::new(XenonState::new(config)?));

    let (tx_terminator, rx_terminator) = tokio::sync::oneshot::channel();

    // Spawn session timeout task.
    let state_clone = state.clone();
    tokio::spawn(async move {
        process_session_timeout(state_clone, rx_terminator).await;
    });
    if using_nodes {
        // Spawn config getter.
        let state_clone = state.clone();
        tokio::spawn(async move {
            process_node_init(state_clone).await;
        });
    }

    // And a MakeService to handle each connection...
    let make_service = make_service_fn(move |conn: &AddrStream| {
        // Clone state.
        let state = state.clone();
        let remote_addr = conn.remote_addr();
        async move {
            let state = state.clone();
            let remote_addr = remote_addr.clone();
            Ok::<_, Infallible>(service_fn(move |req| {
                let state = state.clone();
                handle(req, remote_addr.clone(), state)
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
    remote_addr: SocketAddr,
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
        "node" => handle_node(req, remote_addr, state).await,
        "status" => Ok(Response::builder().status(200).body("OK".into()).unwrap()),
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
            hyper::Method::POST => {
                // Create session.
                let body_bytes = hyper::body::to_bytes(req).await.map_err(|e| {
                    XenonError::RespondWith(XenonResponse::ErrorCreatingSession(e.to_string()))
                })?;

                let w3c_capabilities: W3CCapabilities = serde_json::from_slice(&body_bytes)
                    .map_err(|e| {
                        XenonError::RespondWith(XenonResponse::ErrorCreatingSession(e.to_string()))
                    })?;
                info!("Request new session :: {:#?}", &w3c_capabilities);
                let capabilities: Capabilities =
                    serde_json::from_value(w3c_capabilities.capabilities.clone()).map_err(|e| {
                        XenonError::RespondWith(XenonResponse::ErrorCreatingSession(e.to_string()))
                    })?;

                match handle_create_session(&capabilities, &w3c_capabilities, state.clone()).await {
                    Ok(x) => Ok(x),
                    Err(XenonError::RespondWith(XenonResponse::NoSessionsAvailable)) => {
                        // In this case there is at least 1 matching browser locally, so even if
                        // the node search returns no matching browser, the no matching sessions
                        // error takes precedence.
                        match handle_create_session_node(
                            &capabilities,
                            &w3c_capabilities,
                            state.clone(),
                        )
                        .await
                        {
                            Ok(x) => Ok(x),
                            Err(XenonError::RespondWith(XenonResponse::NoMatchingBrowser)) => {
                                Err(XenonError::RespondWith(XenonResponse::NoSessionsAvailable))
                            }
                            Err(e) => Err(e),
                        }
                    }
                    Err(XenonError::RespondWith(XenonResponse::NoMatchingBrowser)) => {
                        handle_create_session_node(&capabilities, &w3c_capabilities, state.clone())
                            .await
                    }
                    Err(e) => Err(e),
                }
            }
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

                // For local sessions, remove the session from its service group.
                if let Some(session_group) = session.service_group() {
                    // Remove the session reference under read-lock on state and write-lock on
                    // service group. The service may self-destruct if this was the last connection
                    // to it.
                    let s = state.read().await;
                    let rwlock_groups = s.service_groups();
                    let rwlock_port_manager = s.port_manager();
                    let (mut port_manager, mut groups) =
                        tokio::join!(rwlock_port_manager.write(), rwlock_groups.write());

                    if let Some(group) = groups.get_mut(session_group) {
                        group.delete_session(session.port(), &xsession_id, &mut port_manager);
                    }
                }
            }

            Ok(response)
        }
    }
}

pub async fn handle_create_session(
    capabilities: &Capabilities,
    w3c_capabilities: &W3CCapabilities,
    state: Arc<RwLock<XenonState>>,
) -> XenonResult<Response<Body>> {
    let (xsession_id, port, group_name) =
        reserve_available_session(state.clone(), capabilities).await?;

    // Create the session. No locks are held at all here.
    info!("Session Create {:?} :: port {}", xsession_id, port);
    let authority: Authority = match format!("localhost:{}", port).parse() {
        Ok(a) => a,
        Err(e) => {
            return Err(XenonError::RespondWith(
                XenonResponse::ErrorCreatingSession(format!("Invalid port '{}': {}", port, e)),
            ));
        }
    };
    match Session::create(
        Scheme::HTTP,
        authority,
        Some(group_name.clone()),
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

pub async fn reserve_available_session(
    state: Arc<RwLock<XenonState>>,
    capabilities: &Capabilities,
) -> XenonResult<(XenonSessionId, u16, String)> {
    let s = state.read().await;
    let rwlock_groups = s.service_groups();

    // We can do the capability matching under a read lock.
    let group_names = {
        let groups = rwlock_groups.read().await;

        let matching_groups: Vec<&ServiceGroup> = groups
            .values()
            .filter(|v| v.matches_capabilities(capabilities))
            .collect();
        if matching_groups.is_empty() {
            return Err(XenonError::RespondWith(XenonResponse::NoMatchingBrowser));
        }

        let matching_group_names: Vec<String> = matching_groups
            .iter()
            .filter(|v| v.has_capacity())
            .map(|v| v.name().to_string())
            .collect();
        if matching_group_names.is_empty() {
            return Err(XenonError::RespondWith(XenonResponse::NoSessionsAvailable));
        }

        matching_group_names
    };

    // Now we get a write lock to add the new session/service.
    // This only holds a write lock on the port manager and service groups,
    // so it only blocks the creation or deletion of other services or sessions.
    // This will not block any in-progress sessions.
    let rwlock_port_manager = s.port_manager();
    let (mut port_manager, mut groups) =
        tokio::join!(rwlock_port_manager.write(), rwlock_groups.write());

    // Note that a new session request might match several groups.
    // If any session fails to start, fallback to the next available group.
    let mut first_error: Option<XenonError> = None;
    for group_name in group_names {
        let group = groups.get_mut(&group_name).unwrap();

        match group.get_or_start_service(&mut port_manager).await {
            Ok(service) => {
                let xsession_id = XenonSessionId::new();
                service.add_session(xsession_id.clone());
                return Ok((xsession_id, service.port(), group_name));
            }
            Err(e) => {
                if first_error.is_none() {
                    first_error = Some(e);
                }
            }
        }
    }

    Err(first_error.unwrap_or(XenonError::RespondWith(XenonResponse::NoSessionsAvailable)))
}

pub async fn handle_create_session_node(
    capabilities: &Capabilities,
    w3c_capabilities: &W3CCapabilities,
    state: Arc<RwLock<XenonState>>,
) -> XenonResult<Response<Body>> {
    // Note we need to get the node data under read lock but we need to give that up
    // asap because we need a write lock later once a session is created.
    let (node_data, matched_caps) = {
        let s = state.read().await;
        let rwlock_nodes = s.remote_nodes();
        let nodes = rwlock_nodes.read().await;
        let mut node_data = Vec::new();
        let mut matched_caps = false;
        for node in nodes.values() {
            for group in &node.service_groups {
                if group.browser.matches_capabilities(capabilities) {
                    matched_caps = true;
                    if group.remaining_sessions > 0 {
                        node_data.push((
                            node.display_name(),
                            node.scheme.clone(),
                            node.authority.clone(),
                        ));
                    }
                }
            }
        }
        (node_data, matched_caps)
    };

    let xsession_id = XenonSessionId::new();
    for (name, scheme, authority) in node_data {
        info!(
            "Attempt Session Create {:?} :: Node '{}'",
            xsession_id, name
        );
        if let Ok((session, response)) = Session::create(
            scheme,
            authority,
            None,
            &w3c_capabilities.capabilities,
            &w3c_capabilities.desired_capabilities,
            xsession_id.clone(),
        )
        .await
        {
            // Add session to pool. Write lock here.
            let mut s = state.write().await;
            s.add_session(xsession_id, session);
            // Forward the response back to the client.
            return Ok(response);
        }
    }

    if matched_caps {
        Err(XenonError::RespondWith(XenonResponse::NoSessionsAvailable))
    } else {
        Err(XenonError::RespondWith(XenonResponse::NoMatchingBrowser))
    }
}

async fn process_session_timeout(
    state: Arc<RwLock<XenonState>>,
    mut rx: tokio::sync::oneshot::Receiver<bool>,
) {
    while rx.try_recv().is_err() {
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
                    if let Some(session_group) = session.service_group() {
                        if let Some(group) = groups.get_mut(session_group) {
                            group.delete_session(session.port(), &xsession_id, &mut port_manager);
                        }
                    }
                }
            }
        }
        delay_for(Duration::new(60, 0)).await;
    }
}

/// Handle requests to /node endpoints.
async fn handle_node(
    req: Request<Body>,
    _remote_addr: SocketAddr,
    state: Arc<RwLock<XenonState>>,
) -> XenonResult<Response<Body>> {
    let path_elements: Vec<String> = req
        .uri()
        .path()
        .trim_matches('/')
        .split('/')
        .map(|x| x.to_string())
        .collect();

    if path_elements.len() < 2 {
        return Err(XenonError::RespondWith(XenonResponse::EndpointNotFound(
            path_elements.join("/"),
        )));
    }

    match path_elements[1].as_str() {
        "config" => match *req.method() {
            hyper::Method::GET => {
                // GET /node/config
                let s = state.read().await;
                let rwlock_groups = s.service_groups();

                let mut groups_out = Vec::new();
                for group in rwlock_groups.read().await.values() {
                    let remote_group = RemoteServiceGroup {
                        browser: group.browser.clone(),
                        remaining_sessions: group.browser.max_sessions(),
                    };
                    groups_out.push(remote_group);
                }

                let body = Body::from(
                    serde_json::to_string(&groups_out)
                        .unwrap_or_else(|e| format!("Xenon failed to serialize node id: {}", e)),
                );

                Ok(Response::builder()
                    .status(StatusCode::OK)
                    .body(body)
                    .unwrap_or_else(|_| {
                        Response::builder()
                            .status(StatusCode::INTERNAL_SERVER_ERROR)
                            .body(Body::from("Xenon failed to serialize node configuration"))
                            .unwrap()
                    }))
            }
            _ => Err(XenonError::RespondWith(XenonResponse::EndpointNotFound(
                path_elements.join("/"),
            ))),
        },
        _p => Err(XenonError::RespondWith(XenonResponse::EndpointNotFound(
            path_elements.join("/"),
        ))),
    }
}

/// Fetch config for each node.
async fn process_node_init(state: Arc<RwLock<XenonState>>) {
    debug!("Downstream node configuration starting");
    let mut nodes_remaining: IndexMap<NodeId, RemoteNode> = {
        let s = state.read().await;
        let rwlock_nodes = s.remote_nodes();
        let nodes = rwlock_nodes.read().await.clone();
        nodes
    };

    let client = Client::new();

    while !nodes_remaining.is_empty() {
        let mut nodes_done = Vec::new();
        for node in nodes_remaining.values() {
            debug!(
                "Fetching config from downstream node '{}'...",
                node.display_name()
            );
            let uri_out = match hyper::Uri::builder()
                .scheme(node.scheme.clone())
                .authority(node.authority.clone())
                .path_and_query("/node/config")
                .build()
            {
                Ok(uri) => uri,
                Err(e) => {
                    error!(
                        "Invalid URI '{}' for node '{}': {}",
                        node.url,
                        node.display_name(),
                        e
                    );
                    continue;
                }
            };

            match client.get(uri_out).await {
                Ok(res) => match hyper::body::to_bytes(res).await {
                    Ok(bytes) => {
                        let remote_groups: Vec<RemoteServiceGroup> =
                            match serde_json::from_slice(&bytes) {
                                Ok(x) => x,
                                Err(e) => {
                                    error!(
                                        "Failed to parse configuration from node '{}': {}",
                                        node.display_name(),
                                        e
                                    );
                                    continue;
                                }
                            };

                        // Update these. This requires a write lock but only briefly.
                        let s = state.write().await;
                        let rwlock_nodes = s.remote_nodes();
                        let mut nodes = rwlock_nodes.write().await;
                        if let Some(node) = nodes.get_mut(&node.id()) {
                            node.service_groups = remote_groups.clone();
                        }
                        info!(
                            "Configuration for downstream node '{}' fetched successfully",
                            node.display_name()
                        );
                        info!("{:#?}", remote_groups);
                        nodes_done.push(node.id());
                    }
                    Err(e) => {
                        error!(
                            "Failed to receive configuration for node '{}': {}",
                            node.display_name(),
                            e
                        );
                        continue;
                    }
                },
                Err(e) => {
                    warn!(
                        "Unable to fetch configuration for node '{}': {}",
                        node.display_name(),
                        e
                    );
                    continue;
                }
            }
        }

        // Remove the ones we found.
        for node_id in nodes_done {
            nodes_remaining.remove(&node_id);
        }

        if !nodes_remaining.is_empty() {
            // Wait 60 seconds before trying again.
            delay_for(Duration::new(60, 0)).await;
        }
    }

    debug!("Downstream node configuration complete");
}
