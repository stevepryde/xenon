use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::extract::{Path, Request, State};
use axum::http::uri::{Authority, Scheme};
use axum::http::{Method, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use axum::routing::{any, get, post};
use clap::Parser;
use http_body_util::BodyExt;
use indexmap::map::IndexMap;
use tokio::sync::RwLock;
use tokio::time::sleep;
use tracing::{debug, error, info, warn};

use crate::browser::{Capabilities, W3CCapabilities};
use crate::config::load_config;
use crate::error::{XenonError, XenonResult};
use crate::nodes::{NodeId, RemoteNode, RemoteServiceGroup};
use crate::response::XenonResponse;
use crate::service::ServiceGroup;
use crate::session::{Session, XenonSessionId};
use crate::state::{AppState, XenonState};

#[derive(Debug, Parser)]
#[command(name = "Xenon", about = "A powerful WebDriver proxy")]
pub struct Opt {
    /// The host/IP address to bind to. Default is 127.0.0.1.
    #[arg(short = 'H', long, env = "XENON_HOST", default_value = "127.0.0.1")]
    host: String,

    /// The port to listen on. Default is 4444.
    #[arg(short, long, env = "XENON_PORT", default_value_t = 4444)]
    port: u16,

    /// The path to the YAML config file. Default is xenon.yml.
    #[arg(short, long, env = "XENON_CFG")]
    cfg: Option<PathBuf>,
}

pub async fn start_server() -> XenonResult<()> {
    let opt = Opt::parse();

    if opt.port < 1024 {
        return Err(XenonError::InvalidPort);
    }

    let addr: SocketAddr = format!("{}:{}", opt.host, opt.port)
        .parse()
        .map_err(|e: std::net::AddrParseError| XenonError::InvalidBindAddr(e.to_string()))?;

    // Read config.
    let config_filename = opt.cfg.unwrap_or_else(|| PathBuf::from("xenon.yml"));
    let config = load_config(&config_filename)?;
    debug!("Config loaded:\n{:#?}", config);
    let using_nodes = config.has_nodes();
    let app_state = AppState::new(XenonState::new(config)?);

    let (tx_terminator, rx_terminator) = tokio::sync::oneshot::channel();

    // Spawn session timeout task.
    {
        let state = app_state.xenon.clone();
        tokio::spawn(async move {
            process_session_timeout(state, rx_terminator).await;
        });
    }
    if using_nodes {
        // Spawn config getter (uses the shared client).
        let state = app_state.clone();
        tokio::spawn(async move {
            process_node_init(state).await;
        });
    }

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .map_err(|e| XenonError::ServerError(e.to_string()))?;
    info!("Server running at {addr}");

    let result = axum::serve(listener, router(app_state))
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
            info!("Shutdown signal received");
        })
        .await
        .map_err(|e| XenonError::ServerError(e.to_string()));

    if let Err(e) = tx_terminator.send(true) {
        error!("Error terminating timeout task: {e:?}");
    }

    result
}

pub fn router(state: AppState) -> Router {
    let webdriver = Router::new()
        .route("/session", post(handle_create_session_route))
        .route("/session/{session_id}", any(handle_forward_root))
        .route("/session/{session_id}/{*rest}", any(handle_forward_path));

    Router::new()
        .route("/", get(index))
        .route("/status", get(handle_status))
        .route("/node/config", get(handle_node_config))
        .merge(webdriver.clone())
        .nest("/wd/hub", webdriver)
        .with_state(state)
}

async fn index() -> &'static str {
    "TODO: show status page"
}

/// Selenium-4-shaped status payload.
async fn handle_status() -> Response {
    let body = serde_json::json!({
        "value": {
            "ready": true,
            "message": "Xenon ready",
        }
    });
    (
        StatusCode::OK,
        [(axum::http::header::CONTENT_TYPE, "application/json")],
        body.to_string(),
    )
        .into_response()
}

