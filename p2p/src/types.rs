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

use crate::util::RwLock;
use std::convert::From;
use std::fs::File;
use std::io::{self, Read, Write};
use std::net::{
	IpAddr, Ipv4Addr, Ipv6Addr, Shutdown, SocketAddr, SocketAddrV4, SocketAddrV6, TcpStream,
};
use std::path::PathBuf;
use std::str;
use std::time::Duration;

use std::sync::mpsc;
use std::sync::Arc;

use chrono::prelude::*;
use enum_primitive::FromPrimitive;
use i2p::net::{I2pAddr, I2pSocketAddr, I2pStream};

use crate::chain;
use crate::core::core;
use crate::core::core::hash::Hash;
use crate::core::global;
use crate::core::pow::Difficulty;
use crate::core::ser::{self, ProtocolVersion, Readable, Reader, Writeable, Writer};
use grin_store;

/// Maximum number of block headers a peer should ever send
pub const MAX_BLOCK_HEADERS: u32 = 512;

/// Maximum number of block bodies a peer should ever ask for and send
#[allow(dead_code)]
pub const MAX_BLOCK_BODIES: u32 = 16;

/// Maximum number of peer addresses a peer should ever send
pub const MAX_PEER_ADDRS: u32 = 256;

/// Maximum number of block header hashes to send as part of a locator
pub const MAX_LOCATORS: u32 = 20;

/// Just enough to allow .b32.i2p addresses. We only accept vanity addresses
/// (like igno.i2p) that are shorter or just as long.
const MAX_I2P_ADDR_LENGTH: usize = 60;

/// How long a banned peer should be banned for
const BAN_WINDOW: i64 = 10800;

/// The max peer count
const PEER_MAX_COUNT: u32 = 125;

/// min preferred peer count
const PEER_MIN_PREFERRED_COUNT: u32 = 8;

#[derive(Debug)]
pub enum Error {
	Serialization(ser::Error),
	Connection(io::Error),
	I2p(i2p::Error),
	Socket(io::Error),
	/// Header type does not match the expected message type
	BadMessage,
	MsgLen,
	Banned,
	ConnectionClose,
	Timeout,
	Store(grin_store::Error),
	Chain(chain::Error),
	PeerWithSelf,
	NoDandelionRelay,
	GenesisMismatch {
		us: Hash,
		peer: Hash,
	},
	Send(String),
	PeerException,
	Internal,
}

impl From<ser::Error> for Error {
	fn from(e: ser::Error) -> Error {
		Error::Serialization(e)
	}
}
impl From<grin_store::Error> for Error {
	fn from(e: grin_store::Error) -> Error {
		Error::Store(e)
	}
}
impl From<chain::Error> for Error {
	fn from(e: chain::Error) -> Error {
		Error::Chain(e)
	}
}
impl From<io::Error> for Error {
	fn from(e: io::Error) -> Error {
		Error::Connection(e)
	}
}
impl From<i2p::Error> for Error {
	fn from(e: i2p::Error) -> Error {
		Error::I2p(e)
	}
}
impl<T> From<mpsc::TrySendError<T>> for Error {
	fn from(e: mpsc::TrySendError<T>) -> Error {
		Error::Send(e.to_string())
	}
}

/// The address of a peer, whether local or remote. Wraps the underlying socket
/// address, whether it's I2P or IP, allowing us to handle both in the same
/// way. Mostly boilerplate manipulating SocketAddr or I2pSocketAddr
/// appropriately.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "peer_type", content = "peer_addr")]
pub enum PeerAddr {
	Socket(SocketAddr),
	I2p(I2pSocketAddr),
}

enum_from_primitive! {
	#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
  pub enum PeerAddrType {
	IPv4 = 0,
	IPv6 = 1,
	I2p = 2,
  }
}

