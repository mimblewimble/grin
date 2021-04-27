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

//! Comments for configuration + injection into output .toml
use std::collections::HashMap;

/// maps entries to Comments that should precede them
fn comments() -> HashMap<String, String> {
	let mut retval = HashMap::new();
	retval.insert(
		"config_file_version".to_string(),
		"
# Generated Server Configuration File for Grin
#
# When running the grin executable without specifying any command line
# arguments, it will look for this file in two places, in the following
# order:
#
# -The working directory
# -[user home]/.grin
#

"
		.to_string(),
	);

	retval.insert(
		"[server]".to_string(),
		"#########################################
### SERVER CONFIGURATION              ###
#########################################

#Server connection details
"
		.to_string(),
	);

	retval.insert(
		"api_http_addr".to_string(),
		"
#path of TLS certificate file, self-signed certificates are not supported
#tls_certificate_file = \"\"
#private key for the TLS certificate
#tls_certificate_key = \"\"

#the address on which services will listen, e.g. Transaction Pool
"
		.to_string(),
	);

	retval.insert(
		"api_secret_path".to_string(),
		"
#path of the secret token used by the Rest API and v2 Owner API to authenticate the calls
#comment the it to disable basic auth
"
		.to_string(),
	);

	retval.insert(
		"foreign_api_secret_path".to_string(),
		"
#path of the secret token used by the Foreign API to authenticate the calls
#comment the it to disable basic auth
"
		.to_string(),
	);

	retval.insert(
		"db_root".to_string(),
		"
#the directory, relative to current, in which the grin blockchain
#is stored
"
		.to_string(),
	);

	retval.insert(
		"chain_type".to_string(),
		"
#The chain type, which defines the genesis block and the set of cuckoo
#parameters used for mining as well as wallet output coinbase maturity. Can be:
#AutomatedTesting - For CI builds and instant blockchain creation
#UserTesting - For regular user testing (cuckoo 16)
#Testnet - For the long term test network
#Mainnet - For mainnet
"
		.to_string(),
	);

	retval.insert(
		"future_time_limit".to_string(),
		"
#The Future Time Limit (FTL) is a limit on how far into the future,
#relative to a node's local time, the timestamp on a new block can be,
#in order for the block to be accepted.
#At Hard Fork 4, this was reduced from 12 minutes down to 5 minutes,
#so as to limit possible timestamp manipulation on the new
#wtema difficulty adjustment algorithm
"
		.to_string(),
	);

	retval.insert(
		"chain_validation_mode".to_string(),
		"
#the chain validation mode, defines how often (if at all) we
#want to run a full chain validation. Can be:
#\"EveryBlock\" - run full chain validation when processing each block (except during sync)
#\"Disabled\" - disable full chain validation (just run regular block validation)
"
		.to_string(),
	);

	retval.insert(
		"archive_mode".to_string(),
		"
#run the node in \"full archive\" mode (default is fast-sync, pruned node)
"
		.to_string(),
	);

	retval.insert(
		"skip_sync_wait".to_string(),
		"
#skip waiting for sync on startup, (optional param, mostly for testing)
"
		.to_string(),
	);

	retval.insert(
		"run_tui".to_string(),
		"
#whether to run the ncurses TUI (Ncurses must be installed)
"
		.to_string(),
	);

	retval.insert(
		"run_test_miner".to_string(),
		"
#Whether to run a test miner. This is only for developer testing (chaintype
#usertesting) at cuckoo 16, and will only mine into the default wallet port.
#real mining should use the standalone grin-miner
"
		.to_string(),
	);

	retval.insert(
		"[server.webhook_config]".to_string(),
		"
#########################################
### WEBHOOK CONFIGURATION             ###
#########################################
"
		.to_string(),
	);

	retval.insert(
		"nthreads".to_string(),
		"
#The url where a POST request will be sent when a new block is accepted by our node.
#block_accepted_url = \"http://127.0.0.1:8080/acceptedblock\"

#The url where a POST request will be sent when a new transaction is received by a peer.
#tx_received_url = \"http://127.0.0.1:8080/tx\"

#The url where a POST request will be sent when a new header is received by a peer.
#header_received_url = \"http://127.0.0.1:8080/header\"

#The url where a POST request will be sent when a new block is received by a peer.
#block_received_url = \"http://127.0.0.1:8080/block\"

#The number of worker threads that will be assigned to making the http requests.
"
		.to_string(),
	);

	retval.insert(
		"timeout".to_string(),
		"
#The timeout of the http request in seconds.
"
		.to_string(),
	);

	retval.insert(
		"[server.dandelion_config]".to_string(),
		"
#########################################
### DANDELION CONFIGURATION           ###
#########################################
"
		.to_string(),
	);

	retval.insert(
		"epoch_secs".to_string(),
		"
#dandelion epoch duration
"
		.to_string(),
	);

	retval.insert(
		"aggregation_secs".to_string(),
		"
#dandelion aggregation period in secs
"
		.to_string(),
	);

	retval.insert(
		"embargo_secs".to_string(),
		"
#fluff and broadcast after embargo expires if tx not seen on network
"
		.to_string(),
	);

	retval.insert(
		"stem_probability".to_string(),
		"
#dandelion stem probability (stem 90% of the time, fluff 10% of the time)
"
		.to_string(),
	);

	retval.insert(
		"always_stem_our_txs".to_string(),
		"
#always stem our (pushed via api) txs regardless of stem/fluff epoch (as per Dandelion++ paper)
"
		.to_string(),
	);

	retval.insert(
		"[server.p2p_config]".to_string(),
		"#test miner wallet URL (burns if this doesn't exist)
#test_miner_wallet_url = \"http://127.0.0.1:3415\"

#########################################
### SERVER P2P CONFIGURATION          ###
#########################################
#The P2P server details (i.e. the server that communicates with other
"
		.to_string(),
	);

	retval.insert(
		"host".to_string(),
		"
#The interface on which to listen.
#0.0.0.0 will listen on all interfaces, allowing others to interact
#127.0.0.1 will listen on the local machine only
"
		.to_string(),
	);

	retval.insert(
		"port".to_string(),
		"
#The port on which to listen.
"
		.to_string(),
	);

	retval.insert(
		"seeding_type".to_string(),
		"
#All seeds/peers can be either IP address or DNS names. Port number must always be specified
#how to seed this server, can be None, List or DNSSeed
"
		.to_string(),
	);

	retval.insert(
		"[server.pool_config]".to_string(),
		"#If the seeding type is List, the list of peers to connect to can
#be specified as follows:
#seeds = [\"192.168.0.1:3414\",\"192.168.0.2:3414\"]

#hardcoded peer lists for allow/deny
#will *only* connect to peers in allow list
#peers_allow = [\"192.168.0.1:3414\", \"192.168.0.2:3414\"]
#will *never* connect to peers in deny list
#peers_deny = [\"192.168.0.3:3414\", \"192.168.0.4:3414\"]
#a list of preferred peers to connect to
#peers_preferred = [\"192.168.0.1:3414\",\"192.168.0.2:3414\"]

#how long a banned peer should stay banned
#ban_window = 10800

#maximum number of inbound peer connections
#peer_max_inbound_count = 128

#maximum number of outbound peer connections
#peer_max_outbound_count = 8

#preferred minimum number of outbound peers (we'll actively keep trying to add peers
#until we get to at least this number)
#peer_min_preferred_outbound_count = 8

#amount of incoming connections temporarily allowed to exceed peer_max_inbound_count
#peer_listener_buffer_count = 8

# A preferred dandelion_peer, mainly used for testing dandelion
# dandelion_peer = \"10.0.0.1:13144\"

#########################################
### MEMPOOL CONFIGURATION             ###
#########################################
"
		.to_string(),
	);

	retval.insert(
		"accept_fee_base".to_string(),
		"
#base fee that's accepted into the pool
"
		.to_string(),
	);

	retval.insert(
		"reorg_cache_period".to_string(),
		"
#reorg cache retention period in minute.
#the reorg cache repopulates local mempool in a reorg scenario.
"
		.to_string(),
	);

	retval.insert(
		"max_pool_size".to_string(),
		"
#maximum number of transactions allowed in the pool
"
		.to_string(),
	);

	retval.insert(
		"max_stempool_size".to_string(),
		"
#maximum number of transactions allowed in the stempool
"
		.to_string(),
	);

	retval.insert(
		"mineable_max_weight".to_string(),
		"
#maximum total weight of transactions that can get selected to build a block
"
		.to_string(),
	);

	retval.insert(
		"[server.stratum_mining_config]".to_string(),
		"
################################################
### STRATUM MINING SERVER CONFIGURATION      ###
################################################
"
		.to_string(),
	);

	retval.insert(
		"enable_stratum_server".to_string(),
		"
#whether stratum server is enabled
"
		.to_string(),
	);

	retval.insert(
		"stratum_server_addr".to_string(),
		"
#what port and address for the stratum server to listen on
"
		.to_string(),
	);

	retval.insert(
		"attempt_time_per_block".to_string(),
		"
#the amount of time, in seconds, to attempt to mine on a particular
#header before stopping and re-collecting transactions from the pool
"
		.to_string(),
	);

	retval.insert(
		"minimum_share_difficulty".to_string(),
		"
#the minimum acceptable share difficulty to request from miners
"
		.to_string(),
	);

	retval.insert(
		"wallet_listener_url".to_string(),
		"
#the wallet receiver to which coinbase rewards will be sent
"
		.to_string(),
	);

	retval.insert(
		"burn_reward".to_string(),
		"
#whether to ignore the reward (mostly for testing)
"
		.to_string(),
	);

	retval.insert(
		"[logging]".to_string(),
		"
#########################################
### LOGGING CONFIGURATION             ###
#########################################
"
		.to_string(),
	);

	retval.insert(
		"log_to_stdout".to_string(),
		"
#whether to log to stdout
"
		.to_string(),
	);

	retval.insert(
		"stdout_log_level".to_string(),
		"
#log level for stdout: Error, Warning, Info, Debug, Trace
"
		.to_string(),
	);

	retval.insert(
		"log_to_file".to_string(),
		"
#whether to log to a file
"
		.to_string(),
	);

	retval.insert(
		"file_log_level".to_string(),
		"
#log level for file: Error, Warning, Info, Debug, Trace
"
		.to_string(),
	);

	retval.insert(
		"log_file_path".to_string(),
		"
#log file path
"
		.to_string(),
	);

	retval.insert(
		"log_file_append".to_string(),
		"
#whether to append to the log file (true), or replace it on every run (false)
"
		.to_string(),
	);

	retval.insert(
		"log_max_size".to_string(),
		"
#maximum log file size in bytes before performing log rotation
#comment it to disable log rotation
"
		.to_string(),
	);

	retval.insert(
		"log_max_files".to_string(),
		"
#maximum count of the log files to rotate over
"
		.to_string(),
	);

	retval
}

fn get_key(line: &str) -> String {
	if line.contains('[') && line.contains(']') {
		return line.to_owned();
	} else if line.contains('=') {
		return line.split('=').collect::<Vec<&str>>()[0].trim().to_owned();
	} else {
		return "NOT_FOUND".to_owned();
	}
}

pub fn insert_comments(orig: String) -> String {
	let comments = comments();
	let lines: Vec<&str> = orig.split('\n').collect();
	let mut out_lines = vec![];
	for l in lines {
		let key = get_key(l);
		if let Some(v) = comments.get(&key) {
			out_lines.push(v.to_owned());
		}
		out_lines.push(l.to_owned());
		out_lines.push("\n".to_owned());
	}
	let mut ret_val = String::from("");
	for l in out_lines {
		ret_val.push_str(&l);
	}
	ret_val
}
