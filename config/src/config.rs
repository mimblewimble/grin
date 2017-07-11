// Copyright 2016 The Grin Developers
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

//! Configuration file management

use toml;
use grin::{ServerConfig,
           MinerConfig};
use wallet::WalletConfig;

/// Going to hold all of the various configuration types 
/// separately for now, then put them together as a single
/// ServerConfig object afterwards. This is to flatten 
/// out the configuration file into logical sections,
/// as they tend to be quite nested in the code
/// Most structs optional, as they may or may not
/// be needed depending on what's being run

#[derive(Debug, Deserialize)]
pub struct GlobalConfig {
    server: Option<ServerConfig>,
    mining: Option<MinerConfig>,
    wallet: Option<WalletConfig>
}

#[test]
fn read_config() {
    let toml_str = r#"
        #Section is optional, if not here or enable_server is false, will only run wallet
        [server]
        enable_server = true
        api_http_addr = "127.0.0.1"
        db_root = "."
        seeding_type = "None"
        test_mode = false
        #7 = FULL_NODE, not sure how to serialise this properly to use constants
        capabilities = [7]
        
        [server.p2p_config]
        host = "127.0.0.1"
        port = 13414
        
        #Mining section is optional, if it's not here it will default to not mining
        [mining]
        enable_mining = true
        wallet_receiver_url = "http://127.0.0.1:13415"
        burn_reward = false
        #testing value, optional
        #slow_down_in_millis = 30
        #testing value, should really be removed and read from consensus instead, optional
        #cuckoo_size = 12

        #Wallet section is optional. If it's not here, server won't run a wallet
        [wallet]
        #whether to run a wallet
        enable_wallet = true
        #the address on which to run the wallet listener
        api_http_addr = "http://127.0.0.1:13415"
        #the address of a listening node to send finalised transactions to
		check_node_api_http_addr = "http://127.0.0.1:13415"
        #The location of the wallet.dat file
		data_file_dir = "."

    "#;

    let mut decoded: GlobalConfig = toml::from_str(toml_str).unwrap();
    decoded.server.as_mut().unwrap().mining_config = decoded.mining;
    println!("Decoded.server: {:?}", decoded.server);
    println!("Decoded wallet: {:?}", decoded.wallet);
    panic!("panic");
}