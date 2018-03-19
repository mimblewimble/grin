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

/// Build router for provided endpoints.
///
/// Format for each endpoint:
///  http_method endpoint => handler
///
/// # Examples
///
/// ```text
/// # #[macro_use] extern crate grin_api;
/// # fn () {
/// let router = router! {
///    get "/v1/a/b" => static_handler,
///    post "/v1/c/:d" => param_handler
/// }
/// # }
/// ```
///
#[macro_export]
macro_rules! router {
    ($($method:ident $glob:expr => $handler:expr),+ $(,)*) => ({
        let mut builder = Router::builder();
        $(builder.$method($glob, $handler);)*
        builder.into_router()
    });
}