//! The module adapts the HTTP/2 abstractions provided by `solicit` in
//! interfaces, such that it becomes possible to seamlessly plug them into
//! `hyper::client::Client` and use it for HTTP communication.
use std::io::Write;
use std::io::Cursor;
use std::net::TcpStream;
use std::marker::PhantomData;

use solicit::http::StreamId;
use solicit::http::HttpResult as Http2Result;
use solicit::http::Response as RawHttp2Response;
use solicit::client::SimpleClient;
use solicit::http::connection::{TlsConnector, CleartextConnector};

use url::Url;

use openssl::ssl::SslStream;

use header::Headers;
use method::Method;
use net::{Fresh, Streaming};

use status;
use HttpResult;

/// A wrapper around `solicit`'s `SimpleClient` such that it hides away the
/// details of which generic version it uses depending on whether it needs
/// to be `http://` or `https://`
///
/// Exposes a minimized interface of the original `SimpleClient`.
pub enum Http2Client {
    Http(SimpleClient<TcpStream>),
    Https(SimpleClient<SslStream<TcpStream>>),
}

impl Http2Client {
    /// Creates a new `Http2Client` that connects to the given host, depending
    /// on the given scheme.
    pub fn new(scheme: &str, hostname: &str) -> Http2Client {
        match scheme {
            "http" => {
                Http2Client::new_http(hostname)
            },
            "https" => {
                Http2Client::new_https(hostname)
            },
            _ => {
                panic!("Invalid scheme.");
            },
        }
    }

    /// Creates a new `Http2Client` that will use an HTTP connection for
    /// communication with the given host.
    pub fn new_http(hostname: &str) -> Http2Client {
        let connector = CleartextConnector { host: hostname };
        let client = SimpleClient::with_connector(connector).unwrap();
        Http2Client::Http(client)
    }

    /// Creates a new `Http2Client` that will use an HTTPS connection for
    /// communication with the given host.
    pub fn new_https(hostname: &str) -> Http2Client {
        // TODO Actually set up an HTTPS connection
        panic!("not yet implemented")
    }

    /// Send a request with the given metadata to the server.
    /// A thin wrapper around `SimpleClient::request`.
    pub fn request(&mut self, method: &[u8], path: &[u8], extras: &[(Vec<u8>, Vec<u8>)])
            -> Http2Result<StreamId> {
        match self {
            &mut Http2Client::Http(ref mut client) => client.request(method, path, extras),
            &mut Http2Client::Https(ref mut client) => client.request(method, path, extras),
        }
    }

    /// Read the response to the given stream_id. It will block the calling thread
    /// until either the response is received or the connection errors.
    /// A thin wrapper around `SimpleClient::get_response`.
    pub fn get_response(&mut self, stream_id: StreamId) -> Http2Result<RawHttp2Response> {
        match self {
            &mut Http2Client::Http(ref mut client) => client.get_response(stream_id),
            &mut Http2Client::Https(ref mut client) => client.get_response(stream_id),
        }
    }
}

/// A struct representing an HTTP/2-based request. Satisfies the same interface
/// that `hyper::client::request::Request` does and allows for a similar
/// transformation of `Fresh -> Streaming -> Response`
pub struct Http2Request<W> {
    client: Http2Client,
    headers: Headers,
    stream_id: Option<StreamId>,
    method: Method,
    url: Url,
    _marker: PhantomData<W>,
}

impl Http2Request<Fresh> {
    fn start(mut self) -> HttpResult<Http2Request<Streaming>> {
        // Prepare the request metadata so that it fits into `solicit`'s API.
        // It basically just adapts the method and path into byte slices.
        let mut method_bytes: Vec<u8> = Vec::new();
        write!(&mut method_bytes, "{}", self.method);

        // TODO Refactor this to a helper function, as it is reused verbatim
        //      from the HTTP/1.x code.
        let path = {
            let mut uri = self.url.serialize_path().unwrap();
            if let Some(ref q) = self.url.query {
                uri.push('?');
                uri.push_str(&q[..]);
            }
            uri.into_bytes()
        };

        // Initiate a request and remember the corresponding HTTP/2 stream ID
        // so that we can refer to it when we want to read the response.
        let stream_id = self.client.request(&method_bytes, &path, &[]).unwrap();

        Ok(Http2Request {
            client: self.client,
            headers: self.headers,
            stream_id: Some(stream_id),
            method: self.method,
            url: self.url,
            _marker: PhantomData,
        })
    }
}

/// A struct representing an HTTP/2 response adapted to fit into `hyper`'s
/// interface. Supports the same operations that the original HTTP/1.x response
/// did.
pub struct Http2Response {
    client: Http2Client,
    stream_id: StreamId,
    pub headers: Headers,
    pub status: status::StatusCode,
    body: Cursor<Vec<u8>>,
}

impl Http2Response {
    fn new(mut client: Http2Client, stream_id: StreamId) -> Http2Response {
        // First, we get the HTTP/2 response, as provided by `solicit`
        let resp = client.get_response(stream_id).unwrap();
        // ...then the various chunks are stripped out and wrapped in a response
        // that satisfies `hyper`'s interface.

        // - The status code
        let status = status::StatusCode::from_u16(resp.status_code().unwrap());

        // - The headers
        let mut headers = Headers::new();
        // Headers are moved into `hyper`'s `Headers` collection; pseudo-headers
        // are ignored (status has already been extracted).
        // TODO: Coalesce possible headers with the name key into a single vector.
        //       `solicit` returns the raw original header list and lets the clients
        //       figure out how they would like to transform it.
        for (name, value) in resp.headers.into_iter().filter(|header| header.0[0] != b':') {
            // Unfortunately, `hyper` wants its header names to be strings...
            // We just panic if it ends up being an invalid utf8 string, but no
            // copies are made.
            headers.set_raw(String::from_utf8(name).unwrap(), vec![value]);
        }

        // - The response body
        // For now, since `solicit` has already read the full response, we just
        // wrap it into a `Cursor` to allow for the public interface to support
        // `io::Read`.
        let body = Cursor::new(resp.body);

        // Finally, we're done -- package them all up in a struct
        Http2Response {
            status: status,
            client: client,
            stream_id: stream_id,
            headers: headers,
            body: body,
        }
    }
}
