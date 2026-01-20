// Copyright 2021 The Grin Developers
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

/// Attribute macro for documenting API endpoints
///
/// Example usage:
/// ```rust
/// #[api_endpoint]
/// #[endpoint(
///     path = "/v1/status",
///     method = "GET",
///     summary = "Get node status",
///     description = "Returns the current status of the node"
/// )]
/// pub struct StatusHandler {
///     pub chain: Weak<Chain>,
/// }
/// ```
#[macro_export]
macro_rules! api_endpoint {
    (
        $(#[endpoint(
            path = $path:expr,
            method = $method:expr,
            summary = $summary:expr,
            description = $description:expr
            $(, params = [$($param:expr),*])?
            $(, response = $response:ty)?
        )])*
        pub struct $name:ident {
            $($field:ident: $type:ty),* $(,)?
        }
    ) => {
        pub struct $name {
            $($field: $type),*
        }

        impl ApiEndpoint for $name {
            fn get_endpoint_spec() -> EndpointSpec {
                EndpointSpec {
                    path: $path.to_string(),
                    method: $method.to_string(),
                    summary: $summary.to_string(),
                    description: $description.to_string(),
                    params: vec![$($($param.into()),*)?],
                    response: None $(Some(stringify!($response).to_string()))?
                }
            }
        }
    };
}

/// Trait for types that represent API endpoints
pub trait ApiEndpoint {
	fn get_endpoint_spec() -> EndpointSpec;
}

/// Represents an OpenAPI endpoint specification
pub struct EndpointSpec {
	pub path: String,
	pub method: String,
	pub summary: String,
	pub description: String,
	pub params: Vec<ParamSpec>,
	pub response: Option<String>,
}

/// Represents an OpenAPI parameter specification
pub struct ParamSpec {
	pub name: String,
	pub description: String,
	pub required: bool,
	pub schema_type: String,
	pub location: String,
}

impl From<(&str, &str, bool, &str, &str)> for ParamSpec {
	fn from(tuple: (&str, &str, bool, &str, &str)) -> Self {
		ParamSpec {
			name: tuple.0.to_string(),
			description: tuple.1.to_string(),
			required: tuple.2,
			schema_type: tuple.3.to_string(),
			location: tuple.4.to_string(),
		}
	}
}
