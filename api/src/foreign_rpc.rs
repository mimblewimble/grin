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

//! JSON-RPC Stub generation for the Foreign API

use crate::core::core::hash::Hash;
use crate::core::core::transaction::Transaction;
use crate::foreign::Foreign;
use crate::pool::PoolEntry;
use crate::pool::{BlockChain, PoolAdapter};
use crate::rest::Error;
use crate::types::{
	BlockHeaderPrintable, BlockPrintable, LocatedTxKernel, OutputListing, OutputPrintable, Tip,
	Version,
};
use crate::util;

/// Public definition used to generate Node jsonrpc api.
/// * When running `grin` with defaults, the V2 api is available at
/// `localhost:3413/v2/foreign`
/// * The endpoint only supports POST operations, with the json-rpc request as the body
#[easy_jsonrpc_mw::rpc]
pub trait ForeignRpc: Sync + Send {
	/**
	Networked version of [Foreign::get_header](struct.Foreign.html#method.get_header).

	# Json rpc example

	```
	# grin_api::doctest_helper_json_rpc_foreign_assert_response!(
	# r#"
	{
		"jsonrpc": "2.0",
		"method": "get_header",
		"params": [null, "00000100c54dcb7a9cbb03aaf55da511aca2c98b801ffd45046b3991e4f697f9", null],
		"id": 1
	}
	# "#
	# ,
	# r#"
	{
		"id": 1,
		"jsonrpc": "2.0",
		"result": {
			"Ok": {
			"cuckoo_solution": [
				9886309,
				35936712,
				43170402,
				48069549,
				70022151,
				97464262,
				107044653,
				108342481,
				118947913,
				130828808,
				144192311,
				149269998,
				179888206,
				180736988,
				207416734,
				227431174,
				238941623,
				245603454,
				261819503,
				280895459,
				284655965,
				293675096,
				297070583,
				299129598,
				302141405,
				313482158,
				321703003,
				351704938,
				376529742,
				381955038,
				383597880,
				408364901,
				423241240,
				436882285,
				442043438,
				446377997,
				470779425,
				473427731,
				477149621,
				483204863,
				496335498,
				534567776
			],
			"edge_bits": 29,
			"hash": "00000100c54dcb7a9cbb03aaf55da511aca2c98b801ffd45046b3991e4f697f9",
			"height": 374336,
			"kernel_mmr_size": 2210914,
			"kernel_root": "d294e6017b9905b288dc62f6f725c864665391c41da20a18a371e3492c448b88",
			"nonce": 4715085839955132421,
			"output_mmr_size": 4092001,
			"output_root": "12464313f7cd758a7761f65b2837e9b9af62ad4060c97180555bfc7e7e5808fa",
			"prev_root": "e22090fefaece85df1441e62179af097458e2bdcf600f8629b977470db1b6db1",
			"previous": "0000015957d92c9e04c6f3aec8c5b9976f3d25f52ff459c630a01a643af4a88c",
			"range_proof_root": "4fd9a9189e0965aa9cdeb9cf7873ecd9e6586eac1dd9ca3915bc50824a253b02",
			"secondary_scaling": 561,
			"timestamp": "2019-10-03T16:08:11+00:00",
			"total_difficulty": 1133587428693359,
			"total_kernel_offset": "0320b6f8a4a4180ed79ecd67c8059c1d7bd74afe144d225395857386e5822314",
			"version": 2
			}
		}
	}
	# "#
	# );
	```
	*/
	fn get_header(
		&self,
		height: Option<u64>,
		hash: Option<String>,
		commit: Option<String>,
	) -> Result<BlockHeaderPrintable, Error>;

	/**
	Networked version of [Foreign::get_block](struct.Foreign.html#method.get_block).

	# Json rpc example

	```
	# grin_api::doctest_helper_json_rpc_foreign_assert_response!(
	# r#"
	{
		"jsonrpc": "2.0",
		"method": "get_block",
		"params": [374274, null, null],
		"id": 1
	}
	# "#
	# ,
	# r#"
	{
		"id": 1,
		"jsonrpc": "2.0",
		"result": {
			"Ok": {
			"header": {
				"cuckoo_solution": [
				1263501,
				14648727,
				42430559,
				58137254,
				68666726,
				72784903,
				101936839,
				104273571,
				123886748,
				131179768,
				155443226,
				162493783,
				164784425,
				167313215,
				169806918,
				183041591,
				184403611,
				210351649,
				215159650,
				239995384,
				240935454,
				257742462,
				280820644,
				300143903,
				303146496,
				311804841,
				341039986,
				354918290,
				363508555,
				377618528,
				396693709,
				397417856,
				399875872,
				413238540,
				413767813,
				432697194,
				436903767,
				447257325,
				453337210,
				459401597,
				496068509,
				511300624
				],
				"edge_bits": 29,
				"hash": "000001e16cb374e38c979c353a0aaffbf5b939da7688f69ad99efda6c112ea9b",
				"height": 374274,
				"kernel_root": "e17920c0e456a6feebf19e24a46f510a85f21cb60e81012f843c00fe2c4cad6e",
				"nonce": 4354431877761457166,
				"output_root": "1e9daee31b80c6b83573eacfd3048a4af57c614bd36f9acd5fb50fbd236beb16",
				"prev_root": "9827b8ffab942e264b6ac81f2b487e3de65e411145c514092ce783df9344fa8a",
				"previous": "00001266a73ba6a8032ef8b4d4f5508407ffb1c270c105dac06f4669c17af020",
				"range_proof_root": "3491b8c46a3919df637a636ca72824377f89c4967dcfe4857379a4a82b510069",
				"secondary_scaling": 571,
				"timestamp": "2019-10-03T15:15:35+00:00",
				"total_difficulty": 1133438031814173,
				"total_kernel_offset": "63315ca0be65c9f6ddf2d3306876caf9f458a01d1a0bf50cc4d3c9b699161958",
				"version": 2
			},
			"inputs": [],
			"kernels": [
				{
				"excess": "08761e9cb1eea5bfcf771d1218b5ec802798d6eecaf75faae50ba3a1997aaef009",
				"excess_sig": "971317046c533d21dff3e449cc9380c2be10b0274f70e009aa2453f755239e3299883c09a1785b15a141d89d563cdd59395886c7d63aba9c2b6438575555e2c4",
				"features": "Coinbase",
				"fee": 0,
				"lock_height": 0
				}
			],
			"outputs": [
				{
				"block_height": 374274,
				"commit": "09d33615563ba2d65acc2b295a024337166b9f520122d49730c73e8bfb43017610",
				"merkle_proof": null,
				"mmr_index": 4091742,
				"output_type": "Coinbase",
				"proof": "7adae7bcecf735c70eaa21e8fdce1d3c83d7b593f082fc29e16ff2c64ee5aaa15b682e5583257cf351de457dda8f877f4d8c1492af3aaf25cf5f496fce7ca54a0ef78cc61c4252c490386f3c69132960e9edc811add6415a6026d53d604414a5f4dd330a63fcbb005ba908a45b2fb1950a9529f793405832e57c89a36d3920715bc2d43db16a718ecd19aeb23428b5d3eeb89d73c28272a7f2b39b8923e777d8eb2c5ce9872353ba026dc79fdb093a6538868b4d184215afc29a9f90548f9c32aa663f9197fea1cadbb28d40d35ed79947b4b2b722e30e877a15aa2ecf95896faad173af2e2795b36ce342dfdacf13a2f4f273ab9927371f52913367d1d58246a0c35c8f0d2330fcddb9eec34c277b1cfdaf7639eec2095930b2adef17e0eb94f32e071bf1c607d2ef1757d66647477335188e5afc058c07fe0440a67804fbdd5d35d850391ead3e9c8a3136ae1c42a33d5b01fb2c6ec84a465df3f74358cbc28542036ae4ef3e63046fbd2bce6b12f829ed193fb51ea87790e88f1ea686d943c46714b076fb8c6be7c577bca5b2792e63d5f7b8f6018730b6f9ddaf5758a5fa6a3859d68b317ad4383719211e78f2ca832fd34c6a222a8488e40519179209ad1979f3095b7b7ba7f57e81c371989a4ace465149b0fe576d89473bc596c54cee663fbf78196e7eb31e4d56604c5226e9242a68bda95e1b45473c52f63fe865901839e82079a9935e25fe8d44e339484ba0a62d20857c6b3f15ab5c56b59c7523b63f86fa8977e3f4c35dc8b1c446c48a28947f9d9bd9992763404bcba95f94b45d643f07bb7c352bfad30809c741938b103a44218696206ca1e18f0b10b222d8685cc1ed89d5fdb0c7258b66486e35c0fd560a678864fd64c642b2b689a0c46d1be6b402265b7808cd61a95c2b4a4df280e3f0ec090197fb039d32538d05d3f0a082f5",
				"proof_hash": "cfd97db403c274220bb0dbaf3ecc88e483c0b707d8e6f16dfda37cd4f2c3211c",
				"spent": false
				}
			]
			}
		}
	}
	# "#
	# );
	```
	 */
	fn get_block(
		&self,
		height: Option<u64>,
		hash: Option<String>,
		commit: Option<String>,
	) -> Result<BlockPrintable, Error>;

