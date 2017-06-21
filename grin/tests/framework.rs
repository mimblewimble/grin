// Copyright 2017 The Grin Developers
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

extern crate grin_grin as grin;
extern crate grin_core as core;
extern crate grin_p2p as p2p;
extern crate grin_chain as chain;
extern crate grin_api as api;
extern crate grin_wallet as wallet;
extern crate secp256k1zkp as secp;
extern crate tiny_keccak;

extern crate env_logger;
extern crate futures;
extern crate tokio_core;
extern crate tokio_timer;

use std::thread;
use std::time;
use std::default::Default;
use std::mem;

use futures::{Future};
use futures::task::park;
use tokio_core::reactor;
use tokio_timer::Timer;

use secp::Secp256k1;
use tiny_keccak::Keccak;

use wallet::WalletConfig;


/// Errors that can be returned by LocalServerContainer
#[derive(Debug)]
pub enum Error {
	Internal(String),
	Argument(String),
	NotFound,
}

/// All-in-one server configuration struct, for convenience
/// 

pub struct LocalServerContainerConfig {
    
    //user friendly name for the server, also denotes what dir
    //the data files will appear in
    pub name: String,

    //Base IP address
    pub base_addr: String,

    //Port the server (p2p) is running on
    pub p2p_server_port: u16, 
    
    //Port the API server is running on
    pub api_server_port: u16,

    //Whether we're going to mine
    pub start_miner: bool,

    //Whether to burn mining rewards
    pub burn_mining_rewards: bool,

    //full address to send coinbase rewards to
    pub coinbase_wallet_address: String,

    //When running a wallet, the address to check inputs and send
    //finalised transactions to, 
    pub wallet_validating_node_url:String,
}

/// Default server config
impl Default for LocalServerContainerConfig {
	fn default() -> LocalServerContainerConfig {
		LocalServerContainerConfig {
			name: String::from("test_host"),
			base_addr: String::from("127.0.0.1"),
            p2p_server_port: 13414,
            api_server_port: 13415,
            start_miner: false,
            burn_mining_rewards: false,
            coinbase_wallet_address: String::from("http://127.0.0.1:13001"),
            wallet_validating_node_url: String::from("http://127.0.0.1:13415"),
		}
	}
}


/// A top-level container to hold everything that might be running
/// on a server, i.e. server, wallet in send or receive mode

pub struct LocalServerContainer {

    //Configuration
    config: LocalServerContainerConfig,

    //keep our own event loop, so each server can be
    //spun up/down at different times 
    event_loop: Option<reactor::Core>,

    //The grin server instance
    pub p2p_server: Option<grin::Server>,

    //The API server instance
    api_server: Option<api::ApiServer>,

    //whether the server is running
    pub server_is_running: bool,

    //Whether the server is mining
    pub server_is_mining: bool,

    //Whether the server is running a wallet
    pub wallet_is_running: bool,
    
    //base directory for the server instance
    working_dir: String,

}

impl LocalServerContainer {

    /// Create a new local server container with defaults, with the given name
    /// all related files will be created in the directory target/test_servers/{name}

    pub fn new(config:LocalServerContainerConfig) -> Result<LocalServerContainer, Error> {
        let working_dir = format!("target/test_servers/{}", config.name);
        Ok((LocalServerContainer {
            config:config,
            event_loop: None,
            p2p_server: None,
            api_server: None,
            server_is_running: false,
            server_is_mining: false,
            wallet_is_running: false,
            working_dir: working_dir,
       }))
    }

    pub fn run_server<F>(&mut self, 
                         duration_in_seconds: u64,
                         f:F) where 
    F: Fn() {

        let mut event_loop = reactor::Core::new().unwrap();

        let api_addr = format!("{}:{}", self.config.base_addr, self.config.api_server_port);
        
        let s = grin::Server::future(
            grin::ServerConfig{
                api_http_addr: api_addr,
                db_root: format!("{}/.grin", self.working_dir),
                cuckoo_size: 12,
                p2p_config: p2p::P2PConfig{port: self.config.p2p_server_port, ..p2p::P2PConfig::default()},
                ..Default::default()
            }, &event_loop.handle()).unwrap();

            

        let mut miner_config = grin::MinerConfig {
            enable_mining: self.config.start_miner,
            burn_reward: self.config.burn_mining_rewards,
            wallet_receiver_url : self.config.coinbase_wallet_address.clone(),
            ..Default::default()
        };

        if self.config.start_miner == true {
            println!("starting Miner on port {}", self.config.p2p_server_port);
            s.start_miner(miner_config);        
        }
        
        event_loop.run(Timer::default().sleep(time::Duration::from_secs(duration_in_seconds)).and_then(|_| {
            if self.wallet_is_running {
                self.stop_wallet();
            }
            f();
            Ok(())
        }));

        self.p2p_server = Some(s);
        self.event_loop = Some(event_loop);

    }
        
    /// Starts a wallet daemon to receive and returns the
    /// listening server url
    
