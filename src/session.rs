use crate::error::{XenonError, XenonResult};
use crate::portmanager::ServicePort;
use crate::response::XenonResponse;
use chrono::{DateTime, Local};
use hyper::client::HttpConnector;
use hyper::http::uri::Authority;
use hyper::{Body, Client, Request, Response};
use serde::export::Formatter;
use serde::Deserialize;
use std::collections::HashMap;
use std::process::{Child, Command};

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

impl std::fmt::Display for XenonSessionId {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl XenonSessionId {
    pub fn new() -> Self {
        Self::default()
    }
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
    session_id: Option<String>,
    port: ServicePort,
    client: Client<HttpConnector, Body>,
    // Timestamp of last request, for handling timeouts.
    last_timestamp: DateTime<Local>,
}

impl Session {
    pub fn new(port: ServicePort) -> Self {
        Self {
            session_id: None,
            port,
            client: Client::new(),
            last_timestamp: Local::now(),
        }
    }

    pub fn session_id(&self) -> XenonResult<&str> {
        match &self.session_id {
            Some(x) => Ok(&x),
            None => Err(XenonError::RespondWith(XenonResponse::SessionNotFound(
                "Missing session id (not started?)".to_string(),
            ))),
        }
    }

    pub async fn forward_request(
        &self,
        req: Request<Body>,
        endpoint: &str,
    ) -> XenonResult<Response<Body>> {
        // Substitute the uri and send the request again...
        let mut path_and_query = format!("/session/{}/{}", self.session_id()?, endpoint);
        if let Some(q) = req.uri().query() {
            path_and_query += "?";
            path_and_query += q;
        }

        let host = format!("localhost:{}", self.port);
        let authority: Authority = host.parse().unwrap();
        let uri_out = hyper::Uri::builder()
            .scheme("http")
            .authority(authority)
            .path_and_query(path_and_query.as_str())
            .build()
            .map_err(|e| XenonError::RequestError(e.to_string()))?;

        let req_out = Request::builder()
            .method(req.method())
            .uri(uri_out)
            .body(req.into_body())
            .map_err(|e| XenonError::RequestError(e.to_string()))?;
        self.client
            .request(req_out)
            .await
            .map_err(|e| XenonError::RequestError(e.to_string()))
    }
}