async fn handle_create_session_route(
    State(state): State<AppState>,
    req: Request,
) -> Result<Response, XenonError> {
    let body_bytes = req
        .into_body()
        .collect()
        .await
        .map_err(|e| XenonError::RespondWith(XenonResponse::ErrorCreatingSession(e.to_string())))?
        .to_bytes();

    let w3c_capabilities: W3CCapabilities = serde_json::from_slice(&body_bytes)
        .map_err(|e| XenonError::RespondWith(XenonResponse::ErrorCreatingSession(e.to_string())))?;
    info!("Request new session :: {:#?}", &w3c_capabilities);
    let capabilities: Capabilities = serde_json::from_value(w3c_capabilities.capabilities.clone())
        .map_err(|e| {
            XenonError::RespondWith(XenonResponse::ErrorCreatingSession(e.to_string()))
        })?;

    match handle_create_session(&capabilities, &w3c_capabilities, &state).await {
        Ok(x) => Ok(x),
        Err(XenonError::RespondWith(XenonResponse::NoSessionsAvailable)) => {
            // In this case there is at least 1 matching browser locally, so even if
            // the node search returns no matching browser, the no matching sessions
            // error takes precedence.
            match handle_create_session_node(&capabilities, &w3c_capabilities, &state).await {
                Ok(x) => Ok(x),
                Err(XenonError::RespondWith(XenonResponse::NoMatchingBrowser)) => Err(
                    XenonError::RespondWith(XenonResponse::NoSessionsAvailable),
                ),
                Err(e) => Err(e),
            }
        }
        Err(XenonError::RespondWith(XenonResponse::NoMatchingBrowser)) => {
            handle_create_session_node(&capabilities, &w3c_capabilities, &state).await
        }
        Err(e) => Err(e),
    }
}

async fn handle_forward_root(
    state: State<AppState>,
    Path(session_id): Path<String>,
    req: Request,
) -> Result<Response, XenonError> {
    forward_inner(state, session_id, None, req).await
}

async fn handle_forward_path(
    state: State<AppState>,
    Path((session_id, rest)): Path<(String, String)>,
    req: Request,
) -> Result<Response, XenonError> {
    forward_inner(state, session_id, Some(rest), req).await
}

async fn forward_inner(
    State(state): State<AppState>,
    session_id: String,
    rest: Option<String>,
    req: Request,
) -> Result<Response, XenonError> {
    let xsession_id = XenonSessionId::from(session_id);
    let is_delete = rest.is_none() && req.method() == Method::DELETE;

    // Look up the session under a brief read-lock.
    let mutex_session = {
        let s = state.xenon.read().await;
        s.get_session(&xsession_id).ok_or_else(|| {
            XenonError::RespondWith(XenonResponse::SessionNotFound(xsession_id.to_string()))
        })?
    };

    let remaining_path = rest.unwrap_or_default();
    let mut session = mutex_session.lock().await;
    let response = session.forward_request(req, &remaining_path).await?;

    if is_delete && response.status().is_success() {
        info!("Session Delete {:?} :: port {}", xsession_id, session.port());
        let port = session.port();
        let session_group = session.service_group().clone();
        // Drop the session lock before reaching for state write-locks below,
        // to avoid holding it across an await we don't need it for.
        drop(session);

        // Remove the session under a write-lock on state.
        {
            let mut s = state.xenon.write().await;
            s.delete_session(&xsession_id);
        }

        // For local sessions, remove the session from its service group.
        if let Some(group_name) = session_group {
            let s = state.xenon.read().await;
            let rwlock_groups = s.service_groups();
            let rwlock_port_manager = s.port_manager();
            let (mut port_manager, mut groups) =
                tokio::join!(rwlock_port_manager.write(), rwlock_groups.write());

            if let Some(group) = groups.get_mut(&group_name) {
                group
                    .delete_session(port, &xsession_id, &mut port_manager)
                    .await;
            }
        }
    }

    Ok(response)
}

