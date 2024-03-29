use crate::error::{XenonError, XenonResult};
use crate::portmanager::ServicePort;
use crate::response::XenonResponse;
use bytes::Bytes;
use hyper::client::HttpConnector;
use hyper::http::uri::{Authority, Scheme};
use hyper::{Body, Client, Request, Response};
use log::*;
use serde::{Deserialize, Serialize};
use tokio::time::{Duration, Instant};

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

impl ToString for XenonSessionId {
    fn to_string(&self) -> String {
        self.0.clone()
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
/// sessions and parallel requests, so the Http client needs to go here
/// in the session and not on the service. This allows multiple Xenon clients
/// to make requests to the same webdriver concurrently if needed.
#[derive(Debug)]
pub struct Session {
    /// NOTE: This is the internal session id for the target WebDriver session itself.
    /// It starts out as None since it is just a placeholder for a session.
    /// This will be updated once the session actually connects.
    session_id: String,
    /// The service group this session belongs to, or None for a remote session.
    service_group: Option<String>,
    scheme: Scheme,
    authority: Authority,
    port: ServicePort,
    client: Client<HttpConnector, Body>,
    // Timestamp of last request, for handling timeouts.
    last_timestamp: Instant,
}

impl Session {
    pub async fn create(
        scheme: Scheme,
        authority: Authority,
        service_group: Option<String>,
        capabilities: &serde_json::Value,
        desired_capabilities: &serde_json::Value,
        xsession_id: XenonSessionId,
    ) -> XenonResult<(Self, Response<Body>)> {
        let client = Client::new();

        // Wait for port to be ready.
        let port = match authority.port_u16() {
            Some(p) => p,
            None => {
                return Err(XenonError::RespondWith(
                    XenonResponse::ErrorCreatingSession("Port not recognised".to_string()),
                ))
            }
        };
        let mut count = 0;
        loop {
            let status_req = Session::build_request(
                hyper::Method::GET,
                &scheme,
                &authority,
                "/status",
                Body::empty(),
            )?;
            if let Ok(response) = client.request(status_req).await {
                if response.status().is_success() {
                    break;
                }
            }

            count += 1;
            if count > 30 {
                return Err(XenonError::RespondWith(
                    XenonResponse::ErrorCreatingSession(
                        "Timed out waiting for WebDriver".to_string(),
                    ),
                ));
            }

            debug!(
                "WebDriver not available on port {}. Will retry in 1 second...",
                port
            );
            tokio::time::sleep(Duration::new(1, 0)).await;
        }

        // Send capabilities to driver verbatim.
        let caps = serde_json::json!({
            "capabilities": capabilities,
            "desiredCapabilities": desired_capabilities
        });
        let body_str = serde_json::to_string(&caps).map_err(|e| {
            XenonError::RespondWith(XenonResponse::ErrorCreatingSession(e.to_string()))
        })?;
        let req_out = Session::build_request(
            hyper::Method::POST,
            &scheme,
            &authority,
            "/session",
            Body::from(body_str),
        )?;

        let mut response = client
            .request(req_out)
            .await
            .map_err(|e| XenonError::RequestError(e.to_string()))?;
        if !response.status().is_success() {
            return Err(XenonError::ResponsePassThrough(response));
        }

        let body_bytes: Bytes = hyper::body::to_bytes(response.body_mut())
            .await
            .map_err(|e| {
                XenonError::RespondWith(XenonResponse::ErrorCreatingSession(e.to_string()))
            })?;

        // Deserialize the response into something WebDriver clients will understand.
        let mut resp: ConnectionResp = serde_json::from_slice(&body_bytes).map_err(|e| {
            XenonError::RespondWith(XenonResponse::ErrorCreatingSession(e.to_string()))
        })?;

        let session_id = if resp.session_id.is_empty() {
            resp.value.session_id
        } else {
            resp.session_id
        };

        // Switch out the session ids in the response with the one from Xenon.
        resp.session_id = xsession_id.to_string();
        resp.value.session_id = xsession_id.to_string();

        let bytes_out = serde_json::to_vec(&resp).map_err(|e| {
            XenonError::RespondWith(XenonResponse::ErrorCreatingSession(e.to_string()))
        })?;

        let resp_out = Response::builder()
            .status(response.status())
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
        method: hyper::Method,
        scheme: &Scheme,
        authority: &Authority,
        path: &str,
        body: Body,
    ) -> XenonResult<Request<Body>> {
        let uri_out = hyper::Uri::builder()
            .scheme(scheme.clone())
            .authority(authority.clone())
            .path_and_query(path)
            .build()
            .map_err(|e| XenonError::RequestError(e.to_string()))?;

        let req_out = Request::builder()
            .method(method)
            .uri(uri_out)
            .body(body)
            .map_err(|e| XenonError::RequestError(e.to_string()))?;
        Ok(req_out)
    }

    pub async fn forward_request(
        &mut self,
        req: Request<Body>,
        endpoint: &str,
    ) -> XenonResult<Response<Body>> {
        self.last_timestamp = Instant::now();

        // Substitute the uri and send the request again...
        let mut path_and_query = if endpoint.is_empty() {
            format!("/session/{}", self.session_id)
        } else {
            format!("/session/{}/{}", self.session_id, endpoint)
        };

        if let Some(q) = req.uri().query() {
            path_and_query += "?";
            path_and_query += q;
        }
        let req_out = Session::build_request(
            req.method().clone(),
            &self.scheme,
            &self.authority,
            &path_and_query,
            req.into_body(),
        )?;
        self.client
            .request(req_out)
            .await
            .map_err(|e| XenonError::RequestError(e.to_string()))
    }
}
