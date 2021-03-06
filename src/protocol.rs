//! This module handles the low level details of sending requests to
//! the webdriver server using hyper, and interpreting errors. It also
//! handles some of the inconsistancies in the implementation of the
//! webdriver standard between different browsers.

// Many thanks to "Jon Gjengset <jon@thesquareplanet.com>" the author
// of fantoccini for encoding lots of useful knowledge about webdriver
// in his library. Without that single repository of quirks this
// library would have been much harder to write.

use crate::error::*;
use futures::prelude::*;
use hyper::{self, Method};
use hyper_tls;
use serde_json::Value;
use bytes::BytesMut;
use std::str::from_utf8;
use url;
use webdriver::{
    self,
    command::WebDriverCommand,
    common::{FrameId, ELEMENT_KEY},
    error::{ErrorStatus, WebDriverError},
};

type Cmd = WebDriverCommand<webdriver::command::VoidWebDriverExtensionCommand>;
type HttpClient =
    hyper::Client<hyper_tls::HttpsConnector<hyper::client::HttpConnector>, hyper::Body>;

/// A WebDriver client tied to a single browser session.
pub(crate) struct Client {
    http_client: HttpClient,
    webdriver_url: url::Url,
    user_agent: Option<String>,
    session_id: Option<String>,
    pub(crate) legacy: bool,
}

impl Drop for Client {
    fn drop(&mut self) {
        let _ = self.shutdown();
    }
}

impl Client {
    fn shutdown(&mut self) -> Result<()> {
        match self.session_id {
            None => Ok(()),
            Some(ref s) => {
                let url = self.webdriver_url.join(&format!("/session/{}", s))?;
                self.session_id = None;
                let req = hyper::Request::builder()
                    .method(Method::DELETE)
                    .uri(url.as_str())
                    .body(hyper::Body::from(""))?;
                let http = self.http_client.clone();
                tokio::spawn(async move {
                    let _ = http.request(req).await;
                });
                Ok(())
            }
        }
    }

