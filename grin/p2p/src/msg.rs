// ...existing code...
use log::{info, warn};
// ...existing code...

impl Message {
	pub fn read<R: Read>(
		reader: &mut R,
		msg_type: Option<MessageTypeEnum>,
	) -> Result<Message, Error> {
		// ...existing code...
		let header = MessageHeader::read(reader)?;
		let msg_len = header.msg_len as usize;

		match msg_type {
			Some(msg_type) => {
				let max_len = max_msg_size(msg_type);
				let current_max_len = max_len * 4; // Current 4x limit
				if msg_len > current_max_len {
					return Err(Error::MsgTooLarge(msg_len, current_max_len));
				}
				info!(
					"Received {:?} message: size={} bytes, 1x limit={} bytes, 4x limit={} bytes",
					msg_type, msg_len, max_len, current_max_len
				);
				if msg_len > max_len {
					warn!(
						"Message size ({} bytes) exceeds 1x limit ({} bytes) for type {:?}",
						msg_len, max_len, msg_type
					);
				}
			}
			None => {
				info!("Received unknown message type: size={} bytes", msg_len);
			}
		}

		let mut payload = vec![0u8; msg_len];
		reader.read_exact(&mut payload)?;
		Ok(Message { header, payload })
	}
	// ...existing code...
}
// ...existing code...
