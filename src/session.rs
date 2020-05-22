use crate::browser::Capabilities;
use crate::error::{XenonError, XenonResult};
use crate::portmanager::ServicePort;
use crate::response::XenonResponse;
use chrono::{DateTime, Local};
use hyper::client::HttpConnector;
use hyper::http::uri::Authority;
use hyper::{Body, Client, Request, Response};
use log::*;
use serde::{Deserialize, Serialize};

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
    #[serde(default, rename(deserialize = "sessionId"))]
    session_id: String,
    #[serde(default)]
    capabilities: serde_json::Value,
}

#[derive(Debug, Serialize, Deserialize)]
struct ConnectionResp {
    #[serde(default)]
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
    // NOTE: This is the internal session id for the target WebDriver session itself.
    // It starts out as None since it is just a placeholder for a session.
    // This will be updated once the session actually connects.
    session_id: String,
    service_group: String,
    port: ServicePort,
    client: Client<HttpConnector, Body>,
    // Timestamp of last request, for handling timeouts.
    last_timestamp: DateTime<Local>,
}

impl Session {
    pub async fn create(
        port: ServicePort,
        service_group: &str,
        capabilities: &Capabilities,
        xsession_id: XenonSessionId,
    ) -> XenonResult<(Self, Response<Body>)> {
        let client = Client::new();

        // Wait for port to be ready.
        let host = format!("localhost:{}", port);
        loop {
            let status_req =
                Session::build_request(hyper::Method::GET, &host, "/status", Body::empty())?;
            match client.request(status_req).await {
                Ok(response) => {
                    debug!("STATUS is {:?}", response);
                    break;
                }
                Err(e) => {
                    debug!("STATUS ERROR: {:?}", e);
                }
            }
        }

        // Send empty capabilities request to the WebDriver because we already
        // handled the capabilities matching internally.
        let caps = serde_json::json!({
            "capabilities": {
                "firstMatch": [{}], "alwaysMatch": {}
            }
        });
        let body_str = serde_json::to_string(&caps).map_err(|e| {
            XenonError::RespondWith(XenonResponse::ErrorCreatingSession(e.to_string()))
        })?;
        let req_out =
            Session::build_request(hyper::Method::POST, &host, "/session", Body::from(body_str))?;

        debug!("WebDriver request: {:?}", req_out);
        let mut response = client
            .request(req_out)
            .await
            .map_err(|e| XenonError::RequestError(e.to_string()))?;
        debug!("WebDriver returned {:?}", response);
        if !response.status().is_success() {
            return Err(XenonError::ResponsePassThrough(response));
        }

        let body_bytes = hyper::body::to_bytes(response.body_mut())
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
        debug!("Sending response: {:?}", bytes_out);
        let resp_out = Response::builder()
            .status(response.status())
            .body(Body::from(bytes_out))
            .map_err(|e| {
                XenonError::RespondWith(XenonResponse::ErrorCreatingSession(e.to_string()))
            })?;

        Ok((
            Self {
                session_id,
                port,
                service_group: service_group.to_string(),
                client,
                last_timestamp: Local::now(),
            },
            resp_out,
        ))
    }

    pub fn port(&self) -> ServicePort {
        self.port
    }

    pub fn service_group(&self) -> &str {
        &self.service_group
    }

    pub fn build_request(
        method: hyper::Method,
        host: &str,
        path: &str,
        body: Body,
    ) -> XenonResult<Request<Body>> {
        let authority: Authority = host.parse().unwrap();
        let uri_out = hyper::Uri::builder()
            .scheme("http")
            .authority(authority)
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
        &self,
        req: Request<Body>,
        endpoint: &str,
    ) -> XenonResult<Response<Body>> {
        // Substitute the uri and send the request again...
        let mut path_and_query = format!("/session/{}/{}", self.session_id, endpoint);
        if let Some(q) = req.uri().query() {
            path_and_query += "?";
            path_and_query += q;
        }
        let host = format!("localhost:{}", self.port);
        let req_out = Session::build_request(
            req.method().clone(),
            &host,
            &path_and_query,
            req.into_body(),
        )?;
        self.client
            .request(req_out)
            .await
            .map_err(|e| XenonError::RequestError(e.to_string()))
    }
}
