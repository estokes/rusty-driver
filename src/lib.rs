//! A high-level API for programmatically interacting with web pages
//! through WebDriver.
//!
//! [WebDriver protocol]: https://www.w3.org/TR/webdriver/
//! [CSS selectors]: https://developer.mozilla.org/en-US/docs/Web/CSS/CSS_Selectors
//! [powerful]: https://developer.mozilla.org/en-US/docs/Web/CSS/Pseudo-classes
//! [operators]: https://developer.mozilla.org/en-US/docs/Web/CSS/Attribute_selectors
//! [WebDriver compatible]: https://github.com/Fyrd/caniuse/issues/2757#issuecomment-304529217
//! [`geckodriver`]: https://github.com/mozilla/geckodriver
#![feature(use_extern_macros, proc_macro_non_items, generators, nll)]

extern crate futures_await as futures;
extern crate http;
extern crate hyper;
extern crate hyper_tls;
extern crate rustc_serialize;
extern crate tokio;
extern crate tokio_timer;
extern crate url;
extern crate webdriver;
#[macro_use] extern crate error_chain;

pub mod error;
mod protocol;

use std::time::Duration;
use tokio_timer::sleep;
use webdriver::{
    command::{WebDriverCommand, SwitchToFrameParameters, SwitchToWindowParameters},
    error::{WebDriverError, ErrorStatus}, common::{ELEMENT_KEY, FrameId, WebElement}
};
use rustc_serialize::json::{ToJson, Json};
use futures::prelude::await;
use futures::prelude::*;
pub use hyper::Method;
use protocol::Client;
use error::*;

#[derive(Clone, Eq, PartialEq, Ord, PartialOrd, Debug, Hash)]
pub enum Locator {
    Css(String),
    LinkText(String),
    XPath(String),
}

impl Into<webdriver::command::LocatorParameters> for Locator {
    fn into(self) -> webdriver::command::LocatorParameters {
        match self {
            Locator::Css(s) => webdriver::command::LocatorParameters {
                using: webdriver::common::LocatorStrategy::CSSSelector,
                value: s,
            },
            Locator::XPath(s) => webdriver::command::LocatorParameters {
                using: webdriver::common::LocatorStrategy::XPath,
                value: s,
            },
            Locator::LinkText(s) => webdriver::command::LocatorParameters {
                using: webdriver::common::LocatorStrategy::LinkText,
                value: s,
            },
        }
    }
}

#[derive(Clone)]
pub struct Driver(Client);

macro_rules! generate_wait_for_find {
    ($name:ident, $search_fn:ident, $return_typ:ty) => {
        /// Wait for the specified element(s) to appear on the page
        #[async]
        pub fn $name(
            self,
            search: Locator,
            root: Option<WebElement>
        ) -> Result<$return_typ> {
            loop {
                match await!(self.clone().$search_fn(search.clone(), root.clone())) {
                    Ok(e) => break Ok(e),
                    Err(Error(ErrorKind::WebDriver(
                        WebDriverError {error: ErrorStatus::NoSuchElement, ..}
                    ), _)) => {
                        await!(sleep(Duration::from_millis(100)))?;
                    },
                    Err(e) => break Err(e)
                }
            }
        }
    }
}

impl Driver {
    /// Create a new webdriver session on the specified server
    #[async]
    pub fn new(webdriver_url: String, user_agent: Option<String>) -> Result<Self> {
        Ok(Driver(await!(Client::new(webdriver_url, user_agent))?))
    }

    /// Navigate directly to the given URL.
    #[async]
    pub fn goto(self, url: String) -> Result<()> {
        let cmd = WebDriverCommand::Get(webdriver::command::GetParameters {
            url: await!(self.clone().current_url())?.join(&url)?.into_string(),
        });
        await!(self.0.issue_cmd(cmd))?;
        Ok(())
    }

    /// Retrieve the currently active URL for this session.
    #[async]
    pub fn current_url(self) -> Result<url::Url> {
        match await!(self.0.issue_cmd(WebDriverCommand::GetCurrentUrl))?.as_string() {
            Some(url) => Ok(url.parse()?),
            None => bail!(ErrorKind::NotW3C(Json::Null))
        }
    }

