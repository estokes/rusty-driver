//! Low level communication to the webdriver server built on top of hyper

use webdriver::{
    self, command::WebDriverCommand,
    error::{WebDriverError, ErrorStatus}, common::{ELEMENT_KEY, FrameId}
};
use std::{collections::BTreeMap, sync::{Arc, Mutex}, str::from_utf8};
use url;
use tokio::spawn;
use rustc_serialize::json::Json;
use futures::prelude::await;
use futures::prelude::*;
use hyper_tls;
use hyper::{self, Method};
use error::*;

type Cmd = WebDriverCommand<webdriver::command::VoidWebDriverExtensionCommand>;
type HttpClient =
    hyper::Client<hyper_tls::HttpsConnector<hyper::client::HttpConnector>, hyper::Body>;

struct ClientInner {
    http_client: HttpClient,
    webdriver_url: url::Url,
    user_agent: Option<String>,
    session_id: Option<String>,
    legacy: bool
}

impl ClientInner {
    fn shutdown(&mut self) -> Result<()> {
        match self.session_id {
            None => Ok(()),
            Some(ref s) => {
                let url = self.webdriver_url.join(&format!("/session/{}", s))?;
                self.session_id = None;
                let req = 
                    hyper::Request::builder()
                    .method(Method::DELETE)
                    .uri(url.as_str())
                    .body(hyper::Body::from(""))?;
                let http = self.http_client.clone();
                spawn(async_block! {
                    let _ = await!(http.request(req));
                    Ok(())
                });
                Ok(())
            }
        }
    }
}

impl Drop for ClientInner {
    fn drop(&mut self) { let _ = self.shutdown(); }
}

/// A WebDriver client tied to a single browser session.
#[derive(Clone)]
pub struct Client(Arc<Mutex<ClientInner>>);

fn encode_cmd(cmd: &Cmd) -> (Option<String>, Method) {
    use rustc_serialize::json::ToJson;
    use webdriver::command;
    match cmd {
        WebDriverCommand::NewSession(command::NewSessionParameters::Spec(ref conf)) =>
            (Some(format!("{}", conf.to_json())), Method::POST),
        WebDriverCommand::NewSession(
            command::NewSessionParameters::Legacy(ref conf)
        ) =>
            (Some(format!("{}", conf.to_json())), Method::POST),
        WebDriverCommand::Get(ref params) => 
            (Some(format!("{}", params.to_json())), Method::POST),
        WebDriverCommand::FindElement(ref loc)
            | WebDriverCommand::FindElements(ref loc)
            | WebDriverCommand::FindElementElement(_, ref loc) =>
            (Some(format!("{}", loc.to_json())), Method::POST),
        WebDriverCommand::ExecuteScript(ref script) =>
            (Some(format!("{}", script.to_json())), Method::POST),
        WebDriverCommand::ElementSendKeys(_, ref keys) =>
            (Some(format!("{}", keys.to_json())), Method::POST),
        WebDriverCommand::ElementClick(..)
            | WebDriverCommand::GoBack
            | WebDriverCommand::Refresh => 
            (Some("{}".to_string()), Method::POST),
        WebDriverCommand::SwitchToParentFrame => 
            (Some("{}".to_string()), Method::POST),
        WebDriverCommand::SwitchToFrame(ref param) => {
            // unfortunatly the serializer for this command does
            // not round trip properly so we need to encode the
            // Json manually.
            let id = match param.id {
                FrameId::Element(ref e) => Json::String(e.id.to_string()),
                FrameId::Null | FrameId::Short(_) => unimplemented!()
            };
            let p = move |k, v| {
                let mut m = BTreeMap::new(); 
                m.insert(k, v);
                Json::Object(m)
            };
            let msg = p("id".to_string(), p(ELEMENT_KEY.to_string(), id));
            (Some(format!("{}", msg)), Method::POST)
        },
        WebDriverCommand::SwitchToWindow(ref handle) =>
            (Some(format!("{}", handle.to_json())), Method::POST),
        _ => (None, Method::GET)
    }
}

