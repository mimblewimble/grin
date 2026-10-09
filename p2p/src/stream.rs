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

//! Optional TLS wrapping for P2P connections.
//!
//! When enabled, Grin P2P traffic is encrypted with TLS for privacy against
//! passive observers (packet inspection). Authentication is intentionally
//! *not* performed: peers use self-signed certificates and clients accept any
//! certificate. This matches the goal of issue #1420 (privacy, not PKI).
//!
//! Both peers must enable TLS to connect. Plaintext remains the default for
//! compatibility with the existing network.

use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::{
	CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer, ServerName, UnixTime,
};
use rustls::{
	ClientConfig, ClientConnection, DigitallySignedStruct, Error as TlsError, ServerConfig,
	ServerConnection, SignatureScheme, StreamOwned,
};
use std::fs::{self, File};
use std::io::{self, BufReader, Read, Write};
use std::net::{Shutdown, SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Ensure the rustls crypto provider is installed once.
fn ensure_crypto_provider() {
	let _ = rustls::crypto::ring::default_provider().install_default();
}

/// TLS context shared by the P2P server (server + client configs).
#[derive(Clone)]
pub struct TlsContext {
	server_config: Arc<ServerConfig>,
	client_config: Arc<ClientConfig>,
}

impl TlsContext {
	/// Build a TLS context from PEM certificate / key paths.
	/// If the files do not exist and `auto_dir` is set, generate a self-signed
	/// certificate there.
	pub fn new(
		cert_path: Option<&Path>,
		key_path: Option<&Path>,
		auto_dir: Option<&Path>,
	) -> io::Result<TlsContext> {
		ensure_crypto_provider();

		let (cert_path, key_path) = match (cert_path, key_path, auto_dir) {
			(Some(c), Some(k), _) => (c.to_path_buf(), k.to_path_buf()),
			(_, _, Some(dir)) => {
				fs::create_dir_all(dir)?;
				let cert = dir.join("cert.pem");
				let key = dir.join("key.pem");
				if !cert.exists() || !key.exists() {
					generate_self_signed(&cert, &key)?;
					info!(
						"Generated self-signed P2P TLS certificate at {}",
						cert.display()
					);
				}
				(cert, key)
			}
			_ => {
				return Err(io::Error::new(
					io::ErrorKind::InvalidInput,
					"tls_enabled requires certificate paths or a data directory for auto-generation",
				));
			}
		};

		let certs = load_certs(&cert_path)?;
		let key = load_private_key(&key_path)?;

		let server_config = ServerConfig::builder()
			.with_no_client_auth()
			.with_single_cert(certs, key)
			.map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;

		// Privacy-only: accept any peer certificate (self-signed OK).
		let client_config = ClientConfig::builder()
			.dangerous()
			.with_custom_certificate_verifier(Arc::new(NoCertificateVerification))
			.with_no_client_auth();

		Ok(TlsContext {
			server_config: Arc::new(server_config),
			client_config: Arc::new(client_config),
		})
	}

	/// Perform a TLS server handshake over an accepted TCP stream.
	pub fn accept(&self, tcp: TcpStream) -> io::Result<Stream> {
		let conn = ServerConnection::new(Arc::clone(&self.server_config))
			.map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
		let mut stream = StreamOwned::new(conn, tcp);
		// Complete handshake by forcing a write/read cycle.
		handshake_server(&mut stream)?;
		Ok(Stream::tls_server(stream))
	}

	/// Perform a TLS client handshake over an outbound TCP stream.
	pub fn connect(&self, tcp: TcpStream) -> io::Result<Stream> {
		// Server name is unused for verification (privacy-only TLS).
		let name = ServerName::try_from("grin-p2p")
			.map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "invalid server name"))?
			.to_owned();
		let conn = ClientConnection::new(Arc::clone(&self.client_config), name)
			.map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
		let mut stream = StreamOwned::new(conn, tcp);
		handshake_client(&mut stream)?;
		Ok(Stream::tls_client(stream))
	}
}

fn handshake_server(stream: &mut StreamOwned<ServerConnection, TcpStream>) -> io::Result<()> {
	// Drive the handshake until complete.
	while stream.conn.is_handshaking() {
		if stream.conn.wants_read() {
			stream.conn.read_tls(&mut stream.sock)?;
			stream
				.conn
				.process_new_packets()
				.map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
		}
		if stream.conn.wants_write() {
			stream.conn.write_tls(&mut stream.sock)?;
		}
	}
	Ok(())
}

