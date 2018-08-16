// you must have a webdriver server running on localhost:4444
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
fn curl(url: String) -> Result<()> {
    let driver = await!(Driver::new("http://localhost:4444".into(), None))?;
    await!(driver.clone().goto(url))?;
    let body = await!(driver.clone().find(Locator::Css("body".into())))?;
    println!("{}", await!(driver.html(body, true))?);
    Ok(())
}

fn main() {
    if let Some(url) = args().nth(1) {
        run(async_block! {
            await!(curl(url)).map_err(|e| eprintln!("error: {}", e))
        })
    } else {
        eprintln!("usage curl: <url>");
        exit(1)
    }
}
