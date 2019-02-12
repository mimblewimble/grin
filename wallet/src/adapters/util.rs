use crate::libwallet::slate::{Slate, VersionedSlate};
use crate::libwallet::slate_versions::v0::SlateV0;
use crate::libwallet::ErrorKind;
use serde_json as json;

pub fn get_versioned_slate(slate: &Slate) -> VersionedSlate {
	let slate = slate.clone();
	match slate.version {
		0 => VersionedSlate::V0(SlateV0::from(slate)),
		_ => VersionedSlate::V1(slate),
	}
}

pub fn serialize_slate(slate: &Slate) -> String {
	json::to_string(&get_versioned_slate(slate)).unwrap()
}

pub fn deserialize_slate(raw_slate: &str) -> Slate {
	let versioned_slate: VersionedSlate = json::from_str(&raw_slate)
		.map_err(|err| ErrorKind::Format(err.to_string()))
		.unwrap();
	versioned_slate.into()
}
