use crate::config::BrowserConfig;
use crate::error::XenonError;
use hyper::client::HttpConnector;
use hyper::{Body, Client, Request, Response};
use serde::export::Formatter;
use serde::Deserialize;
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

#[derive(Debug)]
pub struct Session {
    browser: BrowserConfig,
    // NOTE: This is the internal session id for the target WebDriver session itself.
    session_id: String,
    port: u16,
    process: Child,
    client: Client<HttpConnector, Body>,
}

impl Session {
    pub fn start(browser: BrowserConfig, port: u16) -> Result<Self, std::io::Error> {
        // TODO: May need to abstract this to support drivers other than chromedriver/geckodriver.
        let child_process = Command::new(browser.driver_path())
            .arg("--port")
            .arg(port.to_string())
            .spawn()?;
        Ok(Self {
            browser,
            session_id: String::new(),
            port,
            process: child_process,
            client: Client::new(),
        })
    }

    pub async fn handle_request(
        &self,
        req: Request<Body>,
        uri: &str,
    ) -> Result<Response<Body>, XenonError> {
        // Substitute the uri and send the request again...
        let mut path_and_query = format!("/session/{}/{}", self.session_id, uri);
        if let Some(q) = req.uri().query() {
            path_and_query += "?";
            path_and_query += q;
        }

        let uri_out = hyper::Uri::builder()
            .scheme("http")
            .authority("localhost")
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

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserMatch {
    browser_name: String,
    browser_version: String,
    platform_name: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Capabilities {
    always_match: BrowserMatch,
}