    fn decode_error(
        &self,
        status: hyper::StatusCode,
        legacy_status: u64,
        response: Value,
    ) -> Result<Error> {
        let mut response = match response {
            Value::Object(r) => r,
            v => bail!(ErrorKind::NotW3C(v)),
        };
        // phantomjs injects a *huge* field with the entire
        // screen contents -- remove that
        response.remove("screen");
        let es = {
            if self.legacy {
                if !response.contains_key("message") || !response["message"].is_string() {
                    bail!(ErrorKind::NotW3C(Value::Object(response)));
                }
                match legacy_status {
                    6 | 33 => ErrorStatus::SessionNotCreated,
                    7 => ErrorStatus::NoSuchElement,
                    8 => ErrorStatus::NoSuchFrame,
                    9 => ErrorStatus::UnknownCommand,
                    10 => ErrorStatus::StaleElementReference,
                    11 => ErrorStatus::ElementNotInteractable,
                    12 => ErrorStatus::InvalidElementState,
                    13 => ErrorStatus::UnknownError,
                    15 => ErrorStatus::ElementNotSelectable,
                    17 => ErrorStatus::JavascriptError,
                    19 | 32 => ErrorStatus::InvalidSelector,
                    21 => ErrorStatus::Timeout,
                    23 => ErrorStatus::NoSuchWindow,
                    24 => ErrorStatus::InvalidCookieDomain,
                    25 => ErrorStatus::UnableToSetCookie,
                    26 => ErrorStatus::UnexpectedAlertOpen,
                    27 => ErrorStatus::NoSuchAlert,
                    28 => ErrorStatus::ScriptTimeout,
                    29 => ErrorStatus::InvalidCoordinates,
                    34 => ErrorStatus::MoveTargetOutOfBounds,
                    _ => bail!(ErrorKind::NotW3C(Value::Object(response))),
                }
            } else {
                use hyper::StatusCode;
                let error = match response["error"].as_str() {
                    None => bail!(ErrorKind::NotW3C(Value::Object(response))),
                    Some(e) => e,
                };
                if status == StatusCode::BAD_REQUEST {
                    match error {
                        "element click intercepted" => {
                            ErrorStatus::ElementClickIntercepted
                        }
                        "element not selectable" => ErrorStatus::ElementNotSelectable,
                        "element not interactable" => ErrorStatus::ElementNotInteractable,
                        "insecure certificate" => ErrorStatus::InsecureCertificate,
                        "invalid argument" => ErrorStatus::InvalidArgument,
                        "invalid cookie domain" => ErrorStatus::InvalidCookieDomain,
                        "invalid coordinates" => ErrorStatus::InvalidCoordinates,
                        "invalid element state" => ErrorStatus::InvalidElementState,
                        "invalid selector" => ErrorStatus::InvalidSelector,
                        "no such alert" => ErrorStatus::NoSuchAlert,
                        "no such frame" => ErrorStatus::NoSuchFrame,
                        "no such window" => ErrorStatus::NoSuchWindow,
                        "stale element reference" => ErrorStatus::StaleElementReference,
                        e => bail!("StatusCode::BadRequest: {}", e),
                    }
                } else if status == StatusCode::NOT_FOUND {
                    match error {
                        "unknown command" => ErrorStatus::UnknownCommand,
                        "no such cookie" => ErrorStatus::NoSuchCookie,
                        "invalid session id" => ErrorStatus::InvalidSessionId,
                        "no such element" => ErrorStatus::NoSuchElement,
                        "no such frame" => ErrorStatus::NoSuchFrame,
                        "no such window" => ErrorStatus::NoSuchWindow,
                        e => bail!("StatusCode::NotFound: {}", e),
                    }
                } else if status == StatusCode::INTERNAL_SERVER_ERROR {
                    match error {
                        "javascript error" => ErrorStatus::JavascriptError,
                        "move target out of bounds" => ErrorStatus::MoveTargetOutOfBounds,
                        "session not created" => ErrorStatus::SessionNotCreated,
                        "unable to set cookie" => ErrorStatus::UnableToSetCookie,
                        "unable to capture screen" => ErrorStatus::UnableToCaptureScreen,
                        "unexpected alert open" => ErrorStatus::UnexpectedAlertOpen,
                        "unknown error" => ErrorStatus::UnknownError,
                        "unsupported operation" => ErrorStatus::UnsupportedOperation,
                        e => bail!("StatusCode::InternalServerError: {}", e),
                    }
                } else if status == StatusCode::REQUEST_TIMEOUT {
                    match error {
                        "timeout" => ErrorStatus::Timeout,
                        "script timeout" => ErrorStatus::ScriptTimeout,
                        e => bail!("StatusCode::RequestTimeout: {}", e),
                    }
                } else if status == StatusCode::METHOD_NOT_ALLOWED {
                    match error {
                        "unknown method" => ErrorStatus::UnknownMethod,
                        e => bail!("StatusCode::MethodNotAllowed: {}", e),
                    }
                } else {
                    bail!("invalid status code: {:?}", status)
                }
            }
        };
        let message = match response["message"].as_str() {
            None => bail!(ErrorKind::NotW3C(Value::Object(response))),
            Some(s) => String::from(s),
        };
        Ok(Error::from(ErrorKind::WebDriver(WebDriverError::new(
            es, message,
        ))))
    }

    fn endpoint_for(&self, cmd: &Cmd) -> Result<url::Url> {
        if let WebDriverCommand::NewSession(..) = *cmd {
            return Ok(self.webdriver_url.join("/session")?);
        }
        let base = {
            let sid = match self.session_id {
                Some(ref s) => s,
                None => bail!("no session id, but not new session"),
            };
            self.webdriver_url.join(&format!("/session/{}/", sid))?
        };
        let endpoint = match cmd {
            WebDriverCommand::NewSession(..) => bail!("new session handled by init"),
            WebDriverCommand::DeleteSession => bail!("delete session handed by shutdown"),
            WebDriverCommand::Get(..) | WebDriverCommand::GetCurrentUrl => {
                base.join("url")
            }
            WebDriverCommand::GoBack => base.join("back"),
            WebDriverCommand::Refresh => base.join("refresh"),
            WebDriverCommand::GetPageSource => base.join("source"),
            WebDriverCommand::FindElement(..) => base.join("element"),
            WebDriverCommand::FindElements(..) => base.join("elements"),
            WebDriverCommand::GetCookies => base.join("cookie"),
            WebDriverCommand::ExecuteScript(..) if self.legacy => base.join("execute"),
            WebDriverCommand::ExecuteScript(..) => base.join("execute/sync"),
            WebDriverCommand::SwitchToFrame(..) => base.join("frame"),
            WebDriverCommand::SwitchToParentFrame => base.join("frame/parent"),
            WebDriverCommand::SwitchToWindow(..) => base.join("window"),
            WebDriverCommand::GetElementProperty(ref we, ref prop) => {
                base.join(&format!("element/{}/property/{}", we.0, prop))
            }
            WebDriverCommand::GetElementAttribute(ref we, ref attr) => {
                base.join(&format!("element/{}/attribute/{}", we.0, attr))
            }
            WebDriverCommand::FindElementElement(ref p, _) => {
                base.join(&format!("element/{}/element", p.0))
            }
            WebDriverCommand::FindElementElements(ref p, _) => {
                base.join(&format!("element/{}/elements", p.0))
            }
            WebDriverCommand::ElementClick(ref we) => {
                base.join(&format!("element/{}/click", we.0))
            }
            WebDriverCommand::GetElementText(ref we) => {
                base.join(&format!("element/{}/text", we.0))
            }
            WebDriverCommand::ElementSendKeys(ref we, _) => {
                base.join(&format!("element/{}/value", we.0))
            }
            x => unimplemented!("{:?}", x),
        };
        Ok(endpoint?)
    }