impl Writeable for PeerAddr {
	fn write<W: Writer>(&self, writer: &mut W) -> Result<(), ser::Error> {
		match self {
			PeerAddr::Socket(SocketAddr::V4(sav4)) => {
				ser_multiwrite!(
					writer,
					[write_u8, PeerAddrType::IPv4 as u8],
					[write_fixed_bytes, &sav4.ip().octets().to_vec()],
					[write_u16, sav4.port()]
				);
			}
			PeerAddr::Socket(SocketAddr::V6(sav6)) => {
				writer.write_u8(PeerAddrType::IPv6 as u8)?;
				for seg in &sav6.ip().segments() {
					writer.write_u16(*seg)?;
				}
				writer.write_u16(sav6.port())?;
			}
			PeerAddr::I2p(i2p_addr) => {
				ser_multiwrite!(
					writer,
					[write_u8, PeerAddrType::I2p as u8],
					[write_bytes, &i2p_addr.dest().to_string().into_bytes()],
					[write_u16, i2p_addr.port()]
				);
			}
		}
		Ok(())
	}
}

impl Readable for PeerAddr {
	fn read(reader: &mut dyn Reader) -> Result<PeerAddr, ser::Error> {
		if let Some(addr_format) = PeerAddrType::from_u8(reader.read_u8()?) {
			match addr_format {
				PeerAddrType::IPv4 => {
					let ip = reader.read_fixed_bytes(4)?;
					let port = reader.read_u16()?;
					Ok(PeerAddr::Socket(SocketAddr::V4(SocketAddrV4::new(
						Ipv4Addr::new(ip[0], ip[1], ip[2], ip[3]),
						port,
					))))
				}
				PeerAddrType::IPv6 => {
					let ip = try_iter_map_vec!(0..8, |_| reader.read_u16());
					let port = reader.read_u16()?;
					Ok(PeerAddr::Socket(SocketAddr::V6(SocketAddrV6::new(
						Ipv6Addr::new(ip[0], ip[1], ip[2], ip[3], ip[4], ip[5], ip[6], ip[7]),
						port,
						0,
						0,
					))))
				}
				PeerAddrType::I2p => {
					let addr = reader.read_bytes_len_prefix()?;
					if addr.len() > MAX_I2P_ADDR_LENGTH {
						return Err(ser::Error::TooLargeReadErr);
					}
					let addr_str = str::from_utf8(&addr).map_err(|_| ser::Error::CorruptedData)?;
					let port = reader.read_u16()?;
					Ok(PeerAddr::I2p(I2pSocketAddr::new(
						I2pAddr::new(&addr_str),
						port,
					)))
				}
			}
		} else {
			return Err(ser::Error::CorruptedData);
		}
	}
}

impl std::hash::Hash for PeerAddr {
	/// If loopback address then we care about ip and port.
	/// If regular address then we only care about the ip and ignore the port.
	fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
		match self {
			PeerAddr::Socket(s) => {
				if s.ip().is_loopback() {
					s.hash(state);
				} else {
					s.ip().hash(state);
				}
			}
			PeerAddr::I2p(i2p_addr) => i2p_addr.hash(state),
		}
	}
}

impl PartialEq for PeerAddr {
	/// If loopback address then we care about ip and port.
	/// If regular address then we only care about the ip and ignore the port.
	fn eq(&self, other: &PeerAddr) -> bool {
		match self {
			PeerAddr::Socket(s) => {
				if !other.is_ip() {
					return false;
				}
				if s.ip().is_loopback() {
					s == &other.clone().unwrap_ip().unwrap()
				} else {
					s.ip() == other.clone().unwrap_ip().unwrap().ip()
				}
			}
			PeerAddr::I2p(i2p_addr) => i2p_addr == &other.clone().unwrap_i2p().unwrap(),
		}
	}
}

impl Eq for PeerAddr {}

impl std::fmt::Display for PeerAddr {
	fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
		match self {
			PeerAddr::Socket(s) => write!(f, "{}", s),
			PeerAddr::I2p(i2p_addr) => write!(f, "{}", i2p_addr.to_string()),
		}
	}
}

