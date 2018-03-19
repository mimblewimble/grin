// Copyright 2018 The Grin Developers
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

use hyper::Method;
use std::borrow::Cow;
use std::fmt;

use rest::Handler;

/// Route
///
/// Contains Http Method, endpoint and corresponding handler.
pub struct Route<> {
    method: Method,
    endpoint: Cow<'static, str>,
    handler: Box<Handler>,
}

impl Route {

    pub fn new<H: Handler>(method: Method, endpoint: &str, handler: H) -> Route {
        let cow_ep = endpoint.to_owned().into();
        Route {
            method: method,
            endpoint: cow_ep,
            handler: Box::new(handler),
        }
    }

    /// Returns the reference to http method.
    #[inline]
    pub fn method(&self) -> &Method {
        &self.method
    }

    /// Returns the reference to endpoint.
    #[inline]
    pub fn endpoint(&self) -> &str {
        &*self.endpoint
    }

    /// Returns the reference to the handler.
    #[inline]
    pub fn handler(&self) -> &Handler {
        &*self.handler
    }

}

impl fmt::Debug for Route {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.debug_struct("Route")
            .field("method", &self.method)
            .field("endpoint", &self.endpoint)
            .field("handler", &"<handler>")
            .finish()
    }
}