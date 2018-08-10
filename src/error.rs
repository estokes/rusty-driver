error_chain! {
    foreign_links {
        HttpError(::http::Error);
        HyperError(::hyper::error::Error);
        IoErr(::std::io::Error);
        WebDriver(::webdriver::error::WebDriverError);
        BadUrl(::url::ParseError);
        InvalidJson(::rustc_serialize::json::ParserError);
        Utf8(::std::str::Utf8Error);
        HeaderStr(::hyper::header::ToStrError);
    }

    errors {
        NotW3C(o: ::rustc_serialize::json::Json) {
            description("not a valid W3C response")
            display("not a valid W3C response {}", o)
        }

        NotJson(body: ::hyper::Chunk, ctyp: Option<String>) {
            description("expected JSON"),
            display("expected JSON got ctype: {:?}, body: {:?}", ctyp, body)
        }
    }
}