impl PeerAddr {
	/// Convenient way of constructing a new peer_addr from an ip_addr
	/// defaults to port 3414 on mainnet and 13414 on floonet.
	pub fn from_ip(addr: IpAddr) -> PeerAddr {
		let port = if global::is_floonet() { 13414 } else { 3414 };
		PeerAddr::Socket(SocketAddr::new(addr, port))
	}

	/// Convenient way of constructing a new peer_addr from a SocketAddr
	pub fn from_socket_addr(addr: SocketAddr) -> PeerAddr {
		PeerAddr::Socket(addr)
	}

	/// If the ip is loopback then our key is "ip:port" (mainly for local usernet testing).
	/// Otherwise we only care about the ip (we disallow multiple peers on the same ip address).
	pub fn as_key(&self) -> String {
		match self {
			PeerAddr::Socket(s) => {
				if s.ip().is_loopback() {
					format!("{}:{}", s.ip(), s.port())
				} else {
					format!("{}", s.ip())
				}
			}
			PeerAddr::I2p(i2p_addr) => i2p_addr.to_string(),
		}
	}

	/// Whether this is an i2p address
	pub fn is_i2p(&self) -> bool {
		if let PeerAddr::I2p(_) = self {
			return true;
		}
		false
	}

	/// Whether this is an classic IP address
	pub fn is_ip(&self) -> bool {
		!self.is_i2p()
	}

	/// Returns the underlying I2P address if this is one, otherwise panics
	pub fn unwrap_i2p(self) -> Result<I2pSocketAddr, Error> {
		if let PeerAddr::I2p(i2p_addr) = self {
			return Ok(i2p_addr);
		} else {
			return Err(Error::I2p(i2p::Error::from(io::Error::new(
				io::ErrorKind::InvalidInput,
				"not a valid I2P address",
			))));
		}
	}

	/// Returns the underlying IP address if this is one, otherwise panics
	pub fn unwrap_ip(self) -> Result<SocketAddr, Error> {
		if let PeerAddr::Socket(s) = self {
			return Ok(s);
		} else {
			return Err(Error::Socket(io::Error::new(
				io::ErrorKind::InvalidInput,
				"not a valid IP address",
			)));
		}
	}
}

/// Representation of network stream, currently implemented for  I2pStream and TcpStream
pub trait Stream: Read + Write + Send + Sync {
	/// Peer address this stream is connected to
	fn peer_addr(&self) -> Result<PeerAddr, Error>;
	/// Our network address
	fn local_addr(&self) -> Result<PeerAddr, Error>;
	/// Shutdown connection
	fn shutdown(&self, how: Shutdown) -> Result<(), Error>;
	/// Enable non-blocking IO
	fn set_nonblocking(&self, nonblocking: bool) -> Result<(), Error>;
	/// Try clone stream
	fn try_clone(&self) -> Result<Box<Stream>, Error>;
	/// Set read timeout
	fn set_read_timeout(&self, duration: Option<Duration>) -> Result<(), Error>;
	/// Set write timeout
	fn set_write_timeout(&self, duration: Option<Duration>) -> Result<(), Error>;
}

impl<'a, S: Stream> Stream for &'a mut S {
	fn peer_addr(&self) -> Result<PeerAddr, Error> {
		S::peer_addr(self)
	}

	fn local_addr(&self) -> Result<PeerAddr, Error> {
		S::local_addr(self)
	}

	fn shutdown(&self, how: Shutdown) -> Result<(), Error> {
		S::shutdown(self, how)
	}

	fn set_nonblocking(&self, nonblocking: bool) -> Result<(), Error> {
		S::set_nonblocking(self, nonblocking)
	}
	fn try_clone(&self) -> Result<Box<Stream>, Error> {
		S::try_clone(self)
	}

	fn set_read_timeout(&self, duration: Option<Duration>) -> Result<(), Error> {
		S::set_read_timeout(self, duration)
	}