pub async fn handle_create_session(
    capabilities: &Capabilities,
    w3c_capabilities: &W3CCapabilities,
    state: &AppState,
) -> XenonResult<Response> {
    let (xsession_id, port, group_name) = reserve_available_session(state, capabilities).await?;

    // Create the session. No locks are held at all here.
    info!("Session Create {:?} :: port {}", xsession_id, port);
    let authority: Authority = match format!("localhost:{port}").parse() {
        Ok(a) => a,
        Err(e) => {
            return Err(XenonError::RespondWith(
                XenonResponse::ErrorCreatingSession(format!("Invalid port '{port}': {e}")),
            ));
        }
    };
    match Session::create(
        state.client.clone(),
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
            let mut s = state.xenon.write().await;
            s.add_session(xsession_id, session);
            // Forward the response back to the client.
            Ok(response)
        }
        Err(err) => {
            // Free the slot we reserved.
            release_session_slot(state, &group_name, port, &xsession_id).await;
            match err {
                XenonError::ResponsePassThrough(response) => Ok(*response),
                e => Err(e),
            }
        }
    }
}

async fn release_session_slot(
    state: &AppState,
    group_name: &str,
    port: u16,
    xsession_id: &XenonSessionId,
) {
    let s = state.xenon.read().await;
    let rwlock_groups = s.service_groups();
    let rwlock_port_manager = s.port_manager();
    let (mut port_manager, mut groups) =
        tokio::join!(rwlock_port_manager.write(), rwlock_groups.write());
    if let Some(group) = groups.get_mut(group_name) {
        group
            .delete_session(port, xsession_id, &mut port_manager)
            .await;
    }
}

pub async fn reserve_available_session(
    state: &AppState,
    capabilities: &Capabilities,
) -> XenonResult<(XenonSessionId, u16, String)> {
    let s = state.xenon.read().await;
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

    // A new session request might match several groups; if any session fails
    // to start, fall back to the next available group.
    let mut first_error: Option<XenonError> = None;
    for group_name in group_names {
        let Some(group) = groups.get_mut(&group_name) else {
            continue;
        };

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
    state: &AppState,
) -> XenonResult<Response> {
    // Note we need to get the node data under read lock but we need to give that up
    // asap because we need a write lock later once a session is created.
    let (node_data, matched_caps) = {
        let s = state.xenon.read().await;
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
        info!("Attempt Session Create {xsession_id:?} :: Node '{name}'");
        if let Ok((session, response)) = Session::create(
            state.client.clone(),
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
            let mut s = state.xenon.write().await;
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

                    info!("Session Timeout {:?} :: port {}", xsession_id, session.port());
                    if let Some(session_group) = session.service_group() {
                        if let Some(group) = groups.get_mut(session_group) {
                            group
                                .delete_session(session.port(), &xsession_id, &mut port_manager)
                                .await;
                        }
                    }
                }
            }
        }
        sleep(Duration::from_secs(60)).await;
    }
}

async fn handle_node_config(State(state): State<AppState>) -> Result<Response, XenonError> {
    let s = state.xenon.read().await;
    let rwlock_groups = s.service_groups();

    let mut groups_out = Vec::new();
    for group in rwlock_groups.read().await.values() {
        let remote_group = RemoteServiceGroup {
            browser: group.browser.clone(),
            remaining_sessions: group.browser.max_sessions(),
        };
        groups_out.push(remote_group);
    }

    // Also expose remote nodes.
    let rwlock_nodes = s.remote_nodes();
    for node in rwlock_nodes.read().await.values() {
        for remote_group in &node.service_groups {
            groups_out.push(remote_group.clone());
        }
    }

    let body = serde_json::to_string(&groups_out)
        .map_err(|e| XenonError::ServerError(format!("Failed to serialize node config: {e}")))?;

    Ok((
        StatusCode::OK,
        [(axum::http::header::CONTENT_TYPE, "application/json")],
        body,
    )
        .into_response())
}