impl Client {
    /// Create a new `Client` associated with a new WebDriver session
    /// on the server at the given URL.
    #[async]
    pub fn new(webdriver_url: String) -> Result<Self> {
        let webdriver_url = webdriver_url.parse::<url::Url>()?;
        let http_client =
            hyper::Client::builder().build(hyper_tls::HttpsConnector::new(8).unwrap());
        let client = Client(Arc::new(Mutex::new(ClientInner {
            http_client, webdriver_url, legacy: false,
            user_agent: None, session_id: None
        })));
        let cap = {
            let mut c = webdriver::capabilities::Capabilities::new();
            // we want the browser to wait for the page to load
            c.insert("pageLoadStrategy".to_string(), Json::String("normal".to_string()));
            c
        };
        Ok(client)
        /*
        let session_config = webdriver::capabilities::SpecNewSessionParameters {
            alwaysMatch: cap.clone(), firstMatch: vec![],
        };
        let spec = webdriver::command::NewSessionParameters::Spec(session_config);
        match await!(client.clone().init(spec)) {
            Ok(()) => Ok(client),
            Err(Error(ErrorKind::NotW3C(json), _)) => {
                let legacy = match json {
                    // ghostdriver
                    Json::String(ref err) => err.starts_with("Missing Command Parameter"),
                    Json::Object(ref err) =>
                        err.get("message").and_then(|m| m.as_string()).map(|s| {
                            // chromedriver <= 2.29
                            s.contains("cannot find dict 'desiredCapabilities'")
                                || s.contains("Missing or invalid capabilities")
                        }).unwrap_or(false),
                    _ => false
                };
                if !legacy { bail!(ErrorKind::NotW3C(json)) }
                else {
                    let session_config =
                        webdriver::capabilities::LegacyNewSessionParameters {
                            required: cap,
                            desired: webdriver::capabilities::Capabilities::new(),
                        };
                    let spec =
                        webdriver::command::NewSessionParameters::Legacy(session_config);
                    client.0.lock().unwrap().legacy = true;
                    await!(client.clone().init(spec))?;
                    Ok(client)
                }
            },
            e => e,
        }
         */
    }

    #[async]
    fn init(self, params: webdriver::command::NewSessionParameters) -> Result<()> {
        match await!(self.clone().issue_cmd(WebDriverCommand::NewSession(params)))? {
            Json::Object(mut v) => {
                if let Some(session_id) = v.remove("sessionId") {
                    if let Some(session_id) = session_id.as_string() {
                        self.0.lock().unwrap().session_id = Some(session_id.to_string());
                        return Ok(());
                    }
                    v.insert("sessionId".to_string(), session_id);
                    bail!(ErrorKind::NotW3C(Json::Object(v)))
                } else {
                    bail!(ErrorKind::NotW3C(Json::Object(v)))
                }
            }
            v => bail!(ErrorKind::NotW3C(v))
        }
    }

    fn endpoint_for(&self, cmd: &Cmd) -> Result<url::Url> {
        let inner = self.0.lock().unwrap();
        if let WebDriverCommand::NewSession(..) = *cmd {
            return Ok(inner.webdriver_url.join("/session")?);
        }
        let legacy = inner.legacy;
        let base = {
            let sid = match inner.session_id {
                Some(ref s) => s,
                None => bail!("no session id, but not new session")
            };
            inner.webdriver_url.join(&format!("/session/{}/", sid))?
        };
        let endpoint = match *cmd {
            WebDriverCommand::NewSession(..) => unreachable!(),
            WebDriverCommand::DeleteSession => unreachable!(),
            WebDriverCommand::Get(..) | WebDriverCommand::GetCurrentUrl =>
                base.join("url"),
            WebDriverCommand::GoBack => base.join("back"),
            WebDriverCommand::Refresh => base.join("refresh"),
            WebDriverCommand::GetPageSource => base.join("source"),
            WebDriverCommand::FindElement(..) => base.join("element"),
            WebDriverCommand::FindElements(..) => base.join("elements"),
            WebDriverCommand::GetCookies => base.join("cookie"),
            WebDriverCommand::ExecuteScript(..) if legacy => base.join("execute"),
            WebDriverCommand::ExecuteScript(..) => base.join("execute/sync"),
            WebDriverCommand::SwitchToFrame(..) => base.join("frame"),
            WebDriverCommand::SwitchToParentFrame => base.join("frame/parent"),
            WebDriverCommand::SwitchToWindow(..) => base.join("window"),
            WebDriverCommand::GetElementProperty(ref we, ref prop) =>
                base.join(&format!("element/{}/property/{}", we.id, prop)),
            WebDriverCommand::GetElementAttribute(ref we, ref attr) =>
                base.join(&format!("element/{}/attribute/{}", we.id, attr)),
            WebDriverCommand::FindElementElement(ref p, _) => 
                base.join(&format!("element/{}/element", p.id)),
            WebDriverCommand::ElementClick(ref we) => 
                base.join(&format!("element/{}/click", we.id)),
            WebDriverCommand::GetElementText(ref we) => 
                base.join(&format!("element/{}/text", we.id)),
            WebDriverCommand::ElementSendKeys(ref we, _) => 
                base.join(&format!("element/{}/value", we.id)),
            _ => unimplemented!(),
        };
        Ok(endpoint?)
    }