    fn encode_cmd(&self, cmd: &Cmd) -> Result<hyper::Request<hyper::Body>> {
        use webdriver::command;
        let (body, method) = match cmd {
            WebDriverCommand::NewSession(command::NewSessionParameters::Spec(
                ref conf,
            )) => (Some(serde_json::to_string(conf)?), Method::POST),
            WebDriverCommand::NewSession(command::NewSessionParameters::Legacy(
                ref conf,
            )) => (Some(serde_json::to_string(conf)?), Method::POST),
            WebDriverCommand::Get(ref params) => {
                (Some(serde_json::to_string(params)?), Method::POST)
            }
            WebDriverCommand::FindElement(ref loc)
            | WebDriverCommand::FindElements(ref loc)
            | WebDriverCommand::FindElementElement(_, ref loc)
            | WebDriverCommand::FindElementElements(_, ref loc) => {
                (Some(serde_json::to_string(loc)?), Method::POST)
            }
            WebDriverCommand::ExecuteScript(ref script) => {
                (Some(serde_json::to_string(script)?), Method::POST)
            }
            WebDriverCommand::ElementSendKeys(_, ref keys) => {
                (Some(serde_json::to_string(keys)?), Method::POST)
            }
            WebDriverCommand::ElementClick(..)
            | WebDriverCommand::GoBack
            | WebDriverCommand::Refresh => (Some("{}".to_string()), Method::POST),
            WebDriverCommand::SwitchToParentFrame => {
                (Some("{}".to_string()), Method::POST)
            }
            WebDriverCommand::SwitchToFrame(ref param) => {
                // unfortunatly the serializer for this command does
                // not round trip properly so we need to encode the
                // Json manually.
                let id = match param.id {
                    Some(FrameId::Element(ref e)) => Value::String(e.0.to_string()),
                    Some(FrameId::Short(_)) | None => unimplemented!(),
                };
                let p = move |k, v| {
                    let mut m = serde_json::map::Map::with_capacity(1);
                    m.insert(k, v);
                    Value::Object(m)
                };
                let msg = p("id".to_string(), p(ELEMENT_KEY.to_string(), id));
                (Some(format!("{}", msg)), Method::POST)
            }
            WebDriverCommand::SwitchToWindow(ref handle) => {
                (Some(serde_json::to_string(handle)?), Method::POST)
            }
            _ => (None, Method::GET),
        };
        let url = self.endpoint_for(&cmd)?;
        let req = hyper::Request::builder().method(method).uri(url.as_str());
        let req = match self.user_agent {
            None => req,
            Some(ref s) => req.header(hyper::header::USER_AGENT, s.as_str()),
        };
        match body {
            None => Ok(req.body(hyper::Body::from(String::new()))?),
            Some(body) => Ok(req
                .header(
                    hyper::header::CONTENT_TYPE,
                    "application/json; charset=utf-8",
                )
                .header(hyper::header::CONTENT_LENGTH, body.len() as u64)
                .body(hyper::Body::from(body))?),
        }
    }

