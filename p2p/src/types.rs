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

use std::convert::From;
use std::fmt;
use std::fs::File;
use std::io;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV4, SocketAddrV6, ToSocketAddrs};
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::Arc;

use chrono::prelude::*;
use serde::de::{SeqAccess, Visitor};
use serde::{Deserialize, Deserializer};

use crate::chain;
use crate::chain::txhashset::BitmapChunk;
use crate::core::core;
use crate::core::core::hash::Hash;
use crate::core::core::{OutputIdentifier, Segment, SegmentIdentifier, TxKernel};
use crate::core::global;
use crate::core::pow::Difficulty;
use crate::core::ser::{self, ProtocolVersion, Readable, Reader, Writeable, Writer};
use crate::msg::PeerAddrs;
use crate::util::secp::pedersen::RangeProof;
use crate::util::RwLock;

/// Maximum number of block headers a peer should ever send
pub const MAX_BLOCK_HEADERS: u32 = 512;

/// Maximum number of block bodies a peer should ever ask for and send
#[allow(dead_code)]
pub const MAX_BLOCK_BODIES: u32 = 16;

/// Maximum number of peer addresses a peer should ever send
pub const MAX_PEER_ADDRS: u32 = 256;

/// Maximum number of block header hashes to send as part of a locator
pub const MAX_LOCATORS: u32 = 20;

/// How long a banned peer should be banned for
const BAN_WINDOW: i64 = 10800;

/// The max inbound peer count
const PEER_MAX_INBOUND_COUNT: u32 = 128;

/// The max outbound peer count
const PEER_MAX_OUTBOUND_COUNT: u32 = 8;

/// The min preferred outbound peer count
const PEER_MIN_PREFERRED_OUTBOUND_COUNT: u32 = 8;

/// The peer listener buffer count. Allows temporarily accepting more connections
/// than allowed by PEER_MAX_INBOUND_COUNT to encourage network bootstrapping.
const PEER_LISTENER_BUFFER_COUNT: u32 = 8;

#[derive(Debug)]
pub enum Error {
	Serialization(ser::Error),
	Connection(io::Error),
	/// Header type does not match the expected message type
	BadMessage,
	UnexpectedMessage,
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
	PeerNotFound,
	PeerNotBanned,
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

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct PeerAddr(pub SocketAddr);

impl Writeable for PeerAddr {
	fn write<W: Writer>(&self, writer: &mut W) -> Result<(), ser::Error> {
		match self.0 {
			SocketAddr::V4(sav4) => {
				ser_multiwrite!(
					writer,
					[write_u8, 0],
					[write_fixed_bytes, &sav4.ip().octets().to_vec()],
					[write_u16, sav4.port()]
				);
			}
			SocketAddr::V6(sav6) => {
				writer.write_u8(1)?;
				for seg in &sav6.ip().segments() {
					writer.write_u16(*seg)?;
				}
				writer.write_u16(sav6.port())?;
			}
		}
		Ok(())
	}
}

impl Readable for PeerAddr {
	fn read<R: Reader>(reader: &mut R) -> Result<PeerAddr, ser::Error> {
		let v4_or_v6 = reader.read_u8()?;
		if v4_or_v6 == 0 {
			let ip = reader.read_fixed_bytes(4)?;
			let port = reader.read_u16()?;
			Ok(PeerAddr(SocketAddr::V4(SocketAddrV4::new(
				Ipv4Addr::new(ip[0], ip[1], ip[2], ip[3]),
				port,
			))))
		} else {
			let ip = try_iter_map_vec!(0..8, |_| reader.read_u16());
			let ipv6 = Ipv6Addr::new(ip[0], ip[1], ip[2], ip[3], ip[4], ip[5], ip[6], ip[7]);
			let port = reader.read_u16()?;
			if let Some(ipv4) = ipv6.to_ipv4() {
				Ok(PeerAddr(SocketAddr::V4(SocketAddrV4::new(ipv4, port))))
			} else {
				Ok(PeerAddr(SocketAddr::V6(SocketAddrV6::new(
					ipv6, port, 0, 0,
				))))
			}
		}
	}
}

impl<'de> Visitor<'de> for PeerAddrs {
	type Value = PeerAddrs;

	fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
		formatter.write_str("an array of dns names or IP addresses")
	}

