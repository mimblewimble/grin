use crate::core::libtx::slate::{Slate, VersionedSlate};
use crate::core::libtx::slate_versions::v0::SlateV0;
use crate::libwallet::{ErrorKind};
use serde_json as json;

pub fn serialize_slate(slate: &Slate) -> String {
	match slate.version {
		0 => {
			let slate = slate.clone();
			json::to_string(&SlateV0::from(slate)).unwrap()
		},
		_ => json::to_string(slate).unwrap(),
	}
}

pub fn deserialize_slate(raw_slate: &str) -> Slate {
	let versioned_slate: VersionedSlate = json::from_str(&raw_slate)
		.map_err(|err| ErrorKind::Format(err.to_string()))
		.unwrap();
	versioned_slate.into()
}
