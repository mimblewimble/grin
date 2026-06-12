// Copyright 2024 The Grin Developers
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

use serde::{Deserialize, Serialize};
use serde_json;
use serde_yaml;
use std::collections::HashMap;
use std::fs::File;
use std::io::Write;
use std::path::Path;
use syn;

#[allow(non_snake_case)]
#[derive(Serialize, Deserialize)]
pub struct OpenApiSpec {
	openapi: String,
	info: Info,
	paths: HashMap<String, PathItem>,
	components: Components,
}

#[derive(Serialize, Deserialize)]
pub struct Info {
	title: String,
	version: String,
	description: Option<String>,
}

#[derive(Serialize, Deserialize)]
pub struct Components {
	schemas: HashMap<String, Schema>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct Schema {
	#[serde(rename = "type")]
	type_: String,
	#[serde(skip_serializing_if = "Option::is_none")]
	properties: Option<HashMap<String, Schema>>,
	#[serde(skip_serializing_if = "Option::is_none")]
	items: Option<Box<Schema>>,
	#[serde(skip_serializing_if = "Option::is_none")]
	description: Option<String>,
	#[serde(skip_serializing_if = "Option::is_none")]
	format: Option<String>,
	#[serde(skip_serializing_if = "Option::is_none")]
	example: Option<serde_json::Value>,
	#[serde(skip_serializing_if = "Option::is_none")]
	oneOf: Option<Vec<Schema>>,
	#[serde(skip_serializing_if = "Option::is_none")]
	required: Option<Vec<String>>,
	#[serde(skip_serializing_if = "Option::is_none")]
	additionalProperties: Option<bool>,
	#[serde(rename = "enum", skip_serializing_if = "Option::is_none")]
	enum_values_renamed: Option<Vec<String>>,
	#[serde(skip_serializing_if = "Option::is_none")]
	title: Option<String>,
	#[serde(skip_serializing_if = "Option::is_none")]
	summary: Option<String>,
}

#[derive(Serialize, Deserialize)]
pub struct PathItem {
	post: Operation,
}

#[allow(non_snake_case)]
#[derive(Serialize, Deserialize)]
pub struct Operation {
	summary: String,
	description: Option<String>,
	requestBody: RequestBody,
	responses: HashMap<String, Response>,
	tags: Vec<String>,
}

#[derive(Serialize, Deserialize)]
pub struct RequestBody {
	description: String,
	content: HashMap<String, MediaType>,
	required: bool,
}

#[derive(Serialize, Deserialize)]
pub struct Response {
	description: String,
	content: Option<HashMap<String, MediaType>>,
}

#[derive(Serialize, Deserialize)]
pub struct MediaType {
	schema: Schema,
}

fn type_to_schema(ty: &syn::Type) -> Schema {
	match ty {
		syn::Type::Path(type_path) => {
			let path = &type_path.path;
			let last_segment = path.segments.last().unwrap();
			let type_name = last_segment.ident.to_string();

			match type_name.as_str() {
				"String" | "str" => Schema {
					type_: "string".to_string(),
					properties: None,
					items: None,
					description: None,
					format: None,
					example: None,
					oneOf: None,
					required: None,
					additionalProperties: None,
					enum_values_renamed: None,
					title: None,
					summary: None,
				},
				"bool" => Schema {
					type_: "boolean".to_string(),
					properties: None,
					items: None,
					description: None,
					format: None,
					example: None,
					oneOf: None,
					required: None,
					additionalProperties: None,
					enum_values_renamed: None,
					title: None,
					summary: None,
				},
				"i8" | "i16" | "i32" | "i64" | "u8" | "u16" | "u32" | "u64" => Schema {
					type_: "integer".to_string(),
					properties: None,
					items: None,
					description: None,
					format: Some(type_name.to_string()),
					example: None,
					oneOf: None,
					required: None,
					additionalProperties: None,
					enum_values_renamed: None,
					title: None,
					summary: None,
				},
				"f32" | "f64" => Schema {
					type_: "number".to_string(),
					properties: None,
					items: None,
					description: None,
					format: Some(type_name.to_string()),
					example: None,
					oneOf: None,
					required: None,
					additionalProperties: None,
					enum_values_renamed: None,
					title: None,
					summary: None,
				},
				"Vec" => {
					if let syn::PathArguments::AngleBracketed(args) = &last_segment.arguments {
						if let Some(syn::GenericArgument::Type(inner_type)) = args.args.first() {
							Schema {
								type_: "array".to_string(),
								properties: None,
								items: Some(Box::new(type_to_schema(inner_type))),
								description: None,
								format: None,
								example: None,
								oneOf: None,
								required: None,
								additionalProperties: None,
								enum_values_renamed: None,
								title: None,
								summary: None,
							}
						} else {
							default_schema()
						}
					} else {
						default_schema()
					}
				}
				"Option" => {
					if let syn::PathArguments::AngleBracketed(args) = &last_segment.arguments {
						if let Some(syn::GenericArgument::Type(inner_type)) = args.args.first() {
							type_to_schema(inner_type) // For Option, we just use the inner type's schema
						} else {
							default_schema()
						}
					} else {
						default_schema()
					}
				}
				"Result" => {
					if let syn::PathArguments::AngleBracketed(args) = &last_segment.arguments {
						if let Some(syn::GenericArgument::Type(ok_type)) = args.args.first() {
							type_to_schema(ok_type) // For Result, we use the Ok type's schema
						} else {
							default_schema()
						}
					} else {
						default_schema()
					}
				}
				"HashMap" => Schema {
					type_: "object".to_string(),
					properties: None,
					items: None,
					description: None,
					format: None,
					example: None,
					oneOf: None,
					required: None,
					additionalProperties: Some(true),
					enum_values_renamed: None,
					title: None,
					summary: None,
				},
				_ => {
					// For custom types, create an object schema
					Schema {
						type_: "object".to_string(),
						properties: Some(HashMap::new()),
						items: None,
						description: Some(format!("Custom type: {}", type_name)),
						format: None,
						example: None,
						oneOf: None,
						required: None,
						additionalProperties: Some(true),
						enum_values_renamed: None,
						title: None,
						summary: None,
					}
				}
			}
		}
		_ => default_schema(),
	}
}

fn default_schema() -> Schema {
	Schema {
		type_: "object".to_string(),
		properties: None,
		items: None,
		description: None,
		format: None,
		example: None,
		oneOf: None,
		required: None,
		additionalProperties: None,
		enum_values_renamed: None,
		title: None,
		summary: None,
	}
}

fn parse_doc_comment(
	attrs: &[syn::Attribute],
) -> (Option<String>, Option<String>, Vec<(String, String)>) {
	let mut doc_lines = Vec::new();
	for attr in attrs {
		if attr.path().is_ident("doc") {
			if let Ok(doc) = attr.parse_args::<syn::LitStr>() {
				let line = doc.value().trim().to_string();
				if !line.is_empty() {
					doc_lines.push(line);
				}
			}
		}
	}

	if doc_lines.is_empty() {
		return (None, None, Vec::new());
	}

	// Extract summary (first line) and description
	let mut summary = None;
	let mut description = Vec::new();
	let mut params = Vec::new();
	let mut in_params = false;
	let mut in_returns = false;
	let mut current_section = Vec::new();

	for line in doc_lines {
		if line.starts_with("# Arguments") {
			// Add accumulated lines to description if we're not already in a section
			if !in_params && !in_returns && !current_section.is_empty() {
				description.extend(current_section.drain(..));
			}
			in_params = true;
			in_returns = false;
			current_section.clear();
			continue;
		} else if line.starts_with("# Returns") {
			// Add accumulated lines to description if we're not already in a section
			if !in_params && !in_returns && !current_section.is_empty() {
				description.extend(current_section.drain(..));
			}
			in_params = false;
			in_returns = true;
			current_section.clear();
			continue;
		}

		if summary.is_none() && !line.starts_with('#') {
			summary = Some(line.clone());
			continue;
		}

		if in_params {
			if let Some(param_doc) = line.strip_prefix("* `") {
				if let Some((param_name, param_desc)) = param_doc.split_once("` - ") {
					params.push((param_name.to_string(), param_desc.to_string()));
				}
			}
		} else if !in_returns {
			// Only add to current section if it's not a section header
			if !line.starts_with('#') {
				current_section.push(line);
			}
		}
	}

	// Add any remaining lines to description
	if !current_section.is_empty() {
		description.extend(current_section);
	}

	(
		summary,
		if description.is_empty() {
			None
		} else {
			Some(description.join("\n"))
		},
		params,
	)
}

fn create_method_schema(
	method_name: &str,
	doc_summary: Option<String>,
	doc_description: Option<String>,
	params_schema: HashMap<String, Schema>,
	param_descriptions: Vec<(String, String)>,
	return_schema: Schema,
) -> Schema {
	let mut properties = HashMap::new();

	// Add standard JSON-RPC fields with version as enum
	properties.insert(
		"jsonrpc".to_string(),
		Schema {
			type_: "string".to_string(),
			properties: None,
			items: None,
			description: Some("JSON-RPC version".to_string()),
			format: None,
			example: None,
			oneOf: None,
			required: None,
			additionalProperties: None,
			enum_values_renamed: Some(vec!["2.0".to_string()]),
			title: None,
			summary: None,
		},
	);

	// Method field should use enum instead of example
	properties.insert(
		"method".to_string(),
		Schema {
			type_: "string".to_string(),
			properties: None,
			items: None,
			description: doc_summary.clone(),
			format: None,
			example: None,
			oneOf: None,
			required: None,
			additionalProperties: None,
			enum_values_renamed: Some(vec![method_name.to_string()]),
			title: None,
			summary: None,
		},
	);

	// Create params schema with descriptions
	let mut params_with_desc = params_schema.clone();
	for (param_name, param_desc) in param_descriptions {
		if let Some(param_schema) = params_with_desc.get_mut(&param_name) {
			param_schema.description = Some(param_desc);
		}
	}

	// Only add params if we have any
	let params_schema = if params_with_desc.is_empty() {
		Schema {
			type_: "object".to_string(),
			properties: Some(HashMap::new()),
			items: None,
			description: Some("Method parameters".to_string()),
			format: None,
			example: Some(serde_json::json!({})),
			oneOf: None,
			required: None,
			additionalProperties: Some(false),
			enum_values_renamed: None,
			title: None,
			summary: None,
		}
	} else {
		Schema {
			type_: "object".to_string(),
			properties: Some(params_with_desc),
			items: None,
			description: Some("Method parameters".to_string()),
			format: None,
			example: None,
			oneOf: None,
			required: Some(params_schema.keys().map(|k| k.to_string()).collect()),
			additionalProperties: Some(false),
			enum_values_renamed: None,
			title: None,
			summary: None,
		}
	};

	properties.insert("params".to_string(), params_schema);

	properties.insert(
		"id".to_string(),
		Schema {
			type_: "string".to_string(),
			properties: None,
			items: None,
			description: Some("Request ID".to_string()),
			format: None,
			example: Some(serde_json::json!(1)),
			oneOf: None,
			required: None,
			additionalProperties: None,
			enum_values_renamed: None,
			title: None,
			summary: None,
		},
	);

	Schema {
		type_: "object".to_string(),
		properties: Some(properties),
		items: None,
		description: doc_description,
		format: None,
		example: None,
		oneOf: None,
		required: Some(vec![
			"jsonrpc".to_string(),
			"method".to_string(),
			"params".to_string(),
			"id".to_string(),
		]),
		additionalProperties: Some(false),
		enum_values_renamed: None,
		title: Some(method_name.to_string()),
		summary: doc_summary,
	}
}

fn create_rpc_endpoint(
	spec: &mut OpenApiSpec,
	base_path: &str,
	methods: Vec<(
		String,
		Option<String>,
		Option<String>,
		HashMap<String, Schema>,
		Vec<(String, String)>,
		Schema,
	)>,
) {
	let method_schemas: Vec<Schema> = methods
		.into_iter()
		.map(
			|(name, summary, description, params, param_descriptions, ret)| {
				let mut schema = create_method_schema(
					&name,
					summary.clone(),
					description.clone(),
					params,
					param_descriptions,
					ret,
				);

				// Add method description to the schema title and description
				if let Some(desc) = summary {
					schema.title = Some(format!("{} - {}", name, desc));
				} else {
					schema.title = Some(name.clone());
				}

				// Add full description if available
				if let Some(desc) = description {
					schema.description = Some(desc);
				}

				schema
			},
		)
		.collect();

	let operation = Operation {
		summary: format!("JSON-RPC endpoint for {}", &base_path[4..]),
		description: Some("JSON-RPC 2.0 endpoint".to_string()),
		requestBody: RequestBody {
			description: "JSON-RPC request".to_string(),
			content: {
				let mut content = HashMap::new();
				content.insert(
					"application/json".to_string(),
					MediaType {
						schema: Schema {
							type_: "object".to_string(),
							properties: None,
							items: None,
							description: None,
							format: None,
							example: None,
							oneOf: Some(method_schemas),
							required: None,
							additionalProperties: None,
							enum_values_renamed: None,
							title: None,
							summary: None,
						},
					},
				);
				content
			},
			required: true,
		},
		responses: {
			let mut responses = HashMap::new();
			responses.insert(
				"200".to_string(),
				Response {
					description: "Successful response".to_string(),
					content: Some({
						let mut content = HashMap::new();
						content.insert(
							"application/json".to_string(),
							MediaType {
								schema: Schema {
									type_: "object".to_string(),
									properties: Some({
										let mut props = HashMap::new();
										props.insert(
											"jsonrpc".to_string(),
											Schema {
												type_: "string".to_string(),
												properties: None,
												items: None,
												description: Some("JSON-RPC version".to_string()),
												format: None,
												example: None,
												oneOf: None,
												required: None,
												additionalProperties: None,
												enum_values_renamed: Some(vec!["2.0".to_string()]),
												title: None,
												summary: None,
											},
										);
										props.insert(
											"result".to_string(),
											Schema {
												type_: "object".to_string(),
												properties: None,
												items: None,
												description: Some("Method result".to_string()),
												format: None,
												example: None,
												oneOf: None,
												required: None,
												additionalProperties: Some(true),
												enum_values_renamed: None,
												title: None,
												summary: None,
											},
										);
										props.insert(
											"id".to_string(),
											Schema {
												type_: "string".to_string(),
												properties: None,
												items: None,
												description: Some("Request ID".to_string()),
												format: None,
												example: Some(serde_json::json!(1)),
												oneOf: None,
												required: None,
												additionalProperties: None,
												enum_values_renamed: None,
												title: None,
												summary: None,
											},
										);
										props
									}),
									items: None,
									description: None,
									format: None,
									example: None,
									oneOf: None,
									required: Some(vec![
										"jsonrpc".to_string(),
										"result".to_string(),
										"id".to_string(),
									]),
									additionalProperties: Some(false),
									enum_values_renamed: None,
									title: None,
									summary: None,
								},
							},
						);
						content
					}),
				},
			);
			responses.insert(
				"400".to_string(),
				Response {
					description: "Invalid request".to_string(),
					content: Some({
						let mut content = HashMap::new();
						content.insert(
							"application/json".to_string(),
							MediaType {
								schema: Schema {
									type_: "object".to_string(),
									properties: Some({
										let mut props = HashMap::new();
										props.insert(
											"jsonrpc".to_string(),
											Schema {
												type_: "string".to_string(),
												properties: None,
												items: None,
												description: Some("JSON-RPC version".to_string()),
												format: None,
												example: None,
												oneOf: None,
												required: None,
												additionalProperties: None,
												enum_values_renamed: Some(vec!["2.0".to_string()]),
												title: None,
												summary: None,
											},
										);
										props.insert(
											"error".to_string(),
											Schema {
												type_: "object".to_string(),
												properties: Some({
													let mut error_props = HashMap::new();
													error_props.insert(
														"code".to_string(),
														Schema {
															type_: "integer".to_string(),
															properties: None,
															items: None,
															description: Some(
																"Error code".to_string(),
															),
															format: None,
															example: None,
															oneOf: None,
															required: None,
															additionalProperties: None,
															enum_values_renamed: None,
															title: None,
															summary: None,
														},
													);
													error_props.insert(
														"message".to_string(),
														Schema {
															type_: "string".to_string(),
															properties: None,
															items: None,
															description: Some(
																"Error message".to_string(),
															),
															format: None,
															example: None,
															oneOf: None,
															required: None,
															additionalProperties: None,
															enum_values_renamed: None,
															title: None,
															summary: None,
														},
													);
													error_props
												}),
												items: None,
												description: Some("Error details".to_string()),
												format: None,
												example: None,
												oneOf: None,
												required: Some(vec![
													"code".to_string(),
													"message".to_string(),
												]),
												additionalProperties: Some(false),
												enum_values_renamed: None,
												title: None,
												summary: None,
											},
										);
										props.insert(
											"id".to_string(),
											Schema {
												type_: "string".to_string(),
												properties: None,
												items: None,
												description: Some("Request ID".to_string()),
												format: None,
												example: Some(serde_json::json!(1)),
												oneOf: None,
												required: None,
												additionalProperties: None,
												enum_values_renamed: None,
												title: None,
												summary: None,
											},
										);
										props
									}),
									items: None,
									description: None,
									format: None,
									example: None,
									oneOf: None,
									required: Some(vec![
										"jsonrpc".to_string(),
										"error".to_string(),
										"id".to_string(),
									]),
									additionalProperties: Some(false),
									enum_values_renamed: None,
									title: None,
									summary: None,
								},
							},
						);
						content
					}),
				},
			);
			responses
		},
		tags: vec![base_path[4..].to_string()], // Remove /v2/ prefix for tag
	};

	spec.paths
		.insert(base_path.to_string(), PathItem { post: operation });
}

fn collect_methods(
	source: &str,
	trait_name: &str,
) -> Vec<(
	String,
	Option<String>,
	Option<String>,
	HashMap<String, Schema>,
	Vec<(String, String)>,
	Schema,
)> {
	let mut methods = Vec::new();
	let file = syn::parse_str::<syn::File>(source).unwrap();

	for item in file.items {
		if let syn::Item::Trait(item_trait) = item {
			if item_trait.ident == trait_name {
				for item in item_trait.items {
					if let syn::TraitItem::Fn(method) = item {
						let method_name = method.sig.ident.to_string();
						let (doc_summary, doc_description, param_descriptions) =
							parse_doc_comment(&method.attrs);

						let mut params_schema = HashMap::new();
						for param in &method.sig.inputs {
							if let syn::FnArg::Typed(pat_type) = param {
								if let syn::Pat::Ident(pat_ident) = &*pat_type.pat {
									let param_name = pat_ident.ident.to_string();
									if param_name != "self" {
										params_schema
											.insert(param_name, type_to_schema(&pat_type.ty));
									}
								}
							}
						}

						let mut return_schema = default_schema();
						if let syn::ReturnType::Type(_, ty) = &method.sig.output {
							return_schema = type_to_schema(ty);
						}

						methods.push((
							method_name,
							doc_summary,
							doc_description,
							params_schema,
							param_descriptions,
							return_schema,
						));
					}
				}
			}
		}
	}
	methods
}

impl Default for Info {
	fn default() -> Self {
		Info {
			title: "Grin Node API".to_string(),
			version: env!("CARGO_PKG_VERSION").to_string(),
			description: Some("Grin Node JSON-RPC API".to_string()),
		}
	}
}

/// Generate OpenAPI specification from JSON-RPC endpoints
pub fn generate_openapi_spec(
	output_path: &str,
	format: &str,
) -> Result<(), Box<dyn std::error::Error>> {
	let mut spec = OpenApiSpec {
		openapi: "3.0.0".to_string(),
		info: Info::default(),
		paths: HashMap::new(),
		components: Components {
			schemas: HashMap::new(),
		},
	};

	// Add Foreign API endpoints
	let foreign_methods = collect_methods(
		include_str!("../../../api/src/foreign_rpc.rs"),
		"ForeignRpc",
	);
	create_rpc_endpoint(&mut spec, "/v2/foreign", foreign_methods);

	// Add Owner API endpoints
	let owner_methods = collect_methods(include_str!("../../../api/src/owner_rpc.rs"), "OwnerRpc");
	create_rpc_endpoint(&mut spec, "/v2/owner", owner_methods);

	// Write spec to file
	let output = match format {
		"yaml" => serde_yaml::to_string(&spec)?,
		"json" => serde_json::to_string_pretty(&spec)?,
		_ => return Err("Unsupported format. Use 'json' or 'yaml'.".into()),
	};

	let mut file = File::create(Path::new(output_path))?;
	file.write_all(output.as_bytes())?;

	Ok(())
}
