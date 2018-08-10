
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