/// Fetch config for each node.
async fn process_node_init(state: AppState) {
    debug!("Downstream node configuration starting");
    let mut nodes_remaining: IndexMap<NodeId, RemoteNode> = {
        let s = state.xenon.read().await;
        let rwlock_nodes = s.remote_nodes();
        rwlock_nodes.read().await.clone()
    };

    while !nodes_remaining.is_empty() {
        let mut nodes_done = Vec::new();
        for node in nodes_remaining.values() {
            debug!("Fetching config from downstream node '{}'...", node.display_name());
            let uri_out = match Uri::builder()
                .scheme(node.scheme.clone())
                .authority(node.authority.clone())
                .path_and_query("/node/config")
                .build()
            {
                Ok(uri) => uri,
                Err(e) => {
                    error!(
                        "Invalid URI '{}' for node '{}': {e}",
                        node.url,
                        node.display_name()
                    );
                    continue;
                }
            };

            let req = match axum::http::Request::builder()
                .method(Method::GET)
                .uri(uri_out)
                .body(Body::empty())
            {
                Ok(req) => req,
                Err(e) => {
                    error!("Failed to build node config request: {e}");
                    continue;
                }
            };

            match state.client.request(req).await {
                Ok(res) => match res.into_body().collect().await {
                    Ok(collected) => {
                        let bytes = collected.to_bytes();
                        let remote_groups: Vec<RemoteServiceGroup> =
                            match serde_json::from_slice(&bytes) {
                                Ok(x) => x,
                                Err(e) => {
                                    error!(
                                        "Failed to parse configuration from node '{}': {e}",
                                        node.display_name()
                                    );
                                    continue;
                                }
                            };

                        // Update these. Read lock on state. Write lock on nodes.
                        let s = state.xenon.read().await;
                        let rwlock_nodes = s.remote_nodes();
                        let mut nodes = rwlock_nodes.write().await;
                        if let Some(node_mut) = nodes.get_mut(&node.id()) {
                            node_mut.service_groups = remote_groups.clone();
                        }
                        info!(
                            "Configuration for downstream node '{}' fetched successfully",
                            node.display_name()
                        );
                        info!("{remote_groups:#?}");
                        nodes_done.push(node.id());
                    }
                    Err(e) => {
                        error!(
                            "Failed to receive configuration for node '{}': {e}",
                            node.display_name()
                        );
                        continue;
                    }
                },
                Err(e) => {
                    warn!(
                        "Unable to fetch configuration for node '{}': {e}",
                        node.display_name()
                    );
                    continue;
                }
            }
        }

        // Remove the ones we found.
        for node_id in nodes_done {
            nodes_remaining.shift_remove(&node_id);
        }

        if !nodes_remaining.is_empty() {
            // Wait 60 seconds before trying again.
            sleep(Duration::from_secs(60)).await;
        }
    }

    debug!("Downstream node configuration complete");
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;
    use axum::http::Request as HttpRequest;
    use tower::ServiceExt;

    fn empty_state() -> AppState {
        let cfg = crate::config::XenonConfig::default();
        AppState::new(XenonState::new(cfg).expect("state"))
    }

    #[tokio::test]
    async fn status_returns_w3c_shaped_json() {
        let app = router(empty_state());
        let resp = app
            .oneshot(
                HttpRequest::builder()
                    .uri("/status")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let bytes = to_bytes(resp.into_body(), 4096).await.unwrap();
        let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(v["value"]["ready"], true);
    }

    #[tokio::test]
    async fn node_config_returns_empty_array_for_empty_state() {
        let app = router(empty_state());
        let resp = app
            .oneshot(
                HttpRequest::builder()
                    .uri("/node/config")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let bytes = to_bytes(resp.into_body(), 4096).await.unwrap();
        assert_eq!(&bytes[..], b"[]");
    }

    #[tokio::test]
    async fn unknown_session_returns_invalid_session_id() {
        let app = router(empty_state());
        let resp = app
            .oneshot(
                HttpRequest::builder()
                    .uri("/session/does-not-exist/url")
                    .method(Method::POST)
                    .body(Body::from(r#"{"url":"https://x"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
        let bytes = to_bytes(resp.into_body(), 4096).await.unwrap();
        let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(v["value"]["error"], "invalid session id");
    }
}