    fn decode_error(
        &self, status: hyper::StatusCode, legacy_status: u64, body: Json
    ) -> Result<Error> {
        if !body.is_object() {
            bail!(ErrorKind::NotW3C(body))
        }
        let mut body = body.into_object().unwrap();
        // phantomjs injects a *huge* field with the entire
        // screen contents -- remove that
        body.remove("screen");
        let es = {
            if self.0.lock().unwrap().legacy {
                if !body.contains_key("message") || !body["message"].is_string() {
                    bail!(ErrorKind::NotW3C(Json::Object(body)));
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
                    _ => bail!(ErrorKind::NotW3C(Json::Object(body))),
                }
            } else {
                use hyper::StatusCode;
                let error = match body["error"].as_string() {
                    None => bail!(ErrorKind::NotW3C(Json::Object(body))),
                    Some(e) => e
                };
                if status == StatusCode::BAD_REQUEST {
                    match error {
                        "element click intercepted" =>
                            ErrorStatus::ElementClickIntercepted,
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
        let message = match body["message"].as_string() {
            None => bail!(ErrorKind::NotW3C(Json::Object(body))),
            Some(s) => String::from(s)
        };
        Ok(Error::from(ErrorKind::WebDriver(WebDriverError::new(es, message))))
    }

    #[async]
    pub(crate) fn issue_cmd(self, cmd: Cmd) -> Result<Json> {
        let url = self.endpoint_for(&cmd)?;
        let req = {
            let mut req = hyper::Request::builder();
            let (body, method) = encode_cmd(&cmd);
            req.method(method).uri(url.as_str());
            if let Some(ref s) = self.0.lock().unwrap().user_agent {
                req.header(hyper::header::USER_AGENT, s.as_str());
            }
            match body {
                None => req.body(hyper::Body::from(String::new()))?,
                Some(body) => {
                    req.header(hyper::header::CONTENT_TYPE, "application/json")
                        .header(hyper::header::CONTENT_LENGTH, body.len() as u64)
                        .body(hyper::Body::from(body))?
                }
            }
        };
        bail!("err");
        let res = await!(self.0.lock().unwrap().http_client.request(req))?;
        let status = res.status();
        let res_body = await!(res.body().concat2())?;
        match res.headers().get(hyper::header::CONTENT_TYPE) {
            None => bail!(ErrorKind::NotJson(res_body, None)),
            Some(ctype) =>
                if ctype != "application/json" {
                    let c = Some(String::from(ctype.to_str()?));
                    bail!(ErrorKind::NotJson(res_body, c));
                }
        }
        let is_new_session =
            if let WebDriverCommand::NewSession(..) = cmd { true } else { false };
        let (body, is_success, legacy_status) =
            match Json::from_str(from_utf8(&*res_body)?)? {
                Json::Object(mut v) => {
                    let legacy = self.0.lock().unwrap().legacy;
                    let mut is_success = status.is_success();
                    let mut legacy_status = 0;
                    if legacy {
                        legacy_status = v["status"].as_u64().unwrap();
                        is_success = legacy_status == 0;
                    }
                    if legacy && is_new_session {
                        (Json::Object(v), is_success, legacy_status)
                    } else {
                        let body = v.remove("value").ok_or_else(|| {
                            Error::from(ErrorKind::NotW3C(Json::Object(v)))
                        })?;
                        (body, is_success, legacy_status)
                    }
                }
                v => bail!(ErrorKind::NotW3C(v)),
            };
        if is_success { Ok(body) }
        else { Err(self.decode_error(status, legacy_status, body)?) }
    }
}