    /// Get the HTML source for the current page.
    #[async]
    pub fn source(self) -> Result<String> {
        match await!(self.0.issue_cmd(WebDriverCommand::GetPageSource))?.as_string() {
            Some(src) => Ok(src.to_string()),
            None => bail!(ErrorKind::NotW3C(Json::Null))
        }
    }

    /// Go back to the previous page.
    #[async]
    pub fn back(self) -> Result<()> {
        await!(self.0.issue_cmd(WebDriverCommand::GoBack))?;
        Ok(())
    }

    /// Refresh the current previous page.
    #[async]
    pub fn refresh(self) -> Result<()> {
        await!(self.0.issue_cmd(WebDriverCommand::Refresh))?;
        Ok(())
    }

    /// Switch the focus to the frame contained in Element
    #[async]
    pub fn switch_to_frame(self, frame: WebElement) -> Result<()> {
        let p = SwitchToFrameParameters {id: FrameId::Element(frame)};
        await!(self.0.issue_cmd(WebDriverCommand::SwitchToFrame(p)))?;
        Ok(())
    }

    /// Switch the focus to this frame's parent frame
    #[async]
    pub fn switch_to_parent_frame(self) -> Result<()> {
        await!(self.0.issue_cmd(WebDriverCommand::SwitchToParentFrame))?;
        Ok(())
    }

    /// Switch the focus to the window identified by handle
    #[async]
    pub fn switch_to_window(self, window: String) -> Result<()> {
        let p = SwitchToWindowParameters {handle: window};
        await!(self.0.issue_cmd(WebDriverCommand::SwitchToWindow(p)))?;
        Ok(())
    }

    /// Execute the given JavaScript `script` in the current browser session.
    ///
    /// `args` is available to the script inside the `arguments`
    /// array. Since `Element` implements `ToJson`, you can also
    /// provide serialized `Element`s as arguments, and they will
    /// correctly serialize to DOM elements on the other side.
    #[async]
    pub fn execute(self, script: String, mut args: Vec<Json>) -> Result<Json> {
        self.fixup_elements(&mut args);
        let cmd = webdriver::command::JavascriptCommandParameters {
            script: script,
            args: webdriver::common::Nullable::Value(args),
        };
        await!(self.0.issue_cmd(WebDriverCommand::ExecuteScript(cmd)))
    }

    /// Wait for the page to navigate to a new URL before proceeding.
    ///
    /// If the `current` URL is not provided, `self.current_url()`
    /// will be used. Note however that this introduces a race
    /// condition: the browser could finish navigating *before* we
    /// call `current_url()`, which would lead to an eternal wait.
    #[async]
    pub fn wait_for_navigation(self, current: Option<url::Url>) -> Result<()> {
        let current = match current {
            Some(current) => current,
            None => await!(self.clone().current_url())?,
        };
        loop {
            if await!(self.clone().current_url())? != current { break Ok(()) }
            await!(sleep(Duration::from_millis(100)))?
        }
    }

    /// Starting from the document root, find the first element on the page that
    /// matches the specified selector.
    #[async]
    pub fn find(self, locator: Locator, root: Option<WebElement>) -> Result<WebElement> {
        let cmd = match root {
            Option::None => WebDriverCommand::FindElement(locator.into()),
            Option::Some(elt) => WebDriverCommand::FindElementElement(elt, locator.into())
        };
        let res = await!(self.0.clone().issue_cmd(cmd))?;
        Ok(self.parse_lookup(res)?)
    }

    #[async]
    pub fn find_all(
        self,
        locator: Locator,
        root: Option<WebElement>
    ) -> Result<Vec<WebElement>> {
        let cmd = match root {
            Option::None => WebDriverCommand::FindElements(locator.into()),
            Option::Some(elt) => WebDriverCommand::FindElementElements(elt, locator.into())
        };
        match await!(self.0.clone().issue_cmd(cmd))? {
            Json::Array(a) => Ok(
                a.into_iter().map(|e| self.parse_lookup(e))
                    .collect::<Result<Vec<WebElement>>>()?
            ),
            r => bail!(ErrorKind::NotW3C(r))
        }
    }