	fn set_write_timeout(&self, duration: Option<Duration>) -> Result<(), Error> {
		S::set_write_timeout(self, duration)
	}
}

impl Stream for TcpStream {
	fn peer_addr(&self) -> Result<PeerAddr, Error> {
		Ok(PeerAddr::Socket(self.peer_addr()?))
	}

	fn local_addr(&self) -> Result<PeerAddr, Error> {
		Ok(PeerAddr::Socket(self.local_addr()?))
	}

	fn shutdown(&self, how: Shutdown) -> Result<(), Error> {
		self.shutdown(how).map_err(|e| e.into())
	}

	fn set_nonblocking(&self, nonblocking: bool) -> Result<(), Error> {
		self.set_nonblocking(nonblocking).map_err(|e| e.into())
	}

	fn try_clone(&self) -> Result<Box<Stream>, Error> {
		let s = self.try_clone()?;
		Ok(Box::new(s))
	}

	fn set_read_timeout(&self, duration: Option<Duration>) -> Result<(), Error> {
		match self.set_read_timeout(duration) {
			Ok(t) => Ok(t),
			Err(e) => Err(Error::Socket(e).into()),
		}
	}

	fn set_write_timeout(&self, duration: Option<Duration>) -> Result<(), Error> {
		match self.set_write_timeout(duration) {
			Ok(t) => Ok(t),
			Err(e) => Err(Error::Socket(e).into()),
		}
	}
}

impl Stream for I2pStream {
	fn peer_addr(&self) -> Result<PeerAddr, Error> {
		Ok(PeerAddr::I2p(self.peer_addr()?))
	}

	fn local_addr(&self) -> Result<PeerAddr, Error> {
		Ok(PeerAddr::I2p(self.local_addr()?))
	}

	fn shutdown(&self, how: Shutdown) -> Result<(), Error> {
		self.shutdown(how).map_err(|e| e.into())
	}

	fn set_nonblocking(&self, nonblocking: bool) -> Result<(), Error> {
		self.set_nonblocking(nonblocking).map_err(|e| e.into())
	}

	fn try_clone(&self) -> Result<Box<Stream>, Error> {
		let s = self.try_clone()?;
		Ok(Box::new(s))
	}

	fn set_read_timeout(&self, duration: Option<Duration>) -> Result<(), Error> {
		match self.set_read_timeout(duration) {
			Ok(t) => Ok(t),
			Err(e) => Err(Error::I2p(e).into()),
		}
	}

	fn set_write_timeout(&self, duration: Option<Duration>) -> Result<(), Error> {
		match self.set_write_timeout(duration) {
			Ok(t) => Ok(t),
			Err(e) => Err(Error::I2p(e).into()),
		}
	}
}

/// I2P configuration, if enabled
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "mode", content = "i2p_config")]
pub enum I2pMode {
	/// No I2P, only use classic TCP
	Disabled,
	/// Enable I2P
	Enabled {
		/// Attempts to start i2pd with grin
		autostart: bool,
		/// Only connect through I2P, disable classic TCP
		exclusive: bool,
		/// Address of the I2P server
		addr: String,
	},
}

impl Default for I2pMode {
	fn default() -> I2pMode {
		I2pMode::Disabled
	}
}

/// Configuration for the peer-to-peer server.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct P2PConfig {
	pub host: IpAddr,
	pub port: u16,

	/// Method used to get the list of seed nodes for initial bootstrap.
	#[serde(default)]
	pub seeding_type: Seeding,

	/// The list of seed nodes, if using Seeding as a seed type
	pub seeds: Option<Vec<PeerAddr>>,

	/// Capabilities expose by this node, also conditions which other peers this
	/// node will have an affinity toward when connection.
	pub capabilities: Capabilities,

	pub peers_allow: Option<Vec<PeerAddr>>,

	pub peers_deny: Option<Vec<PeerAddr>>,

	/// The list of preferred peers that we will try to connect to
	pub peers_preferred: Option<Vec<PeerAddr>>,

	pub ban_window: Option<i64>,

	pub peer_max_count: Option<u32>,

	pub peer_min_preferred_count: Option<u32>,

	pub dandelion_peer: Option<PeerAddr>,

	/// Mode of use and configuration for i2p
	pub i2p_mode: I2pMode,
}

