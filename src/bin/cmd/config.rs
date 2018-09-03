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

/// Grin configuration file output command
use config::{GlobalConfig, GlobalWalletConfig};
use std::env;

/// Create a config file in the current directory
pub fn config_command_server(file_name: &str) {
	let mut default_config = GlobalConfig::default();
	let current_dir = env::current_dir().unwrap_or_else(|e| {
		panic!("Error creating config file: {}", e);
	});
	let mut config_file_name = current_dir.clone();
	config_file_name.push(file_name);
	if config_file_name.exists() {
		panic!(
			"{} already exists in the current directory. Please remove it first",
			file_name
		);
	}
	default_config.update_paths(&current_dir);
	default_config
		.write_to_file(config_file_name.to_str().unwrap())
		.unwrap_or_else(|e| {
			panic!("Error creating config file: {}", e);
		});

	println!(
		"{} file configured and created in current directory",
		file_name
	);
}

/// Create a config file in the current directory
pub fn config_command_wallet(file_name: &str) {
	let mut default_config = GlobalWalletConfig::default();
	let current_dir = env::current_dir().unwrap_or_else(|e| {
		panic!("Error creating config file: {}", e);
	});
	let mut config_file_name = current_dir.clone();
	config_file_name.push(file_name);
	if config_file_name.exists() {
		panic!(
			"{} already exists in the target directory. Please remove it first",
			file_name
		);
	}
	default_config.update_paths(&current_dir);
	default_config
		.write_to_file(config_file_name.to_str().unwrap())
		.unwrap_or_else(|e| {
			panic!("Error creating config file: {}", e);
		});

	println!(
		"File {} configured and created",
		config_file_name.to_str().unwrap(),
	);
}