    generate_wait_for_find!(wait_for_find, find, WebElement);
    generate_wait_for_find!(wait_for_find_all, find_all, Vec<WebElement>);

    /// Extract the `WebElement` from a `FindElement` or `FindElementElement` command.
    fn parse_lookup(&self, res: Json) -> Result<WebElement> {
        let key = if self.0.legacy { "ELEMENT" } else { ELEMENT_KEY };
        let mut res = {
            if !res.is_object() { bail!(ErrorKind::NotW3C(res)) }
            else { res.into_object().unwrap() }
        };
        match res.remove(key) {
            None => bail!(ErrorKind::NotW3C(Json::Object(res))),
            Some(Json::String(wei)) => Ok(webdriver::common::WebElement::new(wei)),
            Some(v) => {
                res.insert(key.to_string(), v);
                bail!(ErrorKind::NotW3C(Json::Object(res)))
            }
        }
    }

    fn fixup_elements(&self, args: &mut [Json]) {
        if self.0.legacy {
            for arg in args {
                // the serialization of WebElement uses the W3C index,
                // but legacy implementations need us to use the "ELEMENT" index
                if let Json::Object(ref mut o) = *arg {
                    if let Some(wei) = o.remove(ELEMENT_KEY) {
                        o.insert("ELEMENT".to_string(), wei);
                    }
                }
            }
        }
    }

    /// Look up an attribute value for this element by name.
    #[async]
    pub fn attr(self, eid: WebElement, attribute: String) -> Result<Option<String>> {
        let cmd = WebDriverCommand::GetElementAttribute(eid, attribute);
        match await!(self.0.clone().issue_cmd(cmd))? {
            Json::String(v) => Ok(Some(v)),
            Json::Null => Ok(None),
            v => bail!(ErrorKind::NotW3C(v)),
        }
    }

    /// Look up a DOM property for this element by name.
    #[async]
    pub fn prop(self, eid: WebElement, prop: String) -> Result<Option<String>> {
        let cmd = WebDriverCommand::GetElementProperty(eid, prop);
        match await!(self.0.clone().issue_cmd(cmd))? {
            Json::String(v) => Ok(Some(v)),
            Json::Null => Ok(None),
            v => bail!(ErrorKind::NotW3C(v)),
        }
    }

    /// Retrieve the text contents of this elment.
    #[async]
    pub fn text(self, eid: WebElement) -> Result<String> {
        let cmd = WebDriverCommand::GetElementText(eid);
        match await!(self.0.clone().issue_cmd(cmd))? {
            Json::String(v) => Ok(v),
            v => bail!(ErrorKind::NotW3C(v)),
        }
    }

    /// Retrieve the HTML contents of this element. if inner is true,
    /// also return the wrapping nodes html. Note: this is the same as
    /// calling `prop("innerHTML")` or `prop("outerHTML")`.
    #[async]
    pub fn html(self, eid: WebElement, inner: bool) -> Result<String> {
        let prop = if inner { "innerHTML" } else { "outerHTML" };
        await!(self.prop(eid, prop.to_owned()))?
            .ok_or_else(|| Error::from(ErrorKind::NotW3C(Json::Null)))
    }

    /// Click on this element
    #[async]
    pub fn click(self, eid: WebElement) -> Result<()> {
        let cmd = WebDriverCommand::ElementClick(eid);
        let r = await!(self.0.clone().issue_cmd(cmd))?;
        if r.is_null() || r.as_object().map(|o| o.is_empty()).unwrap_or(false) {
            // geckodriver returns {} :(
            Ok(())
        } else {
            bail!(ErrorKind::NotW3C(r))
        }
    }

    /// Scroll this element into view
    #[async]
    pub fn scroll_into_view(self, eid: WebElement) -> Result<()> {
        let args = vec![eid.to_json()];
        let js = "arguments[0].scrollIntoView(true)".to_string();
        await!(self.clone().execute(js, args))?;
        Ok(())
    }

    /// Follow the `href` target of the element matching the given CSS
    /// selector *without* causing a click interaction.
    #[async]
    pub fn follow(self, eid: WebElement) -> Result<()> {
        match await!(self.clone().attr(eid.clone(), String::from("href")))? {
            None => bail!("no href attribute"),
            Some(href) => {
                let current = await!(self.clone().current_url())?;
                await!(self.clone().goto(current.join(&href)?.into_string()))
            }
        }
    }