/// Default address for peer-to-peer connections.
impl Default for P2PConfig {
	fn default() -> P2PConfig {
		let ipaddr = "0.0.0.0".parse().unwrap();
		P2PConfig {
			host: ipaddr,
			port: 3414,
			capabilities: Capabilities::FULL_NODE,
			seeding_type: Seeding::default(),
			seeds: None,
			peers_allow: None,
			peers_deny: None,
			peers_preferred: None,
			ban_window: None,
			peer_max_count: None,
			peer_min_preferred_count: None,
			dandelion_peer: None,
			i2p_mode: I2pMode::default(),
		}
	}
}

/// Note certain fields are options just so they don't have to be
/// included in grin-server.toml, but we don't want them to ever return none
impl P2PConfig {
	/// return ban window
	pub fn ban_window(&self) -> i64 {
		match self.ban_window {
			Some(n) => n,
			None => BAN_WINDOW,
		}
	}

	/// return peer_max_count
	pub fn peer_max_count(&self) -> u32 {
		match self.peer_max_count {
			Some(n) => n,
			None => PEER_MAX_COUNT,
		}
	}

	/// return peer_preferred_count
	pub fn peer_min_preferred_count(&self) -> u32 {
		match self.peer_min_preferred_count {
			Some(n) => n,
			None => PEER_MIN_PREFERRED_COUNT,
		}
	}
}

/// Type of seeding the server will use to find other peers on the network.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub enum Seeding {
	/// No seeding, mostly for tests that programmatically connect
	None,
	/// A list of seed addresses provided to the server
	List,
	/// Automatically get a list of seeds from multiple DNS
	DNSSeed,
	/// Mostly for tests, where connections are initiated programmatically
	Programmatic,
}

impl Default for Seeding {
	fn default() -> Seeding {
		Seeding::DNSSeed
	}
}

bitflags! {
	/// Options for what type of interaction a peer supports
	#[derive(Serialize, Deserialize)]
	pub struct Capabilities: u32 {
		/// We don't know (yet) what the peer can do.
		const UNKNOWN = 0b00000000;
		/// Can provide full history of headers back to genesis
		/// (for at least one arbitrary fork).
		const HEADER_HIST = 0b00000001;
		/// Can provide block headers and the TxHashSet for some recent-enough
		/// height.
		const TXHASHSET_HIST = 0b00000010;
		/// Can provide a list of healthy peers
		const PEER_LIST = 0b00000100;
		/// Can broadcast and request txs by kernel hash.
		const TX_KERNEL_HASH = 0b00001000;
		/// I2P addresses can be received and connected to
		const I2P_SUPPORTED = 0b00010000;

		/// All nodes right now are "full nodes".
		/// Some nodes internally may maintain longer block histories (archival_mode)
		/// but we do not advertise this to other nodes.
		/// All nodes by default will accept lightweight "kernel first" tx broadcast.
		const FULL_NODE = Capabilities::HEADER_HIST.bits
			| Capabilities::TXHASHSET_HIST.bits
			| Capabilities::PEER_LIST.bits
			| Capabilities::TX_KERNEL_HASH.bits
	  | Capabilities::I2P_SUPPORTED.bits;
	}
}

// Types of connection
enum_from_primitive! {
	#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
	pub enum Direction {
		Inbound = 0,
		Outbound = 1,
	}
}

// Ban reason
enum_from_primitive! {
	#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
	pub enum ReasonForBan {
		None = 0,
		BadBlock = 1,
		BadCompactBlock = 2,
		BadBlockHeader = 3,
		BadTxHashSet = 4,
		ManualBan = 5,
		FraudHeight = 6,
		BadHandshake = 7,
	}
}

