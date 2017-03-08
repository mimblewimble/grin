// Copyright 2016 The Grin Developers
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

use std::{io, str};
use std::convert::From;
use std::marker::PhantomData;
use std::net::SocketAddr;
use std::sync::Arc;
use std::thread;

use net2;

use futures::{future, Future, Stream};
use tokio_core::io::{Io, Codec, EasyBuf, Framed};
use tokio_core::net::{TcpStream, TcpListener};
use tokio_core::reactor::{Core, Handle};
use tokio_proto::{TcpClient, TcpServer, BindClient, BindServer};
use tokio_proto::streaming::{Message, Body};
use tokio_proto::streaming::multiplex::{Frame, ServerProto, ClientProto};
use tokio_service::{Service, NewService};

use core::ser;
use msg::*;

struct GrinCodec {
  decoding_head: bool,
}

impl Codec for GrinCodec {
  type In = Frame<MsgHeader, Vec<u8>, io::Error>;
  type Out = Frame<MsgHeader, Vec<u8>, io::Error>;
 
  fn encode(&mut self, msg: Self::In, mut buf: &mut Vec<u8>) -> io::Result<()> {
    match msg {
      Frame::Message{id, message, ..} => {
        ser::serialize(&mut buf, &message).map_err(|e| io::Error::new(io::ErrorKind::Other, format!("Encoding error: {:?}", e)))?;
      },
      Frame::Body{id, chunk} => {
        if let Some(chunk) = chunk {
          buf.extend(chunk);
        }
      },
      Frame::Error{error, ..} => return Err(error),
    }
    Ok(())
  }

  fn decode(&mut self, buf: &mut EasyBuf) -> Result<Option<Self::Out>, io::Error> {
    unimplemented!();
  }
}

struct GrinProto;

impl <T: Io + 'static> ServerProto<T> for GrinProto {
  type Request = MsgHeader;
  type RequestBody = Vec<u8>;
  type Response = MsgHeader;
  type ResponseBody = Vec<u8>;
  type Error = io::Error;

  type Transport = Framed<T, GrinCodec>;
  type BindTransport = Result<Self::Transport, io::Error>;

  fn bind_transport(&self, io: T) -> Self::BindTransport {
    Ok(io.framed(GrinCodec{decoding_head: true}))
  }
}

struct GrinReceiver;

impl Service for GrinReceiver {
  type Request = Message<MsgHeader, Body<Vec<u8>, io::Error>>;
  type Response = Message<MsgHeader, Body<Vec<u8>, io::Error>>;
  type Error = io::Error;
  type Future = Box<Future<Item = Self::Response, Error = Self::Error>>;

  fn call(&self, req: Self::Request) -> Self::Future {
    let header = req.get_ref();
    let response = match header.msg_type {
      Type::Ping => {
        let data = ser::ser_vec(&MsgHeader::new(Type::Pong, 0)).unwrap();
        Message::WithoutBody(MsgHeader::new(Type::Pong, 0))
      },
      _ => {
        unimplemented!()
      }
    };
    Box::new(future::ok(response))
  }
}

struct GrinClient;

impl Service for GrinClient {
  type Request = Message<MsgHeader, Body<Vec<u8>, io::Error>>;
  type Response = Message<MsgHeader, Body<Vec<u8>, io::Error>>;
  type Error = io::Error;
  type Future = Box<Future<Item = Self::Response, Error = Self::Error>>;

  fn call(&self, req: Self::Request) -> Self::Future {
    unimplemented!();
  }
}

pub struct TcpClientServer<Kind, P> {
  _kind: PhantomData<Kind>,
  proto: Arc<P>,
  threads: usize,
  addr: SocketAddr,
}

