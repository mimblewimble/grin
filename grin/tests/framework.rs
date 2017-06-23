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
extern crate futures_cpupool;


use std::thread;
use std::time;
use std::default::Default;
use std::mem;
use std::fs;
use std::sync::{Arc, Mutex, RwLock};

use futures::{Future};
use futures::future::join_all;
use futures::task::park;
use tokio_core::reactor;
use tokio_core::reactor::Remote;
use tokio_core::reactor::Handle;
use tokio_timer::Timer;

use secp::Secp256k1;
use tiny_keccak::Keccak;

use wallet::WalletConfig;


/// Just removes all results from previous runs

pub fn clean_all_output(){
    let target_dir = format!("target/test_servers");
    fs::remove_dir_all(target_dir);
}

/// Errors that can be returned by LocalServerContainer
#[derive(Debug)]
pub enum Error {
	Internal(String),
	Argument(String),
	NotFound,
}

/// All-in-one server configuration struct, for convenience
/// 

#[derive(Clone)]
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

    //Port the wallet server is running on
    pub wallet_port: u16,

    //Whether we're going to mine
    pub start_miner: bool,

    //Whether we're going to run a wallet as well,
    //can use same server instance as a validating node for convenience
    pub start_wallet: bool,

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
            wallet_port: 13416,
            start_miner: false,
            start_wallet: false,
            burn_mining_rewards: false,
            coinbase_wallet_address: String::from(""),
            wallet_validating_node_url: String::from(""),
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
    //event_loop: Option<Ref<reactor::Core>>,

    //The grin server instance
    //pub p2p_server: Option<Ref<grin::Server>>,

    //The API server instance
    api_server: Option<api::ApiServer>,

    //whether the server is running
    pub server_is_running: bool,

    //Whether the server is mining
    pub server_is_mining: bool,

    //Whether the server is also running a wallet
    //Not used if running wallet without server
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
            //event_loop: None,
            //p2p_server: None,
            api_server: None,
            server_is_running: false,
            server_is_mining: false,
            wallet_is_running: false,
            working_dir: working_dir,
       }))
    }

    pub fn run_server(&mut self,
                         duration_in_seconds: u64) -> grin::Server
    {

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


        if self.config.start_wallet == true{
            self.run_wallet(duration_in_seconds+5);
            //give half a second to start before continuing
            thread::sleep(time::Duration::from_millis(500));
        }

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

        let timeout = Timer::default().sleep(time::Duration::from_secs(duration_in_seconds));

        event_loop.run(timeout);

        if self.wallet_is_running{
            self.stop_wallet();
        }

        //return a remote handle to the result of the run, so it can be accessed from the main
        //running thread
        s

    }
        
    /// Starts a wallet daemon to receive and returns the
    /// listening server url
    
    pub fn run_wallet(&mut self, duration_in_seconds: u64) {

        //URL on which to start the wallet listener (i.e. api server)
      	let url = format!("{}:{}", self.config.base_addr, self.config.wallet_port);
                
        //Just use the name of the server for a seed for now
        let seed = format!("{}", self.config.name);

	    let mut sha3 = Keccak::new_sha3_256();
	    sha3.update(seed.as_bytes());
	    let mut seed = [0; 32];
	    sha3.finalize(&mut seed);

	    let s = Secp256k1::new();
	    let key = wallet::ExtendedKey::from_seed(&s, &seed[..])
		         .expect("Error deriving extended key from seed.");
        
        println!("Starting the Grin wallet receiving daemon on {} ", self.config.wallet_port );

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

        self.api_server = Some(api_server);
        self.wallet_is_running = true;

    }
    
    /// Stops the running wallet server
    
    pub fn stop_wallet(&mut self){
        let mut api_server = self.api_server.as_mut().unwrap();
        api_server.stop();
    }
    
}

/// Configuration values for container pool

pub struct LocalServerContainerPoolConfig {
    //Base name to append to all the servers in this pool
    pub base_name: String,

    //Base http address for all of the servers in this pool
    pub base_http_addr: String,

    //Base port server for all of the servers in this pool
    //Increment the number by 1 for each new server
    pub base_p2p_port: u16,

    //Base api port for all of the servers in this pool
    //Increment this number by 1 for each new server
    pub base_api_port: u16,

