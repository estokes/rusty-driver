// you must have a webdriver server running on localhost:4444
// e.g. if you are using firefox you would run something like,
//
// geckodriver --connect-existing --marionette-port 2828 --log trace
//
// to connect to your existing firefox (which you started with firefox --marionette) or
//
// geckodriver --marionette-port 2828 --log trace
//
// to start a new browser when you create a session. The advantage of
// the first method is it can run with all your existing user cookies,
// the second one creates a completely new profile.
#![feature(generators, use_extern_macros, proc_macro_non_items, nll)]
#![recursion_limit="128"]
extern crate tokio;
extern crate futures_await as futures;
extern crate rusty_driver;

use tokio::run;
use futures::prelude::await;
use futures::prelude::*;
use rusty_driver::{Locator, Driver, error::*};
use std::{env::args, process::exit};

#[async]
fn wikipedia(article: String) -> Result<()> {
    let driver = await!(Driver::new("http://localhost:4444".into(), None))?;
    await!(driver.clone().goto("https://www.wikipedia.org".into()))?;
    let search = await!(
        driver.clone().find(Locator::Css("form#search-form".into()), None)
    )?;
    await!(driver.clone().set_by_name(search.clone(), "search".into(), article))?;
    await!(driver.clone().submit(search))?;
    println!("{}", await!(driver.source())?);
    Ok(())
}

fn main() {
    if let Some(article) = args().nth(1) {
        run(async_block! {
            await!(wikipedia(article)).map_err(|e| eprintln!("error: {}", e))
        })
    } else {
        eprintln!("usage curl: <url>");
        exit(1)
    }
}