	/**
	Networked version of [Foreign::get_version](struct.Foreign.html#method.get_version).

	# Json rpc example

	```
	# grin_api::doctest_helper_json_rpc_foreign_assert_response!(
	# r#"
	{
		"jsonrpc": "2.0",
		"method": "get_version",
		"params": [],
		"id": 1
	}
	# "#
	# ,
	# r#"
	{
		"id": 1,
		"jsonrpc": "2.0",
		"result": {
			"Ok": {
			"node_version": "2.1.0-beta.2",
			"block_header_version": 2
			}
		}
	}
	# "#
	# );
	```
	 */
	fn get_version(&self) -> Result<Version, Error>;

	/**
	Networked version of [Foreign::get_tip](struct.Foreign.html#method.get_tip).

	# Json rpc example

	```
	# grin_api::doctest_helper_json_rpc_foreign_assert_response!(
	# r#"
	{
		"jsonrpc": "2.0",
		"method": "get_tip",
		"params": [],
		"id": 1
	}
	# "#
	# ,
	# r#"
	{
		"id": 1,
		"jsonrpc": "2.0",
		"result": {
			"Ok": {
			"height": 374350,
			"last_block_pushed": "000000543c69a0306b5463b92939643442a44a6d9be5bef72bea9fc1d718d310",
			"prev_block_to_last": "000001237c6bac162f1add2b122fab6a254b9fcc2c4b4c8c632a8c39855521f1",
			"total_difficulty": 1133621604919005
			}
		}
	}
	# "#
	# );
	```
	 */
	fn get_tip(&self) -> Result<Tip, Error>;

	/**
	Networked version of [Foreign::get_kernel](struct.Foreign.html#method.get_kernel).

	# Json rpc example

	```
	# grin_api::doctest_helper_json_rpc_foreign_assert_response!(
	# r#"
	{
		"jsonrpc": "2.0",
		"method": "get_kernel",
		"params": ["09c868a2fed619580f296e91d2819b6b3ae61ab734bf3d9c3eafa6d9700f00361b", null, null],
		"id": 1
	}
	# "#
	# ,
	# r#"
	{
		"id": 1,
		"jsonrpc": "2.0",
		"result": {
			"Ok": {
			"height": 374557,
			"mmr_index": 2211662,
			"tx_kernel": {
				"excess": "09c868a2fed619580f296e91d2819b6b3ae61ab734bf3d9c3eafa6d9700f00361b",
				"excess_sig": "1720ec1b94aa5d6ba4d567f7446314f9a6d064eea69c5675cc5659f65f290d80b0e9e3a48d818cadba0a4e894bbc6eb6754b56f53813e2ee0b1447969894ca4a",
				"features": "Coinbase"
			}
			}
		}
	}
	# "#
	# );
	```
	 */
	fn get_kernel(
		&self,
		excess: String,
		min_height: Option<u64>,
		max_height: Option<u64>,
	) -> Result<LocatedTxKernel, Error>;