	fn visit_seq<M>(self, mut access: M) -> Result<Self::Value, M::Error>
	where
		M: SeqAccess<'de>,
	{
		let mut peers = Vec::with_capacity(access.size_hint().unwrap_or(0));

		while let Some(entry) = access.next_element::<&str>()? {
			match SocketAddr::from_str(entry) {
				// Try to parse IP address first
				Ok(ip) => peers.push(PeerAddr(ip)),
				// If that fails it's probably a DNS record
				Err(_) => {
					let socket_addrs = entry.to_socket_addrs().map_err(|_| {
						serde::de::Error::custom(format!("Unable to resolve DNS: {}", entry))
					})?;
					peers.append(&mut socket_addrs.map(PeerAddr).collect());
				}
			}
		}
		Ok(PeerAddrs { peers })
	}
}

impl<'de> Deserialize<'de> for PeerAddrs {
	fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
	where
		D: Deserializer<'de>,
	{
		deserializer.deserialize_seq(PeerAddrs { peers: vec![] })
	}
}

impl std::hash::Hash for PeerAddr {
	/// If loopback address then we care about ip and port.
	/// If regular address then we only care about the ip and ignore the port.
	fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
		if self.0.ip().is_loopback() {
			self.0.hash(state);
		} else {
			self.0.ip().hash(state);
		}
	}
}

impl PartialEq for PeerAddr {
	/// If loopback address then we care about ip and port.
	/// If regular address then we only care about the ip and ignore the port.
	fn eq(&self, other: &PeerAddr) -> bool {
		if self.0.ip().is_loopback() {
			self.0 == other.0
		} else {
			self.0.ip() == other.0.ip()
		}
	}
}

impl Eq for PeerAddr {}

impl std::fmt::Display for PeerAddr {
	fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
		write!(f, "{}", self.0)
	}
}

impl PeerAddr {
	/// Convenient way of constructing a new peer_addr from an ip_addr
	/// defaults to port 3414 on mainnet and 13414 on testnet.
	pub fn from_ip(addr: IpAddr) -> PeerAddr {
		let port = if global::is_testnet() { 13414 } else { 3414 };
		PeerAddr(SocketAddr::new(addr, port))
	}

