#![deny(warnings)]
extern crate hyper;

extern crate env_logger;

use std::env;
use std::io;

use hyper::Client;
use hyper::http2::Http2RequestFactory;

fn main() {
    env_logger::init().unwrap();

    let url = match env::args().nth(1) {
        Some(url) => url,
        None => {
            println!("Usage: client <url>");
            return;
        }
    };

    // The only difference to the HTTP/1.1 client usage is here:
    // We explcitly opt-into using the Http2RequestFactory...
    let mut client: Client = Client::with_factory(Http2RequestFactory);

    let mut res = match client.get(&*url).send() {
        Ok(res) => res,
        Err(err) => panic!("Failed to connect: {:?}", err)
    };

    println!("Response: {}", res.status());
    println!("Headers:\n{}", res.headers());
    io::copy(&mut res, &mut io::stdout()).unwrap();
}
