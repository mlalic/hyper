//! Client Requests
use std::convert::Into;
use std::marker::PhantomData;
use std::io::{self, Write, BufWriter};

use url::Url;

use method::{self, Method};
use header::Headers;
use header::{self, Host};
use net::{NetworkStream, NetworkConnector, HttpConnector, Fresh, Streaming};
use http::{HttpWriter, LINE_ENDING};
use http::HttpWriter::{ThroughWriter, ChunkedWriter, SizedWriter, EmptyWriter};
use version;
use HttpResult;
use client::{Response, get_host_and_port};
use client::response::HttpResponse;

/// A trait that should be implemented for types that represent HTTP requests.
pub trait HttpRequest {
    /// Returns a reference to the headers associated to this request.
    fn headers(&self) -> &Headers;
    /// Returns the HTTP method of the request.
    fn method(&self) -> method::Method;
}

/// A trait that should be implemented for types that represent a "Fresh" HTTP
/// request.
///
/// A fresh request is one for which no part has been sent to the server yet.
///
/// This trait allows for customizing the request before it is sent. Along with
/// that, once the request is started, it needs to return an appropriate
/// instance of a `StreamingRequest`.
pub trait FreshHttpRequest: HttpRequest {
    /// The type of the `StreamingRequest` that is produced by this
    /// `FreshHttpRequest`
    type Streaming: StreamingHttpRequest;
    /// Starts the request by consuming the instance and transforming it into
    /// the appropriate `StreamingRequest`.
    fn start(self) -> HttpResult<Self::Streaming>;

    /// Returns a mutable reference to the `Headers` instance that will be sent
    /// with the request.
    fn headers_mut(&mut self) -> &mut Headers;
}

/// A trait that should be implemented for types that represent "Streaming"
/// HTTP requests.
///
/// A streaming request is one that has already sent the headers to the server
/// and is now writing the body of the request. Therefore, it requires that
/// the `io::Write` trait also be implemented -- the writes represent sending a
/// chunk of the request's body to the server.
///
/// The `send` method needs to flush the request body to the server and transform
/// the insance to an appropriate `HttpResponse`.
pub trait StreamingHttpRequest: HttpRequest + io::Write {
    /// Flush the request body and produce an appropriate `HttpResponse` that
    /// can be used for obtaining the response that the server eventually sends.
    fn send(self) -> HttpResult<HttpResponse>;
}

/// A client request to a remote server.
pub struct Request<W> {
    /// The target URI for this request.
    pub url: Url,

    /// The HTTP version of this request.
    pub version: version::HttpVersion,

    body: HttpWriter<BufWriter<Box<NetworkStream + Send>>>,
    headers: Headers,
    method: method::Method,

    _marker: PhantomData<W>,
}

impl<W> Request<W> {
    /// Read the Request headers.
    #[inline]
    pub fn headers(&self) -> &Headers { &self.headers }

    /// Read the Request method.
    #[inline]
    pub fn method(&self) -> method::Method { self.method.clone() }
}

impl Request<Fresh> {
    /// Create a new client request.
    pub fn new(method: method::Method, url: Url) -> HttpResult<Request<Fresh>> {
        let mut conn = HttpConnector(None);
        Request::with_connector(method, url, &mut conn)
    }

    /// Create a new client request with a specific underlying NetworkStream.
    pub fn with_connector<C, S>(method: method::Method, url: Url, connector: &mut C)
        -> HttpResult<Request<Fresh>> where
        C: NetworkConnector<Stream=S>,
        S: Into<Box<NetworkStream + Send>> {
        debug!("{} {}", method, url);
        let (host, port) = try!(get_host_and_port(&url));

        let stream = try!(connector.connect(&*host, port, &*url.scheme)).into();
        let stream = ThroughWriter(BufWriter::new(stream));

        let mut headers = Headers::new();
        headers.set(Host {
            hostname: host,
            port: Some(port),
        });

        Ok(Request {
            method: method,
            headers: headers,
            url: url,
            version: version::HttpVersion::Http11,
            body: stream,
            _marker: PhantomData,
        })
    }