	/// If the ip is loopback then our key is "ip:port" (mainly for local usernet testing).
	/// Otherwise we only care about the ip (we disallow multiple peers on the same ip address).
	pub fn as_key(&self) -> String {
		if self.0.ip().is_loopback() {
			format!("{}:{}", self.0.ip(), self.0.port())
		} else {
			format!("{}", self.0.ip())
		}
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
	pub seeds: Option<PeerAddrs>,

	pub peers_allow: Option<PeerAddrs>,

	pub peers_deny: Option<PeerAddrs>,

	/// The list of preferred peers that we will try to connect to
	pub peers_preferred: Option<PeerAddrs>,

	pub ban_window: Option<i64>,

	pub peer_max_inbound_count: Option<u32>,

	pub peer_max_outbound_count: Option<u32>,

	pub peer_min_preferred_outbound_count: Option<u32>,

	pub peer_listener_buffer_count: Option<u32>,

	pub dandelion_peer: Option<PeerAddr>,
}

/// Default address for peer-to-peer connections.
impl Default for P2PConfig {
	fn default() -> P2PConfig {
		let ipaddr = "0.0.0.0".parse().unwrap();
		P2PConfig {
			host: ipaddr,
			port: 3414,
			seeding_type: Seeding::default(),
			seeds: None,
			peers_allow: None,
			peers_deny: None,
			peers_preferred: None,
			ban_window: None,
			peer_max_inbound_count: None,
			peer_max_outbound_count: None,
			peer_min_preferred_outbound_count: None,
			peer_listener_buffer_count: None,
			dandelion_peer: None,
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

	/// return maximum inbound peer connections count
	pub fn peer_max_inbound_count(&self) -> u32 {
		match self.peer_max_inbound_count {
			Some(n) => n,
			None => PEER_MAX_INBOUND_COUNT,
		}
	}

	/// return maximum outbound peer connections count
	pub fn peer_max_outbound_count(&self) -> u32 {
		match self.peer_max_outbound_count {
			Some(n) => n,
			None => PEER_MAX_OUTBOUND_COUNT,
		}
	}

	/// return minimum preferred outbound peer count
	pub fn peer_min_preferred_outbound_count(&self) -> u32 {
		match self.peer_min_preferred_outbound_count {
			Some(n) => n,
			None => PEER_MIN_PREFERRED_OUTBOUND_COUNT,
		}
	}

	/// return peer buffer count for listener
	pub fn peer_listener_buffer_count(&self) -> u32 {
		match self.peer_listener_buffer_count {
			Some(n) => n,
			None => PEER_LISTENER_BUFFER_COUNT,
		}
	}
}

/// Type of seeding the server will use to find other peers on the network.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub enum Seeding {
	/// No seeding, mostly for tests that programmatically connect
	None,
	/// A list of seeds provided to the server (can be addresses or DNS names)
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
		const UNKNOWN = 0b0000_0000;
		/// Can provide full history of headers back to genesis
		/// (for at least one arbitrary fork).
		const HEADER_HIST = 0b0000_0001;
		/// Can provide recent txhashset archive for fast sync.
		const TXHASHSET_HIST = 0b0000_0010;
		/// Can provide a list of healthy peers
		const PEER_LIST = 0b0000_0100;
		/// Can broadcast and request txs by kernel hash.
		const TX_KERNEL_HASH = 0b0000_1000;
		/// Can provide PIBD segments during initial byte download (fast sync).
		const PIBD_HIST = 0b0001_0000;
		/// Can provide historical blocks for archival sync.
		const BLOCK_HIST = 0b0010_0000;
		/// As above, with crucial serialization fix #3705 applied
		const PIBD_HIST_1 = 0b0100_0000;
	}
}

/// Default capabilities.
impl Default for Capabilities {
	fn default() -> Self {
		Capabilities::HEADER_HIST
			| Capabilities::TXHASHSET_HIST
			| Capabilities::PEER_LIST
			| Capabilities::TX_KERNEL_HASH
			| Capabilities::PIBD_HIST
			| Capabilities::PIBD_HIST_1
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

	pub fn is_inbound(&self) -> bool {
		self.direction == Direction::Inbound
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
			capabilities: info.capabilities,
			user_agent: info.user_agent.clone(),
			version: info.version,
			addr: info.addr,
			direction: info.direction,
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
		opts: chain::Options,
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
	/// Converts block to v2 compatibility if necessary (based on peer protocol version).
	fn get_block(&self, h: Hash, peer_info: &PeerInfo) -> Option<core::Block>;

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

	fn get_kernel_segment(
		&self,
		hash: Hash,
		id: SegmentIdentifier,
	) -> Result<Segment<TxKernel>, chain::Error>;

	fn get_bitmap_segment(
		&self,
		hash: Hash,
		id: SegmentIdentifier,
	) -> Result<(Segment<BitmapChunk>, Hash), chain::Error>;

	fn get_output_segment(
		&self,
		hash: Hash,
		id: SegmentIdentifier,
	) -> Result<(Segment<OutputIdentifier>, Hash), chain::Error>;

	fn get_rangeproof_segment(
		&self,
		hash: Hash,
		id: SegmentIdentifier,
	) -> Result<Segment<RangeProof>, chain::Error>;

	fn receive_bitmap_segment(
		&self,
		block_hash: Hash,
		output_root: Hash,
		segment: Segment<BitmapChunk>,
	) -> Result<bool, chain::Error>;

	fn receive_output_segment(
		&self,
		block_hash: Hash,
		bitmap_root: Hash,
		segment: Segment<OutputIdentifier>,
	) -> Result<bool, chain::Error>;

	fn receive_rangeproof_segment(
		&self,
		block_hash: Hash,
		segment: Segment<RangeProof>,
	) -> Result<bool, chain::Error>;

	fn receive_kernel_segment(
		&self,
		block_hash: Hash,
		segment: Segment<TxKernel>,
	) -> Result<bool, chain::Error>;
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
	fn peer_difficulty(&self, _: PeerAddr, _: Difficulty, _: u64);

	/// Is this peer currently banned?
	fn is_banned(&self, addr: PeerAddr) -> bool;
}

#[derive(Clone, Debug)]
pub struct AttachmentMeta {
	pub size: usize,
	pub hash: Hash,
	pub height: u64,
	pub start_time: DateTime<Utc>,
	pub path: PathBuf,
}

#[derive(Clone, Debug)]
pub struct AttachmentUpdate {
	pub read: usize,
	pub left: usize,
	pub meta: Arc<AttachmentMeta>,
}