    /// Create a new webdriver session with the server specified by url
    pub(crate) async fn new(
        webdriver_url: &str,
        user_agent: Option<String>,
    ) -> Result<Self> {
        let webdriver_url = webdriver_url.parse::<url::Url>()?;
        let http_client =
            hyper::Client::builder().build(hyper_tls::HttpsConnector::new());
        let mut client = Client {
            http_client,
            webdriver_url,
            user_agent,
            legacy: false,
            session_id: None,
        };
        let cap = {
            let mut c = webdriver::capabilities::Capabilities::new();
            // we want the browser to wait for the page to load
            c.insert(
                "pageLoadStrategy".to_string(),
                Value::String("normal".to_string()),
            );
            c
        };
        let session_config = webdriver::capabilities::SpecNewSessionParameters {
            alwaysMatch: cap.clone(),
            firstMatch: vec![],
        };
        let spec = webdriver::command::NewSessionParameters::Spec(session_config);
        match client.init(spec).await {
            Ok(()) => Ok(client),
            Err(Error(ErrorKind::NotW3C(json), _)) => {
                let legacy = match json {
                    // ghostdriver
                    Value::String(ref err) => {
                        err.starts_with("Missing Command Parameter")
                    }
                    Value::Object(ref err) => err
                        .get("message")
                        .and_then(|m| m.as_str())
                        .map(|s| {
                            // chromedriver <= 2.29
                            s.contains("cannot find dict 'desiredCapabilities'")
                                || s.contains("Missing or invalid capabilities")
                        })
                        .unwrap_or(false),
                    _ => false,
                };
                if !legacy {
                    bail!(ErrorKind::NotW3C(json))
                } else {
                    let session_config =
                        webdriver::capabilities::LegacyNewSessionParameters {
                            required: cap,
                            desired: webdriver::capabilities::Capabilities::new(),
                        };
                    let spec =
                        webdriver::command::NewSessionParameters::Legacy(session_config);
                    client.legacy = true;
                    client.init(spec).await?;
                    Ok(client)
                }
            }
            Err(e) => bail!(e),
        }
    }

    async fn init(
        &mut self,
        params: webdriver::command::NewSessionParameters,
    ) -> Result<()> {
        let cmd = WebDriverCommand::NewSession(params);
        match self.issue_cmd(&cmd).await? {
            Value::Object(mut v) => {
                if let Some(session_id) = v.remove("sessionId") {
                    if let Some(session_id) = session_id.as_str() {
                        self.session_id = Some(session_id.to_string());
                        return Ok(());
                    }
                    v.insert("sessionId".to_string(), session_id);
                    bail!(ErrorKind::NotW3C(Value::Object(v)))
                } else {
                    bail!(ErrorKind::NotW3C(Value::Object(v)))
                }
            }
            v => bail!(ErrorKind::NotW3C(v)),
        }
    }

    /// Issue a command to the webdriver server, and return the Json
    /// object returned by the server on success or Err if the request
    /// failed.
    pub(crate) async fn issue_cmd<'a>(&'a self, cmd: &'a Cmd) -> Result<Value> {
        let req = self.encode_cmd(cmd)?;
        let res = self.http_client.request(req).await?;
        match res.headers().get(hyper::header::CONTENT_TYPE) {
            None => bail!(ErrorKind::NotJson(None)),
            Some(ctype) => {
                if !ctype.to_str()?.starts_with("application/json") {
                    let c = Some(String::from(ctype.to_str()?));
                    bail!(ErrorKind::NotJson(c));
                }
            }
        }
        let status = res.status();
        let res_body = {
            let mut buf = BytesMut::new();
            let mut body = res.into_body();
            loop {
                match body.next().await {
                    Some(r) => { buf.extend_from_slice(&*(r?)); }
                    None => {
                        if buf.len() == 0 {
                            bail!("empty body");
                        } else {
                            break buf.split().freeze();
                        }
                    }
                }
            }
        };
        let is_new_session = if let WebDriverCommand::NewSession(..) = cmd {
            true
        } else {
            false
        };
        let (response, is_success, legacy_status) =
            match serde_json::from_str(from_utf8(&*res_body)?)? {
                Value::Object(mut v) => {
                    let mut is_success = status.is_success();
                    let mut legacy_status = 0;
                    if self.legacy {
                        legacy_status = v["status"].as_u64().unwrap();
                        is_success = legacy_status == 0;
                    }
                    if self.legacy && is_new_session {
                        (Value::Object(v), is_success, legacy_status)
                    } else {
                        let response = v.remove("value").ok_or_else(|| {
                            Error::from(ErrorKind::NotW3C(Value::Object(v)))
                        })?;
                        (response, is_success, legacy_status)
                    }
                }
                v => bail!(ErrorKind::NotW3C(v)),
            };
        if is_success {
            Ok(response)
        } else {
            Err(self.decode_error(status, legacy_status, response)?)
        }
    }
}