    /// Consume a Fresh Request, writing the headers and method,
    /// returning a Streaming Request.
    pub fn start(mut self) -> HttpResult<Request<Streaming>> {
        let mut uri = self.url.serialize_path().unwrap();
        //TODO: this needs a test
        if let Some(ref q) = self.url.query {
            uri.push('?');
            uri.push_str(&q[..]);
        }

        debug!("writing head: {:?} {:?} {:?}", self.method, uri, self.version);
        try!(write!(&mut self.body, "{} {} {}{}",
                    self.method, uri, self.version, LINE_ENDING));


        let stream = match self.method {
            Method::Get | Method::Head => {
                debug!("headers [\n{:?}]", self.headers);
                try!(write!(&mut self.body, "{}{}", self.headers, LINE_ENDING));
                EmptyWriter(self.body.into_inner())
            },
            _ => {
                let mut chunked = true;
                let mut len = 0;

                match self.headers.get::<header::ContentLength>() {
                    Some(cl) => {
                        chunked = false;
                        len = **cl;
                    },
                    None => ()
                };

                // cant do in match above, thanks borrowck
                if chunked {
                    let encodings = match self.headers.get_mut::<header::TransferEncoding>() {
                        Some(&mut header::TransferEncoding(ref mut encodings)) => {
                            //TODO: check if chunked is already in encodings. use HashSet?
                            encodings.push(header::Encoding::Chunked);
                            false
                        },
                        None => true
                    };

                    if encodings {
                        self.headers.set::<header::TransferEncoding>(
                            header::TransferEncoding(vec![header::Encoding::Chunked]))
                    }
                }

                debug!("headers [\n{:?}]", self.headers);
                try!(write!(&mut self.body, "{}{}", self.headers, LINE_ENDING));

                if chunked {
                    ChunkedWriter(self.body.into_inner())
                } else {
                    SizedWriter(self.body.into_inner(), len)
                }
            }
        };

        Ok(Request {
            method: self.method,
            headers: self.headers,
            url: self.url,
            version: self.version,
            body: stream,
            _marker: PhantomData,
        })
    }

    /// Get a mutable reference to the Request headers.
    #[inline]
    pub fn headers_mut(&mut self) -> &mut Headers { &mut self.headers }
}

impl Request<Streaming> {
    /// Completes writing the request, and returns a response to read from.
    ///
    /// Consumes the Request.
    pub fn send(self) -> HttpResult<Response> {
        let raw = try!(self.body.end()).into_inner().unwrap(); // end() already flushes
        Response::new(raw)
    }
}

impl Write for Request<Streaming> {
    #[inline]
    fn write(&mut self, msg: &[u8]) -> io::Result<usize> {
        self.body.write(msg)
    }

    #[inline]
    fn flush(&mut self) -> io::Result<()> {
        self.body.flush()
    }
}

impl<W> HttpRequest for Request<W> {
    fn headers(&self) -> &Headers { &self.headers }
    fn method(&self) -> method::Method { self.method.clone() }
}

impl FreshHttpRequest for Request<Fresh> {
    type Streaming = Request<Streaming>;
    fn start(self) -> HttpResult<Request<Streaming>> { self.start() }
    fn headers_mut(&mut self) -> &mut Headers { &mut self.headers }
}

impl StreamingHttpRequest for Request<Streaming> {
    fn send(self) -> HttpResult<HttpResponse> {
        let resp = try!(self.send());
        Ok(resp.into())
    }
}


#[cfg(test)]
mod tests {
    use std::str::from_utf8;
    use url::Url;
    use method::Method::{Get, Head};
    use mock::{MockStream, MockConnector};
    use super::Request;

    #[test]
    fn test_get_empty_body() {
        let req = Request::with_connector(
            Get, Url::parse("http://example.dom").unwrap(), &mut MockConnector
        ).unwrap();
        let req = req.start().unwrap();
        let stream = *req.body.end().unwrap()
            .into_inner().unwrap().downcast::<MockStream>().ok().unwrap();
        let bytes = stream.write;
        let s = from_utf8(&bytes[..]).unwrap();
        assert!(!s.contains("Content-Length:"));
        assert!(!s.contains("Transfer-Encoding:"));
    }

    #[test]
    fn test_head_empty_body() {
        let req = Request::with_connector(
            Head, Url::parse("http://example.dom").unwrap(), &mut MockConnector
        ).unwrap();
        let req = req.start().unwrap();
        let stream = *req.body.end().unwrap()
            .into_inner().unwrap().downcast::<MockStream>().ok().unwrap();
        let bytes = stream.write;
        let s = from_utf8(&bytes[..]).unwrap();
        assert!(!s.contains("Content-Length:"));
        assert!(!s.contains("Transfer-Encoding:"));
    }
}