fn handshake_client(stream: &mut StreamOwned<ClientConnection, TcpStream>) -> io::Result<()> {
	while stream.conn.is_handshaking() {
		if stream.conn.wants_write() {
			stream.conn.write_tls(&mut stream.sock)?;
		}
		if stream.conn.wants_read() {
			stream.conn.read_tls(&mut stream.sock)?;
			stream
				.conn
				.process_new_packets()
				.map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
		}
	}
	Ok(())
}

/// Unified plain / TLS peer stream used throughout the p2p crate.
pub struct Stream {
	inner: StreamInner,
}

enum StreamInner {
	Plain(TcpStream),
	Tls(Arc<Mutex<TlsInner>>),
}

enum TlsInner {
	Client(StreamOwned<ClientConnection, TcpStream>),
	Server(StreamOwned<ServerConnection, TcpStream>),
}

impl Stream {
	pub fn plain(tcp: TcpStream) -> Stream {
		Stream {
			inner: StreamInner::Plain(tcp),
		}
	}

	fn tls_client(s: StreamOwned<ClientConnection, TcpStream>) -> Stream {
		Stream {
			inner: StreamInner::Tls(Arc::new(Mutex::new(TlsInner::Client(s)))),
		}
	}

	fn tls_server(s: StreamOwned<ServerConnection, TcpStream>) -> Stream {
		Stream {
			inner: StreamInner::Tls(Arc::new(Mutex::new(TlsInner::Server(s)))),
		}
	}

	pub fn try_clone(&self) -> io::Result<Stream> {
		match &self.inner {
			StreamInner::Plain(s) => Ok(Stream::plain(s.try_clone()?)),
			StreamInner::Tls(t) => Ok(Stream {
				inner: StreamInner::Tls(Arc::clone(t)),
			}),
		}
	}

	pub fn set_read_timeout(&self, dur: Option<Duration>) -> io::Result<()> {
		match &self.inner {
			StreamInner::Plain(s) => s.set_read_timeout(dur),
			StreamInner::Tls(t) => {
				let mut g = t.lock().map_err(|_| {
					io::Error::new(io::ErrorKind::Other, "tls stream mutex poisoned")
				})?;
				match &mut *g {
					TlsInner::Client(s) => s.sock.set_read_timeout(dur),
					TlsInner::Server(s) => s.sock.set_read_timeout(dur),
				}
			}
		}
	}

	pub fn set_write_timeout(&self, dur: Option<Duration>) -> io::Result<()> {
		match &self.inner {
			StreamInner::Plain(s) => s.set_write_timeout(dur),
			StreamInner::Tls(t) => {
				let mut g = t.lock().map_err(|_| {
					io::Error::new(io::ErrorKind::Other, "tls stream mutex poisoned")
				})?;
				match &mut *g {
					TlsInner::Client(s) => s.sock.set_write_timeout(dur),
					TlsInner::Server(s) => s.sock.set_write_timeout(dur),
				}
			}
		}
	}

	pub fn peer_addr(&self) -> io::Result<SocketAddr> {
		match &self.inner {
			StreamInner::Plain(s) => s.peer_addr(),
			StreamInner::Tls(t) => {
				let g = t.lock().map_err(|_| {
					io::Error::new(io::ErrorKind::Other, "tls stream mutex poisoned")
				})?;
				match &*g {
					TlsInner::Client(s) => s.sock.peer_addr(),
					TlsInner::Server(s) => s.sock.peer_addr(),
				}
			}
		}
	}

	pub fn shutdown(&self, how: Shutdown) -> io::Result<()> {
		match &self.inner {
			StreamInner::Plain(s) => s.shutdown(how),
			StreamInner::Tls(t) => {
				let g = t.lock().map_err(|_| {
					io::Error::new(io::ErrorKind::Other, "tls stream mutex poisoned")
				})?;
				match &*g {
					TlsInner::Client(s) => s.sock.shutdown(how),
					TlsInner::Server(s) => s.sock.shutdown(how),
				}
			}
		}
	}

}