    //Base wallet port for this server
    //
    pub base_wallet_port: u16,

    //How long the servers in the pool are going to run
    pub run_length_in_seconds: u64,


}

/// Default server config
///
impl Default for LocalServerContainerPoolConfig {
    fn default() -> LocalServerContainerPoolConfig {
        LocalServerContainerPoolConfig {
            base_name: String::from("test_pool"),
            base_http_addr: String::from("127.0.0.1"),
            base_p2p_port: 10000,
            base_api_port: 11000,
            base_wallet_port: 12000,
            run_length_in_seconds: 30,
        }
    }
}

/// A convenience pool for running many servers simultaneously
/// without necessarily having to configure each one manually

pub struct LocalServerContainerPool {
    //configuration
    pub config: LocalServerContainerPoolConfig,

    //keep ahold of all the created servers thread-safely
    pub server_containers: Vec<Arc<grin::Server>>,

    //Keep track of what the last ports a server was opened on
    next_p2p_port: u16,

    next_api_port: u16,

    next_wallet_port: u16,

    //A cpu pool, as we're going to try to keep each server within it's
    //own thread on a CPU core
    pool: futures_cpupool::CpuPool,

    //keep track of futures to run in one go as we add servers
    server_futures: Vec<futures_cpupool::CpuFuture<(), ()>>,

    //keep track of peers that are (will be) running
    peer_list: Vec<String>,
}

impl LocalServerContainerPool {

    pub fn new(config: LocalServerContainerPoolConfig)->LocalServerContainerPool{
        (LocalServerContainerPool{
            next_api_port: config.base_api_port,
            next_p2p_port: config.base_p2p_port,
            next_wallet_port: config.base_wallet_port,
            config: config,
            server_containers: Vec::new(),
            pool: futures_cpupool::Builder::new().create(),
            server_futures: Vec::new(),
            peer_list: Vec::new(),

        })
    }

    /// adds a single server on the next available port
    /// overriding passed-in values as necessary
    ///

    pub fn create_server(&mut self, mut server_config:LocalServerContainerConfig)
    {

        //If we're calling it this way, need to override these
        server_config.p2p_server_port=self.next_p2p_port;
        server_config.api_server_port=self.next_api_port;
        server_config.wallet_port=self.next_wallet_port;

        server_config.name=String::from(format!("{}/{}-{}",
                                                self.config.base_name,
                                                self.config.base_name,
                                                server_config.p2p_server_port));


        //Use self as coinbase wallet
        if server_config.coinbase_wallet_address.len()==0 {
            server_config.coinbase_wallet_address=String::from(format!("http://{}:{}",
                    server_config.base_addr,
                    server_config.wallet_port));
        }

        self.next_p2p_port+=1;
        self.next_api_port+=1;
        self.next_wallet_port+=1;

        let server_address = format!("{}:{}",
                                     server_config.base_addr,
                                     server_config.p2p_server_port);

        self.peer_list.push(server_address);

        let mut server = LocalServerContainer::new(server_config).unwrap();
        //self.server_containers.push(server_arc);

        //Create a future that runs the server for however many seconds
        //collect them all and run them in the run_all_servers
        let run_time = self.config.run_length_in_seconds;

        let future=self.pool.spawn_fn(move || {
            let result_server=server.run_server(run_time);

            println!("peer count: {}", result_server.peer_count());

            //STUMBLING BLOCK HERE
            //There doesn't appear to be any way to collect the grin::Servers from these
            //separate spawned threads as the struct contains a reactor::Handle, which under
            //no circumstances is allowed to share any data across threads

            /*self.server_containers.push(Arc::new(result_server));
            self.result_server_handles.push(result_handle);*/

            let result: Result<_, ()> = Ok(());
            result
        });

        self.server_futures.push(future);

    }

    /// adds n servers, ready to run
    ///
    ///

    pub fn create_servers(&mut self, number: u16){
        for n in 0..number {
            //self.create_server();
        }
    }


    /// runs all servers, calling the closure when all servers
    /// have reported that they're done
    ///

    pub fn run_all_servers(self){
        join_all(self.server_futures).wait();
    }

    pub fn connect_all_peers(&self){

        //s.server.connect_peer(a.parse().unwrap()).unwrap();

    }

}