    /// Set the `value` of the input element named `name` which is a child of `eid`
    #[async]
    pub fn set_by_name(self, eid: WebElement, name: String, value: String) -> Result<()> {
        let locator = Locator::Css(format!("input[name='{}']", name));
        let elt = await!(self.clone().find(locator.into(), Some(eid)))?;
        use rustc_serialize::json::ToJson;
        let args = {
            let mut a = vec![elt.to_json(), Json::String(value)];
            self.fixup_elements(&mut a);
            a
        };
        let js = "arguments[0].value = arguments[1]".to_string();
        let res = await!(self.clone().execute(js, args))?;
        if res.is_null() { Ok(()) } else { bail!(ErrorKind::NotW3C(res)) }
    }

    /// Submit the form specified by `eid` with the first submit button
    #[async]
    pub fn submit(self, eid: WebElement) -> Result<()> {
        let l = Locator::Css("input[type=submit],button[type=submit]".into());
        await!(self.submit_with(eid, l))
    }

    /// Submit the form `eid` using the button matched by the given selector.
    #[async]
    pub fn submit_with(self, eid: WebElement, button: Locator) -> Result<()> {
        let elt = await!(self.clone().find(button.into(), Some(eid)))?;
        Ok(await!(self.clone().click(elt))?)
    }

    /// Submit this form using the form submit button with the given
    /// label (case-insensitive).
    #[async]
    pub fn submit_using(self, eid: WebElement, button_label: String) -> Result<()> {
        let escaped = button_label.replace('\\', "\\\\").replace('"', "\\\"");
        let btn = format!(
            "input[type=submit][value=\"{}\" i],\
             button[type=submit][value=\"{}\" i]",
            escaped, escaped
        );
        Ok(await!(self.submit_with(eid, Locator::Css(btn)))?)
    }

    /// Submit this form directly, without clicking any buttons.
    ///
    /// This can be useful to bypass forms that perform various magic
    /// when the submit button is clicked, or that hijack click events
    /// altogether.
    ///
    /// Note that since no button is actually clicked, the
    /// `name=value` pair for the submit button will not be
    /// submitted. This can be circumvented by using `submit_sneaky`
    /// instead.
    #[async]
    pub fn submit_direct(self, eid: WebElement) -> Result<()> {
        // some sites are silly, and name their submit button
        // "submit". this ends up overwriting the "submit" function of
        // the form with a reference to the submit button itself, so
        // we can't call .submit(). we get around this by creating a
        // *new* form, and using *its* submit() handler but with this
        // pointed to the real form. solution from here:
        // https://stackoverflow.com/q/833032/472927#comment23038712_834197
        use rustc_serialize::json::ToJson;
        let js = "document.createElement('form').submit.call(arguments[0])".to_string();
        let args = {
            let mut a = vec![eid.to_json()];
            self.fixup_elements(&mut a);
            a
        };
        await!(self.clone().execute(js, args))?;
        Ok(())
    }

    /// Submit this form directly, without clicking any buttons, and
    /// with an extra field.
    ///
    /// Like `submit_direct`, this method will submit this form
    /// without clicking a submit button.  However, it will *also*
    /// inject a hidden input element on the page that carries the
    /// given `field=value` mapping. This allows you to emulate the
    /// form data as it would have been *if* the submit button was
    /// indeed clicked.
    #[async]
    pub fn submit_sneaky(
        self,
        eid: WebElement,
        field: String,
        value: String
    ) -> Result<()> {
        use rustc_serialize::json::ToJson;
        let js = r#"
            var h = document.createElement('input');
            h.setAttribute('type', 'hidden');
            h.setAttribute('name', arguments[1]);
            h.value = arguments[2];
            arguments[0].appendChild(h);
        "#.to_string();
        let args = {
            let mut a = vec![
                eid.to_json(),
                Json::String(field),
                Json::String(value),
            ];
            self.fixup_elements(&mut a);
            a
        };
        await!(self.execute(js, args))?;
        Ok(())
    }
}
