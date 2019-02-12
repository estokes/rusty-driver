error_chain! {
    foreign_links {
        HttpError(::http::Error);
        HyperError(::hyper::error::Error);
        IoErr(::std::io::Error);
        WebDriver(::webdriver::error::WebDriverError);
        BadUrl(::url::ParseError);
        InvalidJson(::serde_json::Error);
        Utf8(::std::str::Utf8Error);
        HeaderStr(::hyper::header::ToStrError);
        Timer(::tokio_timer::Error);
    }

    errors {
        NotW3C(o: ::serde_json::Value) {
            description("not a valid W3C response")
            display("not a valid W3C response {}", o)
        }

        NotJson(ctyp: Option<String>) {
            description("expected JSON"),
            display("expected JSON got ctype: {:?}", ctyp)
        }
    }
}