#[derive(Clone, Debug)]
pub struct PeerLiveInfo {
	pub total_difficulty: Difficulty,
	pub height: u64,
	pub last_seen: DateTime<Utc>,
	pub stuck_detector: DateTime<Utc>,
	pub first_seen: DateTime<Utc>,
}

/// General information about a connected peer that's useful to other modules.
#[derive(Clone, Debug)]
pub struct PeerInfo {
	pub capabilities: Capabilities,
	pub user_agent: String,
	pub version: ProtocolVersion,
	pub addr: PeerAddr,
	pub direction: Direction,
	pub live_info: Arc<RwLock<PeerLiveInfo>>,
}

impl PeerLiveInfo {
	pub fn new(difficulty: Difficulty) -> PeerLiveInfo {
		PeerLiveInfo {
			total_difficulty: difficulty,
			height: 0,
			first_seen: Utc::now(),
			last_seen: Utc::now(),
			stuck_detector: Utc::now(),
		}
	}
}

impl PeerInfo {
	/// The current total_difficulty of the peer.
	pub fn total_difficulty(&self) -> Difficulty {
		self.live_info.read().total_difficulty
	}

	pub fn is_outbound(&self) -> bool {
		self.direction == Direction::Outbound
	}

	/// The current height of the peer.
	pub fn height(&self) -> u64 {
		self.live_info.read().height
	}

	/// Time of last_seen for this peer (via ping/pong).
	pub fn last_seen(&self) -> DateTime<Utc> {
		self.live_info.read().last_seen
	}

	/// Time of first_seen for this peer.
	pub fn first_seen(&self) -> DateTime<Utc> {
		self.live_info.read().first_seen
	}

	/// Update the total_difficulty, height and last_seen of the peer.
	/// Takes a write lock on the live_info.
	pub fn update(&self, height: u64, total_difficulty: Difficulty) {
		let mut live_info = self.live_info.write();
		if total_difficulty != live_info.total_difficulty {
			live_info.stuck_detector = Utc::now();
		}
		live_info.height = height;
		live_info.total_difficulty = total_difficulty;
		live_info.last_seen = Utc::now()
	}
}

/// Flatten out a PeerInfo and nested PeerLiveInfo (taking a read lock on it)
/// so we can serialize/deserialize the data for the API and the TUI.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PeerInfoDisplay {
	pub capabilities: Capabilities,
	pub user_agent: String,
	pub version: ProtocolVersion,
	pub addr: PeerAddr,
	pub direction: Direction,
	pub total_difficulty: Difficulty,
	pub height: u64,
}

impl From<PeerInfo> for PeerInfoDisplay {
	fn from(info: PeerInfo) -> PeerInfoDisplay {
		PeerInfoDisplay {
			capabilities: info.capabilities.clone(),
			user_agent: info.user_agent.clone(),
			version: info.version,
			addr: info.addr.clone(),
			direction: info.direction.clone(),
			total_difficulty: info.total_difficulty(),
			height: info.height(),
		}
	}
}

/// The full txhashset data along with indexes required for a consumer to
/// rewind to a consistent requested state.
pub struct TxHashSetRead {
	/// Output tree index the receiver should rewind to
	pub output_index: u64,
	/// Kernel tree index the receiver should rewind to
	pub kernel_index: u64,
	/// Binary stream for the txhashset zipped data
	pub reader: File,
}

/// Bridge between the networking layer and the rest of the system. Handles the
/// forwarding or querying of blocks and transactions from the network among
/// other things.
pub trait ChainAdapter: Sync + Send {
	/// Current total difficulty on our chain
	fn total_difficulty(&self) -> Result<Difficulty, chain::Error>;

	/// Current total height
	fn total_height(&self) -> Result<u64, chain::Error>;

	/// A valid transaction has been received from one of our peers
	fn transaction_received(&self, tx: core::Transaction, stem: bool)
		-> Result<bool, chain::Error>;

