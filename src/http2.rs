//! The module adapts the HTTP/2 abstractions provided by `solicit` in
//! interfaces, such that it becomes possible to seamlessly plug them into
//! `hyper::client::Client` and use it for HTTP communication.
