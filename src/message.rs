//! Defines the `HttpMessage` trait that serves to encapsulate the operations of a single
//! request-response cycle on any HTTP connection.

use std::fmt::Debug;
use std::any::{Any, TypeId};
use std::io::{Read, Write};

use std::mem;

use typeable::Typeable;

use header::Headers;
use http::RawStatus;
use url::Url;

use method;
use version;
use traitobject;

/// Describes a request.
pub struct RequestHead {
    pub headers: Headers,
    pub method: method::Method,
    pub url: Url,
}

/// Describes a response.
pub struct ResponseHead {
    pub headers: Headers,
    pub raw_status: RawStatus,
    pub version: version::HttpVersion,
}

pub trait HttpMessage: Write + Read + Send + Any + Typeable + Debug {
    fn set_outgoing(&mut self, head: RequestHead) -> ::Result<RequestHead>;
    fn get_incoming(&mut self) -> ::Result<ResponseHead>;

    fn close_connection(&mut self) -> ::Result<()>;
}

impl HttpMessage {
    unsafe fn downcast_ref_unchecked<T: 'static>(&self) -> &T {
        mem::transmute(traitobject::data(self))
    }

    unsafe fn downcast_mut_unchecked<T: 'static>(&mut self) -> &mut T {
        mem::transmute(traitobject::data_mut(self))
    }

    unsafe fn downcast_unchecked<T: 'static>(self: Box<HttpMessage>) -> Box<T>  {
        let raw: *mut HttpMessage = mem::transmute(self);
        mem::transmute(traitobject::data_mut(raw))
    }
}

impl HttpMessage {
    /// Is the underlying type in this trait object a T?
    #[inline]
    pub fn is<T: Any>(&self) -> bool {
        (*self).get_type() == TypeId::of::<T>()
    }

    /// If the underlying type is T, get a reference to the contained data.
    #[inline]
    pub fn downcast_ref<T: Any>(&self) -> Option<&T> {
        if self.is::<T>() {
            Some(unsafe { self.downcast_ref_unchecked() })
        } else {
            None
        }
    }

    /// If the underlying type is T, get a mutable reference to the contained
    /// data.
    #[inline]
    pub fn downcast_mut<T: Any>(&mut self) -> Option<&mut T> {
        if self.is::<T>() {
            Some(unsafe { self.downcast_mut_unchecked() })
        } else {
            None
        }
    }

    /// If the underlying type is T, extract it.
    #[inline]
    pub fn downcast<T: Any>(self: Box<HttpMessage>)
            -> Result<Box<T>, Box<HttpMessage>> {
        if self.is::<T>() {
            Ok(unsafe { self.downcast_unchecked() })
        } else {
            Err(self)
        }
    }
}