	fn get_transaction(&self, kernel_hash: Hash) -> Option<core::Transaction>;

	fn tx_kernel_received(
		&self,
		kernel_hash: Hash,
		peer_info: &PeerInfo,
	) -> Result<bool, chain::Error>;

	/// A block has been received from one of our peers. Returns true if the
	/// block could be handled properly and is not deemed defective by the
	/// chain. Returning false means the block will never be valid and
	/// may result in the peer being banned.
	fn block_received(
		&self,
		b: core::Block,
		peer_info: &PeerInfo,
		was_requested: bool,
	) -> Result<bool, chain::Error>;

	fn compact_block_received(
		&self,
		cb: core::CompactBlock,
		peer_info: &PeerInfo,
	) -> Result<bool, chain::Error>;

	fn header_received(
		&self,
		bh: core::BlockHeader,
		peer_info: &PeerInfo,
	) -> Result<bool, chain::Error>;

	/// A set of block header has been received, typically in response to a
	/// block
	/// header request.
	fn headers_received(
		&self,
		bh: &[core::BlockHeader],
		peer_info: &PeerInfo,
	) -> Result<bool, chain::Error>;

	/// Finds a list of block headers based on the provided locator. Tries to
	/// identify the common chain and gets the headers that follow it
	/// immediately.
	fn locate_headers(&self, locator: &[Hash]) -> Result<Vec<core::BlockHeader>, chain::Error>;

	/// Gets a full block by its hash.
	fn get_block(&self, h: Hash) -> Option<core::Block>;

	fn kernel_data_read(&self) -> Result<File, chain::Error>;

	fn kernel_data_write(&self, reader: &mut Read) -> Result<bool, chain::Error>;

	/// Provides a reading view into the current txhashset state as well as
	/// the required indexes for a consumer to rewind to a consistant state
	/// at the provided block hash.
	fn txhashset_read(&self, h: Hash) -> Option<TxHashSetRead>;

	/// Header of the txhashset archive currently being served to peers.
	fn txhashset_archive_header(&self) -> Result<core::BlockHeader, chain::Error>;

	/// Whether the node is ready to accept a new txhashset. If this isn't the
	/// case, the archive is provided without being requested and likely an
	/// attack attempt. This should be checked *before* downloading the whole
	/// state data.
	fn txhashset_receive_ready(&self) -> bool;

	/// Update txhashset downloading progress
	fn txhashset_download_update(
		&self,
		start_time: DateTime<Utc>,
		downloaded_size: u64,
		total_size: u64,
	) -> bool;

	/// Writes a reading view on a txhashset state that's been provided to us.
	/// If we're willing to accept that new state, the data stream will be
	/// read as a zip file, unzipped and the resulting state files should be
	/// rewound to the provided indexes.
	fn txhashset_write(
		&self,
		h: Hash,
		txhashset_data: File,
		peer_peer_info: &PeerInfo,
	) -> Result<bool, chain::Error>;

	/// Get the Grin specific tmp dir
	fn get_tmp_dir(&self) -> PathBuf;

	/// Get a tmp file path in above specific tmp dir (create tmp dir if not exist)
	/// Delete file if tmp file already exists
	fn get_tmpfile_pathname(&self, tmpfile_name: String) -> PathBuf;
}

/// Additional methods required by the protocol that don't need to be
/// externally implemented.
pub trait NetAdapter: ChainAdapter {
	/// Find good peers we know with the provided capability and return their
	/// addresses.
	fn find_peer_addrs(&self, capab: Capabilities) -> Vec<PeerAddr>;

	/// A list of peers has been received from one of our peers.
	fn peer_addrs_received(&self, _: Vec<PeerAddr>);

	/// Heard total_difficulty from a connected peer (via ping/pong).
	fn peer_difficulty(&self, _: &PeerAddr, _: Difficulty, _: u64);

	/// Is this peer currently banned?
	fn is_banned(&self, addr: &PeerAddr) -> bool;
}
