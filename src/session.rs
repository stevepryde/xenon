use crate::error::{XenonError, XenonResult};
use crate::portmanager::ServicePort;
use crate::response::XenonResponse;
use axum::body::Body;
use axum::http::uri::{Authority, Scheme};
use axum::http::{Method, Request, Response};
use http_body_util::BodyExt;
use hyper_util::client::legacy::connect::HttpConnector;
use hyper_util::client::legacy::Client;
use serde::{Deserialize, Serialize};
use std::fmt::{self, Display, Formatter};
use tokio::time::{Duration, Instant};
use tracing::debug;

pub type ProxyClient = Client<HttpConnector, Body>;

#[derive(Debug, Clone, Eq, PartialEq, Hash)]
pub struct XenonSessionId(String);

impl<T> From<T> for XenonSessionId
where
    T: Into<String>,
{
    fn from(value: T) -> Self {
        XenonSessionId(value.into())
    }
}

impl Default for XenonSessionId {
    fn default() -> Self {
        Self(uuid::Uuid::new_v4().to_string())
    }
}

impl XenonSessionId {
    pub fn new() -> Self {
        Self::default()
    }
}

impl Display for XenonSessionId {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct ConnectionData {
    #[serde(default, rename = "sessionId")]
    session_id: String,
    #[serde(default)]
    capabilities: serde_json::Value,
}

#[derive(Debug, Serialize, Deserialize)]
struct ConnectionResp {
    #[serde(default, rename = "sessionId")]
    session_id: String,
    value: ConnectionData,
}

/// A Session represents one browser session with one webdriver.
/// Note that a single webdriver such as chromedriver can have multiple
/// sessions and parallel requests, so the Http client is shared at the
/// process level (held in `AppState`) and reused here. The shared client
/// pools connections per-authority, so back-to-back requests to the same
/// chromedriver port reuse keep-alive sockets.
#[derive(Debug)]
pub struct Session {
    /// NOTE: This is the internal session id for the target WebDriver session itself.
    session_id: String,
    /// The service group this session belongs to, or None for a remote session.
    service_group: Option<String>,
    scheme: Scheme,
    authority: Authority,
    port: ServicePort,
    client: ProxyClient,
    /// Timestamp of last request, for handling timeouts.
    last_timestamp: Instant,
}

impl Session {
    pub async fn create(
        client: ProxyClient,
        scheme: Scheme,
        authority: Authority,
        service_group: Option<String>,
        capabilities: &serde_json::Value,
        desired_capabilities: &serde_json::Value,
        xsession_id: XenonSessionId,
    ) -> XenonResult<(Self, Response<Body>)> {
        // Wait for port to be ready.
        let port = match authority.port_u16() {
            Some(p) => p,
            None => {
                return Err(XenonError::RespondWith(
                    XenonResponse::ErrorCreatingSession("Port not recognised".to_string()),
                ));
            }
        };

        for _ in 0..30 {
            let status_req =
                Session::build_request(Method::GET, &scheme, &authority, "/status", Body::empty())?;
            if let Ok(response) = client.request(status_req).await {
                if response.status().is_success() {
                    break;
                }
            }
            debug!("WebDriver not available on port {port}. Will retry in 1 second...");
            tokio::time::sleep(Duration::from_secs(1)).await;
        }

        // Send capabilities to driver. Selenium 4 / W3C clients only send the
        // `capabilities` object — only forward `desiredCapabilities` if the
        // client actually provided it (legacy Selenium 3 / JSON Wire compat).
        let caps = if desired_capabilities.is_null() {
            serde_json::json!({ "capabilities": capabilities })
        } else {
            serde_json::json!({
                "capabilities": capabilities,
                "desiredCapabilities": desired_capabilities,
            })
        };
        let body_str = serde_json::to_string(&caps).map_err(|e| {
            XenonError::RespondWith(XenonResponse::ErrorCreatingSession(e.to_string()))
        })?;
        let req_out = Session::build_request(
            Method::POST,
            &scheme,
            &authority,
            "/session",
            Body::from(body_str),
        )?;

        let response = client
            .request(req_out)
            .await
            .map_err(|e| XenonError::RequestError(e.to_string()))?;

        let (parts, body) = response.into_parts();
        if !parts.status.is_success() {
            // Pass the upstream response straight back to the client, body and all.
            let response = Response::from_parts(parts, Body::new(body));
            return Err(XenonError::ResponsePassThrough(Box::new(response)));
        }

        let body_bytes = body
            .collect()
            .await
            .map_err(|e| {
                XenonError::RespondWith(XenonResponse::ErrorCreatingSession(e.to_string()))
            })?
            .to_bytes();

        // Deserialize the response into something WebDriver clients will understand.
        let mut resp: ConnectionResp = serde_json::from_slice(&body_bytes).map_err(|e| {
            XenonError::RespondWith(XenonResponse::ErrorCreatingSession(e.to_string()))
        })?;

        let session_id = if resp.session_id.is_empty() {
            std::mem::take(&mut resp.value.session_id)
        } else {
            std::mem::take(&mut resp.session_id)
        };

        // Switch out the session ids in the response with the one from Xenon.
        let xs = xsession_id.to_string();
        resp.session_id = xs.clone();
        resp.value.session_id = xs;

        let bytes_out = serde_json::to_vec(&resp).map_err(|e| {
            XenonError::RespondWith(XenonResponse::ErrorCreatingSession(e.to_string()))
        })?;

        let resp_out = Response::builder()
            .status(parts.status)
            .header("Content-Type", "application/json")
            .body(Body::from(bytes_out))
            .map_err(|e| {
                XenonError::RespondWith(XenonResponse::ErrorCreatingSession(e.to_string()))
            })?;

        Ok((
            Self {
                session_id,
                service_group,
                scheme,
                authority,
                port,
                client,
                last_timestamp: Instant::now(),
            },
            resp_out,
        ))
    }

