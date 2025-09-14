use grin_core::global;
use grin_core::global::set_local_chain_type;
use grin_core::global::ChainTypes;
use grin_core::ser::BinWriter;
use grin_core::ser::ProtocolVersion;
use grin_core::ser::Writeable;
use grin_p2p::msg::{Message, MsgHeader, Type};
use std::convert::TryInto;
use std::io::Cursor;
use std::vec::Vec;

// Make sure chain type is initialized only once for all tests
static INIT: std::sync::Once = std::sync::Once::new();

fn setup() {
	INIT.call_once(|| {
		global::set_local_chain_type(global::ChainTypes::AutomatedTesting);
		// Make sure we're calling this before any tests run
		// This ensures GLOBAL_CHAIN_TYPE is properly set
		let _ = global::get_chain_type();
	});
}

#[test]
fn test_message_too_large() {
	// Ensure chain type is set at the very start of the test
	global::set_local_chain_type(global::ChainTypes::AutomatedTesting);

	let msg_type = Type::Block;
	let max_len = grin_p2p::msg::max_msg_size(msg_type);
	let payload = vec![0u8; (max_len * 4 + 1).try_into().unwrap()]; // Exceeds 4x limit
	let header = MsgHeader::new(msg_type, payload.len() as u64);

	let mut buffer = Vec::new();
	{
		let mut bin_writer = BinWriter::new(&mut buffer, ProtocolVersion::local());
		header.write(&mut bin_writer).unwrap();
	}
	buffer.extend(&payload);
	let mut cursor = Cursor::new(buffer);

	let result = Message::read(&mut cursor, Some(msg_type));
	assert!(result.is_err(), "Expected error for oversized message");
}

#[test]
fn test_message_size_logging() {
	setup();

	let msg_type = Type::Block;
	let max_len = grin_p2p::msg::max_msg_size(msg_type);
	let payload = vec![0u8; (max_len + 1000).try_into().unwrap()]; // Exceeds 1x but within 4x
	let header = MsgHeader::new(msg_type, payload.len() as u64);
	let mut buffer = Vec::new();
	{
		let mut bin_writer = BinWriter::new(&mut buffer, ProtocolVersion::local());
		header.write(&mut bin_writer).unwrap();
	}
	buffer.extend(&payload);
	let mut cursor = Cursor::new(buffer);

	let result = Message::read(&mut cursor, Some(msg_type));
	assert!(result.is_ok(), "Failed to read message: {:?}", result.err());
	// Check logs manually or with a log capture utility if needed
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
	env_logger::init();
	// Set chain type to ensure global state is initialized
	set_local_chain_type(ChainTypes::AutomatedTesting);
	let msg_type = Type::Block;
	let max_len = grin_p2p::msg::max_msg_size(msg_type);
	let payload = vec![0u8; (max_len + 1000).try_into().unwrap()]; // Exceeds 1x but within 4x
	let header = MsgHeader::new(msg_type, payload.len() as u64);

	let mut buffer = Vec::new();
	{
		let mut bin_writer = BinWriter::new(&mut buffer, ProtocolVersion::local());
		header.write(&mut bin_writer).unwrap();
	}
	buffer.extend(&payload);
	let mut cursor = Cursor::new(buffer);

	let result = Message::read(&mut cursor, Some(msg_type));
	assert!(result.is_ok(), "Failed to read message: {:?}", result.err());
	// Check logs manually or with a log capture utility if needed

	let payload = vec![0u8; (max_len * 4 + 1).try_into().unwrap()]; // Exceeds 4x limit
	let header = MsgHeader::new(msg_type, payload.len() as u64);

	let mut buffer = Vec::new();
	{
		let mut bin_writer = BinWriter::new(&mut buffer, ProtocolVersion::local());
		header.write(&mut bin_writer).unwrap();
	}
	buffer.extend(&payload);
	let mut cursor = Cursor::new(buffer);

	let result = Message::read(&mut cursor, Some(msg_type));
	assert!(result.is_err(), "Expected error for oversized message");

	Ok(())
}