impl Read for Stream {
	fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
		match &mut self.inner {
			StreamInner::Plain(s) => s.read(buf),
			StreamInner::Tls(t) => {
				let mut g = t.lock().map_err(|_| {
					io::Error::new(io::ErrorKind::Other, "tls stream mutex poisoned")
				})?;
				match &mut *g {
					TlsInner::Client(s) => s.read(buf),
					TlsInner::Server(s) => s.read(buf),
				}
			}
		}
	}
}

impl Write for Stream {
	fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
		match &mut self.inner {
			StreamInner::Plain(s) => s.write(buf),
			StreamInner::Tls(t) => {
				let mut g = t.lock().map_err(|_| {
					io::Error::new(io::ErrorKind::Other, "tls stream mutex poisoned")
				})?;
				match &mut *g {
					TlsInner::Client(s) => s.write(buf),
					TlsInner::Server(s) => s.write(buf),
				}
			}
		}
	}

	fn flush(&mut self) -> io::Result<()> {
		match &mut self.inner {
			StreamInner::Plain(s) => s.flush(),
			StreamInner::Tls(t) => {
				let mut g = t.lock().map_err(|_| {
					io::Error::new(io::ErrorKind::Other, "tls stream mutex poisoned")
				})?;
				match &mut *g {
					TlsInner::Client(s) => s.flush(),
					TlsInner::Server(s) => s.flush(),
				}
			}
		}
	}
}

/// Verifier that accepts any certificate (privacy-only P2P TLS).
#[derive(Debug)]
struct NoCertificateVerification;

impl ServerCertVerifier for NoCertificateVerification {
	fn verify_server_cert(
		&self,
		_end_entity: &CertificateDer<'_>,
		_intermediates: &[CertificateDer<'_>],
		_server_name: &ServerName<'_>,
		_ocsp_response: &[u8],
		_now: UnixTime,
	) -> Result<ServerCertVerified, TlsError> {
		Ok(ServerCertVerified::assertion())
	}

	fn verify_tls12_signature(
		&self,
		_message: &[u8],
		_cert: &CertificateDer<'_>,
		_dss: &DigitallySignedStruct,
	) -> Result<HandshakeSignatureValid, TlsError> {
		Ok(HandshakeSignatureValid::assertion())
	}

	fn verify_tls13_signature(
		&self,
		_message: &[u8],
		_cert: &CertificateDer<'_>,
		_dss: &DigitallySignedStruct,
	) -> Result<HandshakeSignatureValid, TlsError> {
		Ok(HandshakeSignatureValid::assertion())
	}

	fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
		rustls::crypto::ring::default_provider()
			.signature_verification_algorithms
			.supported_schemes()
	}
}

fn load_certs(path: &Path) -> io::Result<Vec<CertificateDer<'static>>> {
	let file = File::open(path)?;
	let mut reader = BufReader::new(file);
	let certs = rustls_pemfile::certs(&mut reader)
		.map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
	Ok(certs.into_iter().map(CertificateDer::from).collect())
}

fn load_private_key(path: &Path) -> io::Result<PrivateKeyDer<'static>> {
	let file = File::open(path)?;
	let mut reader = BufReader::new(file);
	let keys = rustls_pemfile::pkcs8_private_keys(&mut reader)
		.map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
	if keys.len() != 1 {
		return Err(io::Error::new(
			io::ErrorKind::InvalidData,
			"expected a single PKCS#8 private key",
		));
	}
	Ok(PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(keys[0].clone())))
}

fn generate_self_signed(cert_path: &Path, key_path: &Path) -> io::Result<()> {
	let key_pair = rcgen::KeyPair::generate()
		.map_err(|e| io::Error::new(io::ErrorKind::Other, e.to_string()))?;
	let mut params = rcgen::CertificateParams::new(vec!["grin-p2p".to_string()])
		.map_err(|e| io::Error::new(io::ErrorKind::Other, e.to_string()))?;
	params
		.distinguished_name
		.push(rcgen::DnType::CommonName, "grin-p2p");
	let cert = params
		.self_signed(&key_pair)
		.map_err(|e| io::Error::new(io::ErrorKind::Other, e.to_string()))?;

	fs::write(cert_path, cert.pem())?;
	fs::write(key_path, key_pair.serialize_pem())?;
	Ok(())
}

/// Default subdirectory under the node data root for auto-generated certs.
pub fn default_tls_dir(db_root: &str) -> PathBuf {
	Path::new(db_root).join("p2p_tls")
}