impl<Kind, P> TcpClientServer<Kind, P> where
  P: BindServer<Kind, TcpStream> + BindClient<Kind, TcpStream> + Send + Sync + 'static {

  pub fn new(protocol: P, addr: SocketAddr) -> TcpClientServer<Kind, P> {
    TcpClientServer{
      _kind: PhantomData,
      proto: Arc::new(protocol),
      threads: 1,
      addr: addr,
    }
  }

    /// Set the number of threads running simultaneous event loops (Unix only).
    pub fn threads(&mut self, threads: usize) {
        assert!(threads > 0);
        if cfg!(unix) {
            self.threads = threads;
        }
    }

    /// Start up the server, providing the given service on it.
    ///
    /// This method will block the current thread until the server is shut down.
    pub fn serve<S>(&self, new_service: S) where
        S: NewService<Request = <P as BindServer<Kind, TcpStream>>::ServiceRequest,
                      Response = <P as BindServer<Kind, TcpStream>>::ServiceResponse,
                      Error = <P as BindServer<Kind, TcpStream>>::ServiceError> + Send + Sync + 'static,
    {
        let new_service = Arc::new(new_service);
        self.with_handle(move |_| new_service.clone())
    }

    /// Start up the server, providing the given service on it, and providing
    /// access to the event loop handle.
    ///
    /// The `new_service` argument is a closure that is given an event loop
    /// handle, and produces a value implementing `NewService`. That value is in
    /// turned used to make a new service instance for each incoming connection.
    ///
    /// This method will block the current thread until the server is shut down.
    pub fn with_handle<F, S>(&self, new_service: F) where
        F: Fn(&Handle) -> S + Send + Sync + 'static,
        S: NewService<Request = <P as BindServer<Kind, TcpStream>>::ServiceRequest,
                      Response = <P as BindServer<Kind, TcpStream>>::ServiceResponse,
                      Error = <P as BindServer<Kind, TcpStream>>::ServiceError> + Send + Sync + 'static,
    {
        let proto = self.proto.clone();
        let new_service = Arc::new(new_service);
        let addr = self.addr;
        let workers = self.threads;

        let threads = (0..self.threads - 1).map(|i| {
            let proto = proto.clone();
            let new_service = new_service.clone();

            thread::Builder::new().name(format!("worker{}", i)).spawn(move || {
                serve(proto, addr, workers, &*new_service)
            }).unwrap()
        }).collect::<Vec<_>>();

        serve(proto, addr, workers, &*new_service);

        for thread in threads {
            thread.join().unwrap();
        }
    }
}

fn serve<P, Kind, F, S>(binder: Arc<P>, addr: SocketAddr, workers: usize, new_service: &F)
    where P: BindServer<Kind, TcpStream> + BindClient<Kind, TcpStream>,
          F: Fn(&Handle) -> S,
          S: NewService<Request = <P as BindServer<Kind, TcpStream>>::ServiceRequest,
                        Response = <P as BindServer<Kind, TcpStream>>::ServiceResponse,
                        Error = <P as BindServer<Kind, TcpStream>>::ServiceError> + 'static,
{
    let mut core = Core::new().unwrap();
    let handle = core.handle();
    let new_service = new_service(&handle);
    let listener = listener(&addr, workers, &handle).unwrap();

    let server = listener.incoming().for_each(move |(socket, _)| {
        // Create the service
        let service = try!(new_service.new_service());

        // Bind it!
        binder.bind_server(&handle, socket, service);
        binder.bind_client(&handle, socket);

        Ok(())
    });

    core.run(server).unwrap();
}

fn listener(addr: &SocketAddr,
            workers: usize,
            handle: &Handle) -> io::Result<TcpListener> {
    let listener = match *addr {
        SocketAddr::V4(_) => try!(net2::TcpBuilder::new_v4()),
        SocketAddr::V6(_) => try!(net2::TcpBuilder::new_v6()),
    };
    // TODO re-add
    // try!(configure_tcp(workers, &listener));
    try!(listener.reuse_address(true));
    try!(listener.bind(addr));
    listener.listen(1024).and_then(|l| {
        TcpListener::from_listener(l, addr, handle)
    })
}

