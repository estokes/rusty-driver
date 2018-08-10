//! A high-level API for programmatically interacting with web pages
//! through WebDriver.
//!
//! This crate uses the [WebDriver protocol] to drive a conforming
//! (potentially headless) browser through relatively high-level
//! operations such as "click this element", "submit this form", etc.
//! It is currently nightly-only, but this will change once
//! [`conservative_impl_trait`](https://github.com/rust-lang/rust/issues/34511)
//! lands in stable.
//!
//! Most interactions are driven by using [CSS selectors]. With most
//! WebDriver-compatible browser being fairly recent, the more
//! expressive levels of the CSS standard are also supported, giving
//! fairly [powerful] [operators].
//!
//! Forms are managed by first calling `Client::form`, and then using
//! the methods on `Form` to manipulate the form's fields and
//! eventually submitting it.
//!
//! For low-level access to the page, `Client::source` can be used to
//! fetch the full page HTML source code, and `Client::raw_client_for`
//! to build a raw HTTP request for a particular URL.
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

mod error;
mod protocol;

/*
use std::{collections::HashMap, time::Duration, sync::{Arc, Mutex}};
use tokio::spawn;
use tokio_timer::sleep;
use webdriver::{
    command::{WebDriverCommand, SwitchToFrameParameters, SwitchToWindowParameters},
    error::{WebDriverError, ErrorStatus}, common::{ELEMENT_KEY, FrameId}
};
use rustc_serialize::json::{ToJson, Json};
use futures::prelude::await;
use futures::prelude::*;
pub use hyper::Method;
use error::*;

/// An element locator.
///
/// See <https://www.w3.org/TR/webdriver/#element-retrieval>.
#[derive(Clone, Eq, PartialEq, Ord, PartialOrd, Debug, Hash)]
pub enum Locator {
    /// Find an element matching the given CSS selector.
    Css(String),

    /// Find a link element with the given link text.
    ///
    /// The text matching is exact.
    LinkText(String),

    /// Find an element using the given XPath expression.
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


/// A single element on the current page.
#[derive(Clone)]
pub struct Element {
    client: Client,
    eid: webdriver::common::WebElement,
}

/// An HTML form on the current page.
#[derive(Clone)]
pub struct Form {
    client: Client,
    eid: webdriver::common::WebElement,
}

macro_rules! generate_wait_for_find {
    ($name:ident, $search_fn:ident, $return_typ:ty) => {
        /// Wait for the specified element(s) to appear on the page
        #[async]
        pub fn $name(self, search: Locator) -> Result<$return_typ> {
            loop {
                match await!(self.clone().$search_fn(search.clone())) {
                    Ok(e) => break Ok(e),
                    Err(Error(ErrorKind::WebDriver {
                        error: ErrorStatus::NoSuchElement, ..
                    }, _)) => { await!(sleep(Duration::from_ms(100)))?; },
                    Err(e) => break Err(e)
                }
            }
        }
    }
}

impl Client {
    /// Set the User Agent string to use for all subsequent requests.
    pub fn set_ua<S: Into<String>>(&mut self, ua: S) {
        *self.0.ua.borrow_mut() = Some(ua.into());
    }

    /// Terminate the connection to the webservice.
    ///
    /// Normally, a shutdown of the WebDriver connection will be
    /// initiated when the last clone of a `Client` is
    /// dropped. Specifically, the shutdown request will be issued
    /// using the tokio `Handle` given when creating this
    /// `Client`. This in turn means that any errors will be dropped,
    /// and that the teardown may not even occur if the reactor does
    /// not continue being turned.
    ///
    /// This function is safe to call multiple times, but once it has
    /// been called on one instance of a `Client`, all requests to
    /// other instances of that `Client` will fail. The returned
    /// `Option` will only be true the first time `close` is called.
    ///
    /// This function may be useful in conjunction with
    /// `raw_client_for`, as it allows you to close the automated
    /// browser window while doing e.g., a large download.
    pub fn close(self) { self.0.shutdown() }

    /// Navigate directly to the given URL.
    #[async]
    pub fn goto(self, url: String) -> Result<()> {
        let cmd = WebDriverCommand::Get(webdriver::command::GetParameters {
            url: await!(self.clone().current_url())?.join(&url)?.into_string(),
        });
        await!(self.issue_wd_cmd(cmd))?;
        Ok(())
    }

    /// Retrieve the currently active URL for this session.
    #[async]
    pub fn current_url(self) -> Result<url::Url> {
        match await!(self.issue_wd_cmd(WebDriverCommand::GetCurrentUrl))?.as_string() {
            Some(url) => Ok(url.parse()?),
            None => bail!(ErrorKind::NotW3C(url))
        }
    }

    /// Get the HTML source for the current page.
    #[async]
    pub fn source(self) -> Result<String> {
        match await!(self.issue_wd_cmd(WebDriverCommand::GetPageSource))?.as_string() {
            Some(src) => Ok(src.to_string()),
            None => bail!(ErrorKind::NotW3C(src))
        }
    }

    /// Go back to the previous page.
    #[async]
    pub fn back(self) -> Result<()> {
        await!(self.issue_wd_cmd(WebDriverCommand::GoBack))?;
        Ok(())
    }

    /// Refresh the current previous page.
    #[async]
    pub fn refresh(self) -> Result<()> {
        await!(self.issue_wd_cmd(WebDriverCommand::Refresh))?;
        Ok(())
    }

    /// Switch the focus to the frame contained in Element
    #[async]
    pub fn switch_to_frame(self, frame: Element) -> Result<()> {
        let p = SwitchToFrameParameters {id: FrameId::Element(frame.e)};
        await!(self.issue_wd_cmd(WebDriverCommand::SwitchToFrame(p)))?;
        Ok(())
    }

    /// Switch the focus to this frame's parent frame
    #[async]
    pub fn switch_to_parent_frame(self) -> Result<()> {
        await!(self.issue_wd_cmd(WebDriverCommand::SwitchToParentFrame))?;
        Ok(())
    }

    /// Switch the focus to the window identified by handle
    #[async]
    pub fn switch_to_window(self, window: String) -> Result<()> {
        let p = SwitchToWindowParameters {handle: window};
        await!(self.issue_wd_cmd(WebDriverCommand::SwitchToWindow(p)))?;
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
        await!(self.issue_wd_cmd(WebDriverCommand::ExecuteScript(cmd)))
    }

    /// Issue an HTTP request to the given `url` with all the same
    /// cookies as the current session.
    ///
    /// Calling this method is equivalent to calling
    /// `with_raw_client_for` with an empty closure.
    #[async]
    pub fn raw_client_for<U>(
        self, method: Method, url: String
    ) -> Result<hyper::Response<U>> {
        await!(self.with_raw_client_for(method, url, |r| r.body(String::new())))
    }

    /// Build and issue an HTTP request to the given `url` with all
    /// the same cookies as the current session.
    ///
    /// Before the HTTP request is issued, the given `before` closure
    /// will be called with a handle to the `Request` about to be
    /// sent.
    #[async]
    pub fn with_raw_client_for<T, U, F>(
        self,
        method: Method,
        url: String,
        before: F,
    ) -> Result<hyper::Response<U>>
    where F: FnOnce(http::request::Builder) -> http::Result<http::request::Request<T>>,
    {
        // We need to do some trickiness here. GetCookies will only
        // give us the cookies for the *current* domain, whereas we
        // want the cookies for `url`'s domain. So, we navigate to the
        // URL in question, fetch its cookies, and then navigate
        // back. *Except* that we can't do that either (what if `url`
        // is some huge file?). So we *actually* navigate to some
        // weird url that's unlikely to exist on the target doamin,
        // and which won't resolve into the actual content, but will
        // still give the same cookies.
        //
        // The fact that cookies can have /path and security
        // constraints makes this even more of a pain. /path in
        // particular is tricky, because you could have a URL like:
        //
        //    example.com/download/some_identifier/ignored_filename_just_for_show
        //
        // Imagine if a cookie is set with
        // path=/download/some_identifier. How do we get that cookie
        // without triggering a request for the (large) file? I don't
        // know. Hence: TODO.
        let old_url = await!(self.clone().current_url())?;
        let url = old_url.join(&url);
        let cookie_url = url.clone().join("/please_give_me_your_cookies");
        await!(self.clone().goto(cookie_url.to_string()))?;
        let cookies = {
            let c = await!(self.clone().issue_wd_cmd(WebDriverCommand::GetCookies))?;
            if !c.is_array() { bail!(ErrorKind::NotW3C(cookies)); }
            else { cookies.into_array().unwrap() }
        };
        await!(self.clone().back())?;
        // now add all the cookies
        let mut jar = HashMap::new();
        for cookie in &cookies {
            if !cookie.is_object() { bail!(ErrorKind::NotW3C(Json::Array(cookies))); }

            // https://w3c.github.io/webdriver/webdriver-spec.html#cookies
            let cookie = cookie.as_object().unwrap();
            if !cookie.contains_key("name") || !cookie.contains_key("value") ||
                !cookie["name"].is_string() || !cookie["value"].is_string()
            {
                bail!(ErrorKind::NotW3C(Json::Array(cookies)));
            }

            // Note that since we're sending these cookies, all that matters is the mapping
            // from name to value. The other fields only matter when deciding whether to
            // include a cookie or not, and the driver has already decided that for us
            // (GetCookies is for a particular URL).
            jar.insert(
                cookie["name"].as_string().unwrap().to_owned(),
                cookie["value"].as_string().unwrap().to_owned(),
            );
        }

        let mut req =
            hyper::Request::builder().method(method)
            .uri(url.as_ref().parse().unwrap());
        for (name, value) in jar.drain(0..) {
            req = req.header(hyper::header::COOKIE, format!("{}={}", name, value));
        }
        if let Some(ref s) = *self.0.ua.borrow() {
            req = req.header(hyper::header::USER_AGENT, s.to_owned());
        }
        Ok(await!(self.0.c.request(before(req)?))?)
    }

    /// Starting from the document root, find the first element on the page that
    /// matches the specified selector.
    #[async]
    pub fn find(self, search: Locator) -> Result<Element> {
        await!(self.by(search.into(), None))?
    }

    /// Find all elements on the page that match the specified selector.
    #[async]
    pub fn find_all(self, search: Locator) -> Result<Vec<Element>> {
        await!(self.by_all(search.into(), None))?
    }

    generate_wait_for_find!(wait_for_find, find, Element);
    generate_wait_for_find!(wait_for_find_all, find_all, Vec<Element>);

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
            await!(sleep(Duration::from_ms(100)))?
        }
    }    

    /// Locate a form on the page.
    ///
    /// Through the returned `Form`, HTML forms can be filled out and submitted.
    #[async]
    pub fn form(self, search: Locator) -> Result<Form> {
        let cmd = WebDriverCommand::FindElement(search.into());
        let res = await!(self.issue_wd_cmd(cmd))?;
        let f = self.parse_lookup(res);
        Ok(Form { c: self.clone(), f })
    }

    // helpers

    #[async]
    fn by(
        self,
        locator: webdriver::command::LocatorParameters,
        root: Option<webdriver::common::WebElement>,
    ) -> Result<Element> {
        let cmd = match root {
            Option::None => WebDriverCommand::FindElement(locator),
            Option::Some(elt) => WebDriverCommand::FindElementElement(elt, locator)
        };
        let res = await!(self.clone().issue_wd_cmd(cmd))?;
        let e = self.parse_lookup(r)?;
        Ok(Element { c: self.clone(), e })
    }

    #[async]
    fn by_all(
        self,
        locator: webdriver::command::LocatorParameters,
        root: Option<webdriver::common::WebElement>,
    ) -> Result<Vec<Element>> {
        let cmd = match root {
            Option::None => WebDriverCommand::FindElements(locator),
            Option::Some(elt) => WebDriverCommand::FindElementElements(elt, locator)
        };
        match await!(self.clone().issue_wd_cmd(cmd))? {
            Json::Array(a) => {
                Ok(a.into_iter()
                   .map(|e| Ok(Element {c: self.clone(), e: self.parse_lookup(e)?}))
                   .collect::<Result<Vec<Element>>>()?)
            },
            r => bail!(ErrorKind::NotW3C(r))
        }
    }

    /// Extract the `WebElement` from a `FindElement` or `FindElementElement` command.
    fn parse_lookup(&self, res: Json) -> Result<webdriver::common::WebElement> {
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
}

impl Element {
    /// Look up an [attribute] value for this element by name.
    ///
    /// `Ok(None)` is returned if the element does not have the given attribute.
    ///
    /// [attribute]: https://dom.spec.whatwg.org/#concept-attribute
    #[async]
    pub fn attr(self, attribute: String) -> Result<Option<String>> {
        let cmd = WebDriverCommand::GetElementAttribute(self.e, attribute.to_string());
        match await!(self.clone().c.issue_wd_cmd(cmd))? {
            Json::String(v) => Ok(Some(v)),
            Json::Null => Ok(None),
            v => bail!(ErrorKind::NotW3C(v)),
        }
    }

    /// Look up a DOM [property] for this element by name.
    ///
    /// `Ok(None)` is returned if the element does not have the given property.
    ///
    /// [property]: https://www.ecma-international.org/ecma-262/5.1/#sec-8.12.1
    #[async]
    pub fn prop(self, prop: &str) -> Result<Option<String>> {
        let cmd = WebDriverCommand::GetElementProperty(self.e, prop.to_string());
        match await!(self.c.issue_wd_cmd(cmd))? {
            Json::String(v) => Ok(Some(v)),
            Json::Null => Ok(None),
            v => bail!(ErrorKind::NotW3C(v)),
        }
    }

    /// Retrieve the text contents of this elment.
    #[async]
    pub fn text(self) -> Result<String> {
        let cmd = WebDriverCommand::GetElementText(self.e);
        match await!(self.c.issue_wd_cmd(cmd))? {
            Json::String(v) => Ok(v),
            v => bail!(ErrorKind::NotW3C(v)),
        }
    }

    /// Retrieve the HTML contents of this element.
    ///
    /// `inner` dictates whether the wrapping node's HTML is excluded
    /// or not. For example, take the HTML:
    ///
    /// ```html
    /// <div id="foo"><hr /></div>
    /// ```
    ///
    /// With `inner = true`, `<hr />` would be returned. With `inner = false`,
    /// `<div id="foo"><hr /></div>` would be returned instead.
    #[async]
    pub fn html(self, inner: bool) -> Result<String> {
        let prop = if inner { "innerHTML" } else { "outerHTML" };
        await!(self.prop(prop))?
            .ok_or_else(|| Error::from(ErrorKind::NotW3C(Json::Null)))
    }

    /// Simulate the user clicking on this element.
    ///
    /// Note that since this *may* result in navigation, we give up
    /// the handle to the element.
    #[async]
    pub fn click(self) -> Result<()> {
        let cmd = WebDriverCommand::ElementClick(self.e);
        let r = await!(self.c.issue_wd_cmd(cmd))?;
        if r.is_null() || r.as_object().map(|o| o.is_empty()).unwrap_or(false) {
            // geckodriver returns {} :(
            Ok(())
        } else {
            bail!(ErrorKind::NotW3C(r))
        }
    }

    /// Find the child element matching the specified criteria
    #[async]
    pub fn find(self, search: Locator) -> Result<Self> {
        await!(self.c.by(search.into(), Some(self.e)))
    }

    /// Find the child elements matching the specified criteria
    #[async]
    pub fn find_all(self, search: Locator) -> Result<Vec<Self>> {
        await!(self.c.by_all(search.into(), Some(self.e)))
    }

    /// Scroll this element into view
    #[async]
    pub fn scroll_into_view(self) -> Result<()> {
        let args = vec![self.e.to_json()];
        let js = "arguments[0].scrollIntoView(true)".to_string();
        await!(self.c.execute(js, args))?;
        Ok(())
    }

    generate_wait_for_find!(wait_for_find, find, Self);
    generate_wait_for_find!(wait_for_find_all, find_all, Vec<Self>);

    /// Follow the `href` target of the element matching the given CSS
    /// selector *without* causing a click interaction.
    #[async]
    pub fn follow(self) -> Result<()> {
        match await!(self.clone().attr(String::from("href")))? {
            None => bail!("no href attribute"),
            Some(href) => {
                let current = await!(self.c.clone().current_url())?;
                await!(self.c.goto(current.join(&href)?.into_string()))?
            }
        }
    }
}

impl rustc_serialize::json::ToJson for Element {
    fn to_json(&self) -> Json {
        self.e.to_json()
    }
}

impl Form {
    /// Set the `value` of the given `field` in this form.
    #[async]
    pub fn set_by_name(self, field: String, value: String) -> Result<()> {
        let locator = Locator::Css(format!("input[name='{}']", field));
        let elt = await!(self.c.clone().by(locator, Some(self.f)))?;
        use rustc_serialize::json::ToJson;
        let args = {
            let mut a = vec![elt.e.to_json(), Json::String(value)];
            self.c.fixup_elements(&mut a);
            a
        };
        let js = "arguments[0].value = arguments[1]".to_string();
        let res = await!(self.c.execute(js, args))?;
        if res.is_null() { Ok(()) } else { bail!(ErrorKind::NotW3C(res)) }
    }

    /// Submit this form using the first available submit button.
    #[async]
    pub fn submit(self) -> Result<()> {
        let l = Locator::Css("input[type=submit],button[type=submit]".into());
        await!(self.submit_with(l))
    }

    /// Submit this form using the button matched by the given selector.
    #[async]
    pub fn submit_with(self, button: Locator) -> Result<()> {
        let elt = await!(self.c.clone().by(button, Some(self.f)))?;
        Ok(await!(elt.click())?)
    }

    /// Submit this form using the form submit button with the given
    /// label (case-insensitive).
    pub fn submit_using(self, button_label: &str) -> Result<()> {
        let escaped = button_label.replace('\\', "\\\\").replace('"', "\\\"");
        let btn = format!(
            "input[type=submit][value=\"{}\" i],\
             button[type=submit][value=\"{}\" i]",
            escaped, escaped
        );
        Ok(await!(self.submit_with(Locator::Css(btn)))?)
    }

    /// Submit this form directly, without clicking any buttons.
    ///
    /// This can be useful to bypass forms that perform various magic
    /// when the submit button is clicked, or that hijack click events
    /// altogether (yes, I'm looking at you online advertisement
    /// code).
    ///
    /// Note that since no button is actually clicked, the
    /// `name=value` pair for the submit button will not be
    /// submitted. This can be circumvented by using `submit_sneaky`
    /// instead.
    #[async]
    pub fn submit_direct(self) -> Result<()> {
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
            let a = vec![self.f.to_json()];
            self.c.fixup_elements(&mut a);
            a
        };
        Ok(await!(self.c.execute(js, args))?)
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
    pub fn submit_sneaky(self, field: String, value: String) -> Result<()> {
        use rustc_serialize::json::ToJson;
        let js =
            r#"var h = document.createElement('input');
               h.setAttribute('type', 'hidden');
               h.setAttribute('name', arguments[1]);
               h.value = arguments[2];
               arguments[0].appendChild(h);"#.to_string();
        let args = {
            let mut a = vec![
                self.f.to_json(),
                Json::String(field.to_string()),
                Json::String(value.to_string()),
            ];
            self.c.fixup_elements(&mut a);
            a
        };
        Ok(await!(self.c.execute(js, args))?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio_core::reactor::Core;

    macro_rules! tester {
        ($f:ident) => {{
            let mut core = Core::new().unwrap();
            let h = core.handle();
            let c = Client::new("http://localhost:4444", &h);
            let c = core.run(c)
                .expect("failed to construct test client");
            core.run($f(&c))
                .expect("test produced unexpected error response");
            let fin = c.close();
            core.run(fin).expect("failed to close test session");
        }}
    }

    fn works_inner<'a>(c: &'a Client) -> impl Future<Item = (), Error = error::CmdError> + 'a {
        // go to the Wikipedia page for Foobar
        c.goto("https://en.wikipedia.org/wiki/Foobar")
            .and_then(move |_| c.current_url())
            .and_then(move |(this, url)| {
                assert_eq!(url.as_ref(), "https://en.wikipedia.org/wiki/Foobar");
                // click "Foo (disambiguation)"
                c.find(Locator::Css(".mw-disambig"))
            })
            .and_then(|e| e.click())
            .and_then(move |_| {
                // click "Foo Lake"
                c.find(Locator::LinkText("Foo Lake"))
            })
            .and_then(|e| e.click())
            .and_then(move |_| c.current_url())
            .and_then(|url| {
                assert_eq!(url.as_ref(), "https://en.wikipedia.org/wiki/Foo_Lake");
                Ok(())
            })
    }

    #[test]
    #[ignore]
    fn it_works() {
        tester!(works_inner)
    }

    fn clicks_inner<'a>(c: &'a Client) -> impl Future<Item = (), Error = error::CmdError> + 'a {
        // go to the Wikipedia frontpage this time
        c.goto("https://www.wikipedia.org/")
            .and_then(move |_| {
                // find, fill out, and submit the search form
                c.form(Locator::Css("#search-form"))
            })
            .and_then(|f| f.set_by_name("search", "foobar"))
            .and_then(|f| f.submit())
            .and_then(move |_| c.current_url())
            .and_then(|url| {
                // we should now have ended up in the rigth place
                assert_eq!(url.as_ref(), "https://en.wikipedia.org/wiki/Foobar");
                Ok(())
            })
    }

    #[test]
    #[ignore]
    fn it_clicks() {
        tester!(clicks_inner)
    }

    fn raw_inner<'a>(c: &'a Client) -> impl Future<Item = (), Error = error::CmdError> + 'a {
        // go back to the frontpage
        c.goto("https://www.wikipedia.org/")
            .and_then(move |_| {
                // find the source for the Wikipedia globe
                c.find(Locator::Css("img.central-featured-logo"))
            })
            .and_then(|img| {
                img.attr("src")
                    .map(|src| src.expect("image should have a src"))
            })
            .and_then(move |src| {
                // now build a raw HTTP client request (which also has all current cookies)
                c.raw_client_for(Method::Get, &src)
            })
            .and_then(|raw| {
                // we then read out the image bytes
                raw.body()
                    .map_err(error::CmdError::from)
                    .fold(Vec::new(), |mut pixels, chunk| {
                        pixels.extend(&*chunk);
                        future::ok::<Vec<u8>, error::CmdError>(pixels)
                    })
            })
            .and_then(|pixels| {
                // and voilla, we now have the bytes for the Wikipedia logo!
                assert!(pixels.len() > 0);
                println!("Wikipedia logo is {}b", pixels.len());
                Ok(())
            })
    }

    #[test]
    #[ignore]
    fn it_can_be_raw() {
        tester!(raw_inner)
    }
}
 */