    pub fn port(&self) -> ServicePort {
        self.port
    }

    pub fn service_group(&self) -> &Option<String> {
        &self.service_group
    }

    pub fn seconds_since_last_request(&self) -> u64 {
        self.last_timestamp.elapsed().as_secs()
    }

    pub fn build_request(
        method: Method,
        scheme: &Scheme,
        authority: &Authority,
        path: &str,
        body: Body,
    ) -> XenonResult<Request<Body>> {
        let uri_out = axum::http::Uri::builder()
            .scheme(scheme.clone())
            .authority(authority.clone())
            .path_and_query(path)
            .build()
            .map_err(|e| XenonError::RequestError(e.to_string()))?;

        Request::builder()
            .method(method)
            .uri(uri_out)
            .body(body)
            .map_err(|e| XenonError::RequestError(e.to_string()))
    }

    pub async fn forward_request(
        &mut self,
        req: Request<Body>,
        endpoint: &str,
    ) -> XenonResult<Response<Body>> {
        self.last_timestamp = Instant::now();

        // Substitute the uri and send the request again. Body is moved through
        // unchanged — hyper-util streams it upstream without buffering.
        let mut path_and_query = if endpoint.is_empty() {
            format!("/session/{}", self.session_id)
        } else {
            format!("/session/{}/{}", self.session_id, endpoint)
        };

        if let Some(q) = req.uri().query() {
            path_and_query.push('?');
            path_and_query.push_str(q);
        }
        let req_out = Session::build_request(
            req.method().clone(),
            &self.scheme,
            &self.authority,
            &path_and_query,
            req.into_body(),
        )?;

        let upstream = self
            .client
            .request(req_out)
            .await
            .map_err(|e| XenonError::RequestError(e.to_string()))?;

        let (parts, body) = upstream.into_parts();
        Ok(Response::from_parts(parts, Body::new(body)))
    }
}