    pub fn run_wallet<F>(&mut self, f:F) where 
    F: Fn() {
      
        //URL on which to start the wallet listener (i.e. api server)
      	let url = format!("{}:{}", self.config.base_addr, self.config.api_server_port);
                
        //Just use the name of the server for a seed for now
        let seed = format!("{}", self.config.name);

	    let mut sha3 = Keccak::new_sha3_256();
	    sha3.update(seed.as_bytes());
	    let mut seed = [0; 32];
	    sha3.finalize(&mut seed);

	    let s = Secp256k1::new();
	    let key = wallet::ExtendedKey::from_seed(&s, &seed[..])
		         .expect("Error deriving extended key from seed.");
        
        println!("Starting the Grin wallet receiving daemon on {} ", self.config.api_server_port );

        let mut wallet_config = WalletConfig::default();
        
        wallet_config.api_http_addr = format!("http://{}", url);
        wallet_config.check_node_api_http_addr = self.config.wallet_validating_node_url.clone();
        wallet_config.data_file_dir=self.working_dir.clone();        

        let mut api_server = api::ApiServer::new("/v1".to_string());
	  
	    api_server.register_endpoint("/receive".to_string(), wallet::WalletReceiver { 
            key: key,
            config: wallet_config,
        });

		api_server.start(url).unwrap_or_else(|e| {
		    println!("Failed to start Grin wallet receiver: {}.", e);
		});

        self.api_server=Some(api_server);

        self.wallet_is_running = true;

        //let time_out=tokio_core::reactor::TimeOut::new()
    }
    
    /// Stops the running wallet server
    
    pub fn stop_wallet(&mut self){
        let mut api_server = self.api_server.as_mut().unwrap();
        api_server.stop();
    }
    
}

/*pub struct LocalServerContainerPool {
    
    //Base http address for all of the servers in this pool
    base_http_addr: String, 

    //Base port server for all of the servers in this pool
    //Increment the number by 1 for each new server
    base_port_p2p: u16,

    //Base api port for all of the servers in this pool
    //Increment this number by 1 for each new server
    base_port_api: u16,
    
    //All of the servers
    pub server_containers: Vec<LocalServerContainer>,    
}

impl LocalServerContainerPool {

    pub fn new(base_http_addr:String, base_port_server: u16, base_port_api: u16)->Result<LocalServerContainerPool, Error>{
        LocalServerContainerPool{
            base_http_addr: base_http_addr,
            base_port_p2p: base_port_p2p,
            base_port_api: base_port_api,
            server_containers: Vec::new(),
        }

    }
    
    pub fn create_server(&mut self, enable_mining:bool, enable_wallet:bool ) {

        let server_port = self.base_port_server+self.server_containers.len() as u16;
        let api_port = self.base_port_api+self.server_containers.len() as u16;
        
        let api_addr = format!("{}:{}", self.base_http_addr, api_port);

        let mut server_container = LocalServerContainer::new(api_addr, server_port, &self.event_loop).unwrap();
            
        server_container.enable_mining = enable_mining;
        server_container.enable_wallet = enable_wallet;

        //if we want to start a wallet, use this port
        server_container.wallet_port = self.base_port_wallet+self.server_containers.len() as u16;
        
        self.server_containers.push(server_container);
    }

    /// Connects every server to each other as peers
    /// 

    pub fn connect_all_peers(&self){
        /// just pull out all currently active servers, build a list,
        /// and feed into all servers

        let mut server_addresses:Vec<String> = Vec::new();      
        for s in &self.server_containers {
            let server_address = format!("{}:{}", 
                                     s.server.config.p2p_config.host, 
                                     s.server.config.p2p_config.port);
            server_addresses.push(server_address);
        }

        for a in server_addresses {
           for s in &self.server_containers {
              if format!("{}", s.server.config.p2p_config.host) != a {
                  s.server.connect_peer(a.parse().unwrap()).unwrap();       
              } 
           }
        }
    }

    ///Starts all servers, with or without mining
    ///TODO: This should accept a closure so tests can determine what 
    ///to do when the run is finished

    pub fn run_all_servers<F>(&mut self, f:F) where 
        F: Fn() {
            for s in &mut self.server_containers {
                let mut wallet_url = String::from("http://localhost:13416");
                if s.enable_wallet == true {
                wallet_url=s.start_wallet();
                //Instead of making all sorts of changes to the api server
                //to support futures, just going to pause this thread for 
                //half a second for the wallet to start
                //before continuing

                thread::sleep(time::Duration::from_millis(500));
                }
                let mut miner_config = grin::MinerConfig{
                    enable_mining: true,
                    burn_reward: true,
                    wallet_receiver_url : format!("http://{}", wallet_url),
                    ..Default::default()
                };
                if s.enable_wallet == true {
                    miner_config.burn_reward = false;
                }
                if s.enable_mining == true {
                    println!("starting Miner on port {}", s.server.config.p2p_config.port);
                    s.server.start_miner(miner_config);        
                }
                
            }

            //borrow copy to allow access in closure
            let mut server_containers = mem::replace(&mut self.server_containers, Vec::new());
            //let &mut server_containers = self.server_containers;

            self.event_loop.run(Timer::default().sleep(time::Duration::from_secs(30)).and_then(|_| {
                //Stop any assocated wallet servers
                for s in &mut server_containers {
                    if s.wallet_is_running {
                        s.stop_wallet();
                    }
                }
                f();
                Ok(())

            }));
            //}));
            //}
            //for s in &mut self.server_containers {  
                // occasionally 2 peers will connect to each other at the same time
                //assert!(s.peer_count() >= 4);
            //}
            //Ok(())
            //});
        
        }
}*/