	/**
	Networked version of [Foreign::get_outputs](struct.Foreign.html#method.get_outputs).

	# Json rpc example

	```
	# grin_api::doctest_helper_json_rpc_foreign_assert_response!(
	# r#"
	{
		"jsonrpc": "2.0",
		"method": "get_outputs",
		"params": [
			[
				"09bab2bdba2e6aed690b5eda11accc13c06723ca5965bb460c5f2383655989af3f",
				"08ecd94ae293863286e99d37f4685f07369bc084ba74d5c59c7f15359a75c84c03"
			],
			376150,
			376154,
			true,
			true
		],
		"id": 1
	}
	# "#
	# ,
	# r#"
	{
		"id": 1,
		"jsonrpc": "2.0",
		"result": {
			"Ok": [
			{
				"block_height": 374568,
				"commit": "09bab2bdba2e6aed690b5eda11accc13c06723ca5965bb460c5f2383655989af3f",
				"merkle_proof": null,
				"mmr_index": 4093403,
				"output_type": "Transaction",
				"proof": "e30aa961d6f89361a9a3c60f73e3551f50a3887212e524b5874ac50c1759bb95bc8e588d82dd51d84c7cbaa9abe79e0b8fe902bcfda17276c24d269fbf636aa2016c65a760a02e18338a33e83dec8e51fbfd953ee5b765d97ce39ba0850790d2104812a1d15d5eaa174de548144d3a7d413906d85e22f89065ef727910ee4c573494520c43e36e83dacee8096666aa4033b5e8322e72930c3f8476bb7be9aef0838a2ad6c28f4f5212708bf3e5954fc3971d66b7835383b96406fa65415b64ecd53a747f41d785c3e3615c18dfdbe39a0920fefcf6bc55fe65b4b215b1ad98c80fdafbef6f21ab60596f2d9a3e7bc45d750e807d5eb883dadde1625d4f20af9f1315b8bea08c97fad922afe2000c84c9eb5f96b2a24da7a637f95c1102ecfc1257e19bc4120082f5ee76448c90abd55108256f8341e0f4009cfc3906a598de465467ee1ee072bfd3384e1a0b9039192d1edc33092d7b09d1164c4fc4c378227a391600a8a5d5ba5fe36a2a4eabe0dbae270aefa5a5f2df810cda79211805206ad93ae08689e2675aad025db3499d43f1effc110dfb2f540ccd6eb972c02f98e8151535c099381c8aeb1ea8aad2cfdf952e6ab9d26e74a5611d943d02315e212eb06ce2cd20b4675e6f245e5302cdb8b31d46bb2e718b50ecfad2d440323826570447c2498376c8bad6e4ee97bde41c47f6a20eea406d758c53fb9e8542f114c1a277a6335ad97fdc542c6bbec756dc4a9085c319fe6f0c9e1bb043f01a43c12aa6f4dff8b1220e7f16bc56dee9ccb59fb7c3b7aa6bb33b41c33d8e4b03b6b9cb89491504210dd691b46ffe2862387339d2b62a9dc4c20d629e23eb8b06490c4999433c1b4626fb4d21517072bd8e82511c115ee47bf9a5e40f0a74177f5b573db2e277459877a01b172e026cbb3f76aaf0c61f244584f3a76804dea62175a80d777238",
				"proof_hash": "660d706330fc36f611c50d90cb965fddf750cc91f8891a58b5e39b83a5fc6b46",
				"spent": false
			},
			{
				"block_height": 376151,
				"commit": "08ecd94ae293863286e99d37f4685f07369bc084ba74d5c59c7f15359a75c84c03",
				"merkle_proof": "6b2abbd334c9d75409461fba9c1acd4a8d7bc2ab0bc43143f42388b2a3a87b881505ccf8ffc8737fa6fd4fe412a082d974911bd223eae612d0d1d7ddcc09b5e6079c40b011405b2ccb49ce32473c93aea6d843488d5765fea114d3368d34cd05fcb8c2de3903fbaf39b1f064c809f9f1c0d47959d81a508957040eda55c6dce6dd8c43a79c72faffacfabe1d73055790b6249de2f7c603f186cb109eee58fb1426ea48cb781f88df9acd8996d235fe6bfe60e02aae6e3bfe38ed2599baca1430b3b637072d9bdcdc7644f873728e3cd38eff7124ea848cfad67f8e114cf8595c89a3686a4271cfb2b5098597c315c01d04270ca8f70262af967a947f49adacfa4aad8b6fd196dd0ef4e5cefa132c38c7e5f43db12b3d74f0a8d83c3404e73c6b25a12bff70a8ef4526c89b1558810bb744ede53f8c4cc8cc2555e953637722adb41ea5752281cf1f75599f7e59b17f11f5f9ce4f6b2da4141a3398f51d8b834cdc8b00f61915a41d200572a10bb2102cbae7e94aa7ced3c388dcd58282932c99a8fa66f6fc511ff3e8c60d442bbdb49cca1166328ca8c9bbc97d024570b4cc1ca6c7dba3db223e9e27fd9345b94d3cf10e2b54915b87c57e32965bc2db1b1f956d1962812738ca9b2c93fd7825adf4dffddc97aa85ca0f3f412f02d30678a816d2efbfb6778305fd5e610b6e8af30030bc059880c337bfde326b392d5dcd7c36cb0076fbccc7099b94f1f03bdb525d6e3818b6d50b93ced802957a4b03892c71b6679052bd35e92ceea71a96b22b2ed2c129755f0c74fa172f43da2790f3132a7e57e408d2fc5f1126b088cd0398e6dedcb237242e6720e12e8d7a5a1e196eda6241cfee1cc85e9d20af67f3f9bdf91160516ebcd0b8da6bb7b12229e1112b22c9f1aaef1d75441465cfee2ac1c47b5255514316ed4637e192b00ff28491168f2f2b00",
				"mmr_index": 4107711,
				"output_type": "Coinbase",
				"proof": "7083884b5f64e4e61fb910d2c3c603f7c94490716d95e7144b4c927d0ca6ccc0e069cc285e25f38ee90c402ef26005cad2b4073eeba17f0ae3ea2b87095106ef00634f321d8a49c2feaad485bc9ee552564a6a883c99886d0d3a85af3490d718f5a5cbc70f9dcc9bf5d987fb6072132a4c247d4bbd4af927532a887b1e4250b7277771f6b82f43f4fb5a48089ed58e7d3190a19197e07acfed650f8b2cd5f103e994fb3d3735c5727f06f302bd1f182586297dd57a7951ff296bdf6106704abedc39db77f1293effaa7496a77d19420a6208bc1c589b33dad9540cb6180cccf5e085006b01309419f931e54531d770e5fe00eca584072692a7e4883fd65ed4a7c460665608ab96bf0c7d564fe96a341f14066db413a6fddc359eb11f6f962aca70ca1414c35d7941ce06b77d0a0606081b78d5e64a4501f8e8eba9f0e0889042bc54b4cbfd71087a95af63e0306dba214084d4860b0ce66dc80af44224e5a6fef55800650b05cf1639f81bfdc30950f3634d1fd4375d50c22c7f13f3dfb690e5f155a535aff041b7f800bfe74c60f606e8ab47df60754a0e08221c2a50abe643bb086433afd040a7e6290d1d00b3fe657be3bb05c67f90eb183c2acb53c81e1ca15cd8d35fe9d7d52d8f455398e905bdc77ffb211697d477af25704cf9896e8ce797f4fed03e2ba1615e3ad5646eecaa698470f99437d01d5193f041201502763e8bde51e6dc830b5c676d05c8f7f87c4972c578b8d9d5922ba29f6e4a89a123311d02b5ac44a7d5307f7ed5e4e66aaf749afc76c6fc1114445d6fafeea816a0f985eeacdbe9e6d32a8514ca4aaf7faad4e9d43cde55327ac84bac4d70a9319840e136e713aa31d639e43302f3c71a79f08f4e5c9a19a48d4b46403734cd8f3cc9b67bc26ea8e2a01e63a6f5be6e044e8ed5db5f26d15d25de75f672a79315c5e2407e",
				"proof_hash": "7cf77fdaecef6c6fc01edca744c1521581f854a9bac0153971edbb1618fc36ad",
				"spent": false
			},
			{
				"block_height": 376154,
				"commit": "095c12db5e57e4a1ead0870219bda4ebfb1419f6ab1501386b9dd8dc9811a8c5ff",
				"merkle_proof": "00000000003eadc6000000000000000e13c509a17cbb0d81634215cd2482ab6d9eb58b332fcbe6b2c4fa458a63d3cb0dfe3614ebe6e52657870df225d132179fa1ea0fdc2105f0e51d03bc3765a9cd059c60d434a7cae0a3d669b37588c25410f57405c841312cfa50cf514678877a3f4ce8bd3e57723ba75a2b7d61027b2088fbabebdb7336b97ea88b00a7e809a6245def980eba18d987601f4cbd6c3cc9f12a5684fe7a1bc2565a9f8ab63c2db1afa8304f5e23d4754cd97f29c8b06dcb3de4f6d3a83079676b6e9941afe5553a7195384b564ecd6d37522cb5e452cc930d2b549af22698a8fd9bf6cad05a06b09e3f6e672b94e82c0255394b5c187ab76fda653a2491378997ba3d49f9d9c34ca93bc627fe5d98b327c03d429b5473f62672e9d73c4eafd9cb8f62e5158a1ec7eb56653696b10fb9ec205f5e4d1c7a1f3e2dd2994b12eeed93e84776d8dcd8a5d78aecd4f96ae95c0b090d104adf2aa84f0a1fbd8d319fea5476d1a306b2800716e60b00115a5cca678617361c5a89660b4536c56254bc8dd7035d96f05de62b042d16acaeff57c111fdf243b859984063e3fcfdf40c4c4a52889706857a7c3e90e264f30f40cc87bd20e74689f14284bc5ea0a540950dfcc8d33c503477eb1c60",
				"mmr_index": 4107717,
				"output_type": "Coinbase",
				"proof": "073593bc475478f1e4b648ab261df3b0a6e5a58a617176dd0c8f5e0e1d58b012b40eb9b341d16ee22baf3645ea37705895e731dee5c220b58b0f780d781806a10dfa33e870d0494fba18aaa8a7a709bfb3ddf9eb3e4e75a525b382df68dc6f710275cdffb623373c47c1310ae63479826f435ca4520fdc13bb0d995b7d9a10a7587d61bd4a51c9e32c87f3eb6b0f862cdff19a9ac6cb04d6f7fafb8e94508a851dcf5dc6acea4271bb40117a45319da5522b966091b089698f4f940842458b5b49e212d846be35e0c2b98a00ac3d0b7ceaf081272dbed8abd84fe8f26d57bac1340e8184602436ed8c4470ef9dc214df3405de0e71703abec4456b15e122a94706852bb476213ceadf00529d00d8d3b16dc57f4e4a9a86dacfa719e00366728de42f3f830e73f6113f1e391fab07eba1b40f6466203b0ce14701230e934f6138c575660a03dbb0e59d7295df3115a4fc0909a5520d74657b319fc83481079ad6c13400175e39fa2b86071ba563ce8836320713ef8f55d4e90bee3f57df96c7aef0f2e896f57192fae9675471cd9751bcaf2b15e5a65a9733a6f7f9b8147b8f6e8dac51d056018d411fd252225cf88e56f143143f49e8a0d2e43c10de0442dbc84966817532b1256b6769db987526790a389c371a1fe7a36eacffef82877b4db7a9b5e58722ffbd0fc4fdbd7624365ee326bb8b1e60b999f513715b30f37ef6116eabf53b3524b46c33a1fac49205b39e24aa388d823269c1fc43c3599a06b69433a0a47a03bd871321afb7846a6dbfd5891bd84f89c556231745c929d08445f66f332857bfda1c4f86ae58a01007b7303f870ac24e0ba72d84c0ef4903ac2ff777e2c2dcb4d8e303c74e0c8a559686b4d4c25024ee97601787d4e5a97224af41e5d35d91744292f5a41f64d4e1cae77bebebd77a473f3b54e86f7221aac230942f0468",
				"proof_hash": "5dd69c083e2c0fd797a499bbafedee0728849afa3476034280ecadf6eb4bffc2",
				"spent": false
			},
			{
				"block_height": 376153,
				"commit": "0948cb346b7affe004a6f84fa4b5b44995830f1c332b03537df4c258d51d1afb50",
				"merkle_proof": "00000000003eadc4000000000000000dfe3614ebe6e52657870df225d132179fa1ea0fdc2105f0e51d03bc3765a9cd059c60d434a7cae0a3d669b37588c25410f57405c841312cfa50cf514678877a3f4ce8bd3e57723ba75a2b7d61027b2088fbabebdb7336b97ea88b00a7e809a6245def980eba18d987601f4cbd6c3cc9f12a5684fe7a1bc2565a9f8ab63c2db1afa8304f5e23d4754cd97f29c8b06dcb3de4f6d3a83079676b6e9941afe5553a7195384b564ecd6d37522cb5e452cc930d2b549af22698a8fd9bf6cad05a06b09e3f6e672b94e82c0255394b5c187ab76fda653a2491378997ba3d49f9d9c34ca93bc627fe5d98b327c03d429b5473f62672e9d73c4eafd9cb8f62e5158a1ec7eb56653696b10fb9ec205f5e4d1c7a1f3e2dd2994b12eeed93e84776d8dcd8a5d78aecd4f96ae95c0b090d104adf2aa84f0a1fbd8d319fea5476d1a306b2800716e60b00115a5cca678617361c5a89660b4536c56254bc8dd7035d96f05de62b042d16acaeff57c111fdf243b859984063e3fcfdf40c4c4a52889706857a7c3e90e264f30f40cc87bd20e74689f14284bc5ea0a540950dfcc8d33c503477eb1c60",
				"mmr_index": 4107716,
				"output_type": "Coinbase",
				"proof": "72950da23ad7f0d0381e2f788bf0ac6b6bcb17aaccf0373534122a95714d2d0dbf6a24822b4aab0711a595c80bc36122957111c39292f2a36a973252fb88cbda0b1d61ea8ea84f5171a61f751cac97332637b7cf74cc73144b912ba700dedaa60895f06e947f1e42a8c79d70f924f45fdcb6df5d30289f36ff77d0ae368df5775a739b7a25cbfb63f0cdbdc167b046067c2a021fe0950c7b67515b185b9e4a00ce63b795d49ae184fe5cc726d72fc05d717c4fb55dd5f65967dc282d3c47cb6f8a92cb696e5a1d8cca21214bc766e3de6271791cebf646cda97ae77035da16606f3397f71e103137358c97b9943c3e15403184f61230bd0e3954c7681a0891aa7a0cc32e82d830fb7d8759a04d1da7058630a853508df095142f22158c28bd5e3f2477ad6c8990e63d0377a0fa3d588b6584453778eb38cbaec8a33c1d3772c97a826d4a2f6953c35342993b04567e9fea6fc64fb714653f934faa1a8f635d39eb2903de4bed960a3df07dce7c2e3ff517bbc15f467d0190a579bc07b0f1a910b23269d794835bbb34e8318dcc4fd4159f8f03faa77842d445cf61af9e33caf46aa5fae0812a6476a09c0757e929271a96a245701ab14c1fdd836b92b7e763afa623017f68f1bc4eb716ce735820a1311b743dd8d5c6bb275a2e4e7d2eff8f45417b60cc937086c3e7fd3b612ae064d7237eb6a7bd1a39d8575fac312068fa060bc1ceac4df0754601edaf04ecb1b89c0661ea01a593c3763e456bebbd8487edc0ff3bc6f203965cd92b1706070c59a3795f9dee23087cea0aaec015f1b7bfe4df81818d7a37af781ca7b757ace2fa489f85215ecb85976b1c74c7f1df6d834a8bc63e887407ef6e233c55ea040bc5f2471e99ebc92f2283ff592ff751d9226bd105e68e187c91ecb236c9fa4fb060ae4d706c571ac2123da1debd12737d98be118578",
				"proof_hash": "0ce421970d13fe9b3981e308c5d0b549982cdda9f69918289cd95ffcd09e0fc2",
				"spent": false
			}
			]
		}
	}
	# "#
	# );
	```
	 */
	fn get_outputs(
		&self,
		commits: Option<Vec<String>>,
		start_height: Option<u64>,
		end_height: Option<u64>,
		include_proof: Option<bool>,
		include_merkle_proof: Option<bool>,
	) -> Result<Vec<OutputPrintable>, Error>;

	/**
	Networked version of [Foreign::get_unspent_outputs](struct.Foreign.html#method.get_unspent_outputs).

	# Json rpc example

	```
	# grin_api::doctest_helper_json_rpc_foreign_assert_response!(
	# r#"
	{
		"jsonrpc": "2.0",
		"method": "get_unspent_outputs",
		"params": [1, null, 2, true],
		"id": 1
	}
	# "#
	# ,
	# r#"
	{
		"id": 1,
		"jsonrpc": "2.0",
		"result": {
			"Ok": {
			"highest_index": 2078061,
			"last_retrieved_index": 30,
			"outputs": [
				{
				"block_height": 1,
				"commit": "08b7e57c448db5ef25aa119dde2312c64d7ff1b890c416c6dda5ec73cbfed2edea",
				"merkle_proof": null,
				"mmr_index": 1,
				"output_type": "Coinbase",
				"proof": "9330ad8cde205f317c6537eca96b866293a0489615a9a277b4d3a597c873544c82474932b641e06ac8719604ee52e895e8cd4621b6bfb85780cd9becce14d0700b83a664db2f52a26c425fd777ad88944cdfff38043a2793ed4d9aa67e36cbfd5585579fc69dda930418af5eaf603654f6f751258d2dfc8c2113c171e130f31ec1e6cce2a718e435298fce5d64ffe1bd3464fd7c87cfa92093855be034bfe4439e928bd92ad77fd0a0e00355ee1d1a9ceb1ed0c408dcfdba8c583e7598dc700aaa9f91432097259a405f5b7315a2f7658861e3349bb0dc8bf883726a215f0149ded6613e5ac0670c0c5202247d7c27c8a7d03bdb03c9cf5455463f9b42cf87403e31f8383cc4f49a34c62ae459f5801a9eed4f0ee3dfd5f55b7011c0cae393c474abd6f8c7965b9b5fff3104dd4e39542077c0c8dd2f8ffceb6bb598512d90506d0a7184f20f1498cf458787f23284b54888c9be416d103f760406357a16b6d841a303d5c95b6b474d2d7f0fea0a2a76c897dd2110e9303f54684169421147684c6f1819c33cef3f38ec995a508450c02cd1872f8065fdee723109c18b1dd2ddde75825546ecf0df0793c353b20c946cd64122cea8c116f432336899a16ad24a2aafcb8f900e09a1147135fcf2a54cbf81db308a47a08a49c77c130e5dc5e661cd55a5cc69e607055a5b08111bf61a62ea5778f85119043633f1cab8c756d756c5a34851024ac311a596b1cd919bbca43226f0ba057f6b57de2f6955b0823c3826de7f6096c1c1b6b9b8e4063e1645c0bff32f80561aaa959d97120fbc2ecd9d2be28bd0c17811dc59a88049f6d8952ee9a0a0207693c89ca3ad1197e9bfdfc03be9d845aea8d663969217e3b494cee9e652bc9f8713e2fd5cb1843848f46c3a6ab024d0e3d57ca45454cdbda414adaa835fa147deb4ffb7129cf3a8d86726a0144794",
				"proof_hash": "6c301688d9186c3a99444f827bdfe3b858fe87fc314737a4dc1155d9884491d2",
				"spent": false
				},
				{
				"block_height": 29,
				"commit": "09bab1ddad0f6fec1aedcd3830c5c647515ad543929e722344e4a8d390b6fdd51b",
				"merkle_proof": null,
				"mmr_index": 55,
				"output_type": "Coinbase",
				"proof": "4a5f858d4311bdd902f4446682f27f64be376283b1171060fd2ad33d85350fee13c25a030874d6308d2b325995a3fe545eb1d85ba66e2ba002b794edfdeacb3f0fd2a690b9a78137771b3633aaef2a77f62fbe4d6b4b373c4bdb7e5f58cfae361a3b4c2e4420cc0d38465b2444e01b50e57c6ebfc2afd6dda9017e54585638bddef17d181d1fd7064d975d8bb1dcfd96c89486aed4680b4d39294a141581d1f51c1acfbb80e2ffc40f8499cdc43be04cacda1e34dd6592edfc500229aa70db1c2869f974cfe9aee0cab696c198624de8ecdaf5ae481a1e46fe79fe983209459b89492f2b24416c368394c43c60c33d0fdd1792f0a58d11763e7c8b89d27da25109db346e4d7b62935d182b45dfb659829c55922350e6f7e3452d9311e527ec5b561f4d043cef865f683fce1ce2d410d414f5bcee63c4bbc00964b0fa757bdfd68158e22c1068d871a45759fbd527883c0451db6f36b15139864b6177a78ad64d326e0152914e5313a97ed7b685e5089f2758bf072c804560306bd944831f067c3413ded09330fd788f353e4ee875d3c9303dd4ec0dda9d55b4a27d7748b3247fe85cf3d26b7004e6e3379041fad136fccdacd02b06456a50ad40a3259842c0794f2d59dbd8fa6b4af065b38c388d76b82136b633b06779e4eb05b5b62ec37cdc2986327639bafa8651318f4c00c066e6f45504ec9a96874d5510b519f434a1a88175d51f86e8ee36ae18d107cfaf83e60b2e62fff032c7539be66d776e3a52c5f9b0ee6fe08820d65cd75d35c793e5ab3914adf5a97b7dba75e90d4a4c9aa844e2f1e9464cd5fc4923b475defca4e3b03e1b33353ff91ac1084712cf4445e329ffdbe1e2da16ae71dee0e914b546fdc0db9b0fcde80822ee716e9f2eec90db7aa4417d53a1266e1e8383e20c9a9548bae35c2a8e1293a49e7afbd8011a9e66e79ed6be",
				"proof_hash": "a64ed774d824dc55123c6c5ba46d84bac15b6ead8cb60200836c2a0e74506ab0",
				"spent": false
				}
			]
			}
		}
	}
	# "#
	# );
	```
	 */
	fn get_unspent_outputs(
		&self,
		start_index: u64,
		end_index: Option<u64>,
		max: u64,
		include_proof: Option<bool>,
	) -> Result<OutputListing, Error>;

	/**
	Networked version of [Foreign::get_pmmr_indices](struct.Foreign.html#method.get_pmmr_indices).

	# Json rpc example

	```
	# grin_api::doctest_helper_json_rpc_foreign_assert_response!(
	# r#"
	{
		"jsonrpc": "2.0",
		"method": "get_pmmr_indices",
		"params": [0, 100],
		"id": 1
	}
	# "#
	# ,
	# r#"
	{
		"id": 1,
		"jsonrpc": "2.0",
		  "result": {
			"Ok": {
				  "highest_index": 398,
				  "last_retrieved_index": 2,
				  "outputs": []
			}
	}
	# "#
	# );
	```
	 */
	fn get_pmmr_indices(
		&self,
		start_block_height: u64,
		end_block_height: Option<u64>,
	) -> Result<OutputListing, Error>;

	/**
	Networked version of [Foreign::get_pool_size](struct.Foreign.html#method.get_pool_size).

	# Json rpc example

	```
	# grin_api::doctest_helper_json_rpc_foreign_assert_response!(
	# r#"
	{
		"jsonrpc": "2.0",
		"method": "get_pool_size",
		"params": [],
		"id": 1
	}
	# "#
	# ,
	# r#"
	{
		"id": 1,
		"jsonrpc": "2.0",
		"result": {
			"Ok": 1
		}
	}
	# "#
	# );
	```
	 */
	fn get_pool_size(&self) -> Result<usize, Error>;

	/**
	Networked version of [Foreign::get_stempool_size](struct.Foreign.html#method.get_stempool_size).

	# Json rpc example

	```
	# grin_api::doctest_helper_json_rpc_foreign_assert_response!(
	# r#"
	{
		"jsonrpc": "2.0",
		"method": "get_stempool_size",
		"params": [],
		"id": 1
	}
	# "#
	# ,
	# r#"
	{
		"id": 1,
		"jsonrpc": "2.0",
		"result": {
			"Ok": 0
		}
	}
	# "#
	# );
	```
	 */
	fn get_stempool_size(&self) -> Result<usize, Error>;

	/**
	Networked version of [Foreign::get_unconfirmed_transactions](struct.Foreign.html#method.get_unconfirmed_transactions).

	# Json rpc example

	```
	# grin_api::doctest_helper_json_rpc_foreign_assert_response!(
	# r#"
	{
		"jsonrpc": "2.0",
		"method": "get_unconfirmed_transactions",
		"params": [],
		"id": 1
	}
	# "#
	# ,
	# r#"
	{
		"id": 1,
		"jsonrpc": "2.0",
		"result": {
			"Ok": [
			{
				"src": "Broadcast",
				"tx": {
				"body": {
					"inputs": [
					{
						"commit": "0992ce1827ec349e9f339ce183ffd01db39bf43999799d8191bfc267a58f0a715c",
						"features": "Coinbase"
					},
					{
						"commit": "0943a3c4ee4a22a5b086c26f8e6dc534204dafde0cf4c07e0c468d224dd79127ec",
						"features": "Plain"
					}
					],
					"kernels": [
					{
						"excess": "083c49eaaf6380d44596f52cce4cf278cfac6dd34fbef73981002d8f1e8ee8abe4",
						"excess_sig": "3f011e7e288231d67f42cb4f6416c4720e6170d5e3c805a52d33aa4521328f9be0303be654bc8ddcd3111aadc27c848b9cf07e0a70885ef79be70b7bb70f8c75",
						"features": {
						"Plain": {
							"fee": 7000000
						}
						}
					}
					],
					"outputs": [
					{
						"commit": "0873fafd4a0e4f365939e24c68eeb18aafc6674ca244a364dcdbfa8fa525e7bae1",
						"features": "Plain",
						"proof": "4b675be40672d5965c43d9f03880560a8ac784ee3de8768e28c236a4bc43b8c3d4bc83dee00d2b96530af9607c3b91d9a828f0234bf2aaf7e7c0e9cf936db69c04ca1b267668fbdb2f08ce05c8b119c9d886ceaafb4634b7fae7ea01966ad825dddc9ffab8093155d9c5d268160b86fcad95f4f5e66bf46ff642a51629dbdfd7bba7936846915b925d547337a1b95c33030fad4178468825936242e631797aa3a8f0a5ae0d23040938622648c8432fc247a902abad27e383affb4ec518e4f6f55f55e264bc0f99957be203cfb26d4b8e561fb36da55a50b6ef5861134c484556d701133e1dceda5ea53e731184e0a11f33d06e13ca37d03d39dd047170580534b049862fcd6c73decc7c0af45a267ed148fe6ef2cc375ffebfa8187d2fa0a134428a036d2ec1f65d3ce036b955730fc1ee43b23b574bae2b58b7adfa2a7a45cdec393d9b658857c911560aa3c44cf4435a99d68f3dbc81c82ea43e426ef0198148a90336ee72472aab5f7feea1df93ec830fe5ec642c93c1046dec955df361bfdc3ab74477f847a1b72e8735ef65a8a6d1680745c0152bfb5cbb2a4b4671491a253a1a09d5a07d55f4872c9f0a3d25e07b257926629d5bb96aed96f5debab02503eb0ac45033323cc5a46c8e5d4469ee9f3dd618a20d54d6f5740c010fe5a0fe853efeb253a6df196bd24469ac51c1be8ba84737cecdb5ab73d7c52570d2273621fb69bd7ed985bbc6999dbd2d6fd2687ae44a391d604ff232cc6b3fbedd5d1cd0cd8c658c5d56069b5a5099cc5c9f48bbf7d7e83b4f9a7bdef6eabd164c8395468f818e8cd8c1c800bc3adfd66dbcb247d1bda5a7af38c288c0beb8e0d9160bf67500094530a0f8be52e97b5c2114f5a4a333a11c7f37f4c47a437422455d8cbcfa770cdc85ec55accf48cf14550b07f1346a02fccdf280fcb24c1fb38751d889a17e"
					},
					{
						"commit": "08de9e42d361cabd99e566c67f7f8599c7e6985cd285a841277f1aeb89ad6c8fe3",
						"features": "Plain",
						"proof": "5eb7afa00e9681e3b6425fb4256c96905303505787d6a065e88a50154410b9a371b0f879d3f97cfa00425e9c8266e180188656acdbb46cacfdfb159fb135c5eb03b08be3c231c4b21df777da2e2afe8d30db91e602dc4ceed71aeb1b45a0266cfeadc4acbf9fdf7a67f67408fbbea7bf14182bc407373d243c6875373b655695604deb575369a9b28274885601b338882219c7f508aa2a0ae1d02736af2249327145f1d3d00093f9587f0e0b408692700fac0f2a048c329e81cabaa4b997dd88923fe97420125f394e21b4835e36cce9de383d9e223df1b5a6ba6f48ffeac315991189dc2716cc7ec07f6ccc8062344d5ed4fcaddf9070f44f0c59ffe8160d1f6fdfe42b40066f51e687d38b6b5255771800ac060bd8034cd68d14eee1b2f43b6d7bf20d71549ea9a50006dd30b9a795e785385801546eb9a83721a09fc34d3b69d4ccdc0ff0fb74d224048aeb66ecff5515296cadd57f42e0717cbba7c70719a10c007db4520e868efe98a51001b67952d7bda3174195a3d76b93ee4dac60137a38b2e8309cad13ef1cfb6c467f1969385e5b334b52f4fd55da440e036d2a428e9f3be905d79f717c169060468acc6d469636fed098b1aba5cd055a120314bcab55d5b8b6889321edf373517e93ef67fbe74557ec6c0211265efefa25a34ac267cf1db891c47163bfed20d2b535abfe60390c2844dcef5f0aad5fa7f1db9f726d7f223c025861069603936a22377707cdd3915e762e7061132124c716212b0e91bb7fc5d7816366f5d169d93fe75669a6ba19057bb2450958aa6f5ada09042570f46215af5a41b623d140be574b7a8c9ab24ea48da416dbe6ec0fa3b889206fb804df8d69805ceb80f1e9d4e8b664b3939491cba946d87585c830e3dab0638fa279b5e911642f18452e2731764aa62f92bbcf194c97f344c90c1931fd2c3af4bcf6b0"
					}
					]
				},
				"offset": "0eb2c2669ce918675c72697891e5527bd13da5a499396381409219b8bbbd8129"
				},
				"tx_at": "2019-10-07T16:20:08.709114Z"
			}
			]
		}
	}
	# "#
	# );
	```
	 */
	fn get_unconfirmed_transactions(&self) -> Result<Vec<PoolEntry>, Error>;

	/**
	Networked version of [Foreign::push_transaction](struct.Foreign.html#method.push_transaction).

	# Json rpc example

	```
	# grin_api::doctest_helper_json_rpc_foreign_assert_response!(
	# r#"
	{
		"jsonrpc": "2.0",
		"method": "push_transaction",
		"params": [ {
		"body": {
				"inputs": [
				{
					"commit": "0904cbd34d0745eb00ffc3e95c9f4746738794d00268e243e9b57163a73b384102",
					"features": "Coinbase"
				}
				],
				"kernels": [
				{
					"excess": "08385257d22f1b8a758903f78ae12545245d620cffc50e7ee7bc852c5815513dc7",
					"excess_sig": "e001a7349fd40d4a9dfc1df275d30906fb3b304f8c7892a20ed5c9b10923c871cbabedcf322511a9ce56f10113b48855441f681280133e121b25ea1ff7efad9e",
					"features": {
					"Plain": {
						"fee": 8000000
					}
					}
				}
				],
				"outputs": [
				{
					"commit": "087c3ca7419751e96cdae4908bb8a92fc2826f2ad36690420b905d51beb7409ca0",
					"features": "Plain",
					"proof": "379ae236937883c2e1e613fb30f1b18d2a44d4173360e94bcd07862aafaf81b3aaa1154d67287cc03efde0d3981c6da8a18e2e426f5c30afc0f2e3a75012448402d8d56df52b87f4815575a56d4da174f8187e4faae64bf883b249ceed694271f84ef62a3711d36c997dff7a11111419011e36e3a070b7552415a55faaa3999f99439edccdfe5313277147fdb42be1798442bb225c2b546f5347920584b365aa81a0365b4a706c97c89617b0e6218d2c9bc15805caab27c438ed06340cc4f8dc7bfca0e9d38864c88bb0c834372f6b662b9159134f3f8ec9b8a87878739a7e516b97419ac29e1d4a2b250321470a9a6b98d07065bb7e79afc25a5ab6fc47108f53223078a64502bd4af1a109641447dab82741ebe3fbdbd803ee7a42fe2554e78fa86bd1d1e6e3b913118e9419b0be6f976b2404447d943b5f1bac19a5809fd6834797945a62d21b1ecb6ddebbc5ef94ca9e704d033bd64afde67bd3e06e2cca3bb10190188afc0af80b48dd862b86753d8b4af314763324deb1c97cf020cb87285a47cd28874bb91c6cdf858965e8b9daafbcbc1b4817d334a97d7e25e01b2d072d8dcc6418e3dc7b8e7712632f939238e65ed0731c7af02d55a8884cd8f7f88dc0f63a21955a7364562532f5716c89e14f8f23ad78f6fe2f1649e13ea8f8185f3ee63cc174684d1ef8d8c33fb25bc802f8e05e53fe200b1ea5231f588a020942e6fd7eec67301700088dae8816c16a337120063c21e1604e009df932032812f88be6473af13f802b42d8ad6fc14230fbe13ede178319a7b6540656234ec1f2fcfa70f6faa9c4b6b8150b81fe0fdc273a9bb385d766a02041a5c3f58471d42059c17d84d13ad592aa0ccf337970e7eef06f306b13288795123c9c005b815d848f359b23450656b310f09cda9ad4b7b6931805d47dcd10a8745d834a984e2055168ac3"
				},
				{
					"commit": "09a7b2c1d4b346c4ebe9c6c979e32e7740446624d5439d9d7abb82166c2545e5be",
					"features": "Plain",
					"proof": "5fb0ee4093a153e2ed173207dbfa02b4d185f1f313ea4cbf222558819074543f19e9bcdb595a23d4ee971aafcc614b6d2774e22cee6627bc4388297fe6ebf03e0d422f3eb8003cc8516417a6b32eb22f87e1745e0ae5bf1733f2ea253399719b1ef0067934dc548c58729604d24a44040165b32d05e82c9efc9a1f30151dd73ce893ae94709ec2fe5d0f409bb54a86604f0e92915b4f93e7adde823eccf87830ae91d71a7b99967dbcc8531fee44c20c24fb6fe2a34fe86ba5da3a9235cbcdcde033ead57d65c03903a9c9ed877bf0fab9f26d08552c64ea668d5408c84b74bc3ac8335aaaa04ebcf523d36d2207fb8770e976b6fde7d04e2148de5a4169c60b1958bb840b79a8c8f356e1f1fadc35a5a7e276fcd67c354cde546548c9bf788981f38edf5a406977826aa4524004e770b3d3cd6b26f0dc99729ffd9929fa4509b145ef0c3e4293e71b964da731a47cc9f082350acf32afb64b3b12f8383c8f2cc9880131a80ea957b2908c92f21d2db7aa5d67bafb11eb07674e52b920e67a86259dd9c5dcdd18bad182fd85ec4b659c47ea2e2e8a89c57e4d2cde87958fc2ab932e169f6805d2fb14549ac93807bc426eb4cf6d29ff6a4cf22e35dbb27f04211b06b65173501c17a3bb3ff0eecc9bb05dca23379abe457ca3010ebea69e1a2f7f3ed6531bf766007cdd1ac7d6c762785fb56f36194cc2ccaee76a499a7383288e84981b103d76cbe007f66c913eacb277746e78ae08627b279ac1f9a43ab284d8a3b32c6edcd2ea99e8ea836b31a1e2582be6c41f2282cf5fc7bdb95e4b412a5eeccad29670197873a888a100c4b2704ce75137fc997a5632d81001f9b57300a9bf99edd857065be83f835e4c49d852165ba18e1c96316c153459a913773d5d86ddc26c5cd1fff38a8fbb62506b0aef6076382674c0fa95a50a03b0c3df0a688a2cbf"
				}
				]
			},
			"offset": "0ec14d3875ad5a366418256fe65bad2a4d4ff1914e1b9488db72dd355138ca3a"
			},
			true
		],
		"id": 1
	}
	# "#
	# ,
	# r#"
	{
		"id": 1,
		"jsonrpc": "2.0",
		"result": {
			"Ok": null
		}
	}
	# "#
	# );
	```
	 */
	fn push_transaction(&self, tx: Transaction, fluff: Option<bool>) -> Result<(), Error>;
}

impl<B, P> ForeignRpc for Foreign<B, P>
where
	B: BlockChain,
	P: PoolAdapter,
{
	fn get_header(
		&self,
		height: Option<u64>,
		hash: Option<String>,
		commit: Option<String>,
	) -> Result<BlockHeaderPrintable, Error> {
		let mut parsed_hash: Option<Hash> = None;
		if let Some(hash) = hash {
			let vec = util::from_hex(&hash)
				.map_err(|e| Error::Argument(format!("invalid block hash: {}", e)))?;
			parsed_hash = Some(Hash::from_vec(&vec));
		}
		Foreign::get_header(self, height, parsed_hash, commit)
	}
	fn get_block(
		&self,
		height: Option<u64>,
		hash: Option<String>,
		commit: Option<String>,
	) -> Result<BlockPrintable, Error> {
		let mut parsed_hash: Option<Hash> = None;
		if let Some(hash) = hash {
			let vec = util::from_hex(&hash)
				.map_err(|e| Error::Argument(format!("invalid block hash: {}", e)))?;
			parsed_hash = Some(Hash::from_vec(&vec));
		}
		Foreign::get_block(self, height, parsed_hash, commit)
	}

	fn get_version(&self) -> Result<Version, Error> {
		Foreign::get_version(self)
	}

	fn get_tip(&self) -> Result<Tip, Error> {
		Foreign::get_tip(self)
	}

	fn get_kernel(
		&self,
		excess: String,
		min_height: Option<u64>,
		max_height: Option<u64>,
	) -> Result<LocatedTxKernel, Error> {
		Foreign::get_kernel(self, excess, min_height, max_height)
	}

	fn get_outputs(
		&self,
		commits: Option<Vec<String>>,
		start_height: Option<u64>,
		end_height: Option<u64>,
		include_proof: Option<bool>,
		include_merkle_proof: Option<bool>,
	) -> Result<Vec<OutputPrintable>, Error> {
		Foreign::get_outputs(
			self,
			commits,
			start_height,
			end_height,
			include_proof,
			include_merkle_proof,
		)
	}

	fn get_unspent_outputs(
		&self,
		start_index: u64,
		end_index: Option<u64>,
		max: u64,
		include_proof: Option<bool>,
	) -> Result<OutputListing, Error> {
		Foreign::get_unspent_outputs(self, start_index, end_index, max, include_proof)
	}

	fn get_pmmr_indices(
		&self,
		start_block_height: u64,
		end_block_height: Option<u64>,
	) -> Result<OutputListing, Error> {
		Foreign::get_pmmr_indices(self, start_block_height, end_block_height)
	}

	fn get_pool_size(&self) -> Result<usize, Error> {
		Foreign::get_pool_size(self)
	}

	fn get_stempool_size(&self) -> Result<usize, Error> {
		Foreign::get_stempool_size(self)
	}

	fn get_unconfirmed_transactions(&self) -> Result<Vec<PoolEntry>, Error> {
		Foreign::get_unconfirmed_transactions(self)
	}
	fn push_transaction(&self, tx: Transaction, fluff: Option<bool>) -> Result<(), Error> {
		Foreign::push_transaction(self, tx, fluff)
	}
}

#[doc(hidden)]
#[macro_export]
macro_rules! doctest_helper_json_rpc_foreign_assert_response {
	($request:expr, $expected_response:expr) => {
		// create temporary grin server, run jsonrpc request on node api, delete server, return
		// json response.

		{
			/*use grin_servers::test_framework::framework::run_doctest;
			use grin_util as util;
			use serde_json;
			use serde_json::Value;
			use tempfile::tempdir;

			let dir = tempdir().map_err(|e| format!("{:#?}", e)).unwrap();
			let dir = dir
				.path()
				.to_str()
				.ok_or("Failed to convert tmpdir path to string.".to_owned())
				.unwrap();

			let request_val: Value = serde_json::from_str($request).unwrap();
			let expected_response: Value = serde_json::from_str($expected_response).unwrap();
			let response = run_doctest(
				request_val,
				dir,
				$use_token,
				$blocks_to_mine,
				$perform_tx,
				$lock_tx,
				$finalize_tx,
					)
			.unwrap()
			.unwrap();
			if response != expected_response {
				panic!(
					"(left != right) \nleft: {}\nright: {}",
					serde_json::to_string_pretty(&response).unwrap(),
					serde_json::to_string_pretty(&expected_response).unwrap()
				);
				}*/
		}
	};
}
