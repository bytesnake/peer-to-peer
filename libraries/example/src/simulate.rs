use std::env::args;
use std::collections::HashMap;
use std::time::{Duration, Instant};

use futures::{Future, Stream, IntoFuture, future};
use tokio::timer;
use std::net::{SocketAddr, SocketAddrV4, Ipv4Addr};

use rand::Rng;

struct AddressSpace(HashMap<u32, ()>);

impl AddressSpace {
    pub fn new() -> AddressSpace {
        AddressSpace(HashMap::new())
    }

    pub fn generate(&mut self) -> SocketAddr {
        let mut addr: u32 = rand::random();

        while self.0.contains_key(&addr) {
            addr = rand::random();
        }

        self.0.insert(addr, ());

        let addr = Ipv4Addr::from(addr);

        SocketAddr::from(SocketAddrV4::new(addr, 8000))
    }


    pub fn pick(&self) -> SocketAddr {
        let keys: Vec<u32> = self.0.keys().cloned().collect();

        let addr = Ipv4Addr::from(*rand::thread_rng().choose(&keys).unwrap());
        SocketAddr::from(SocketAddrV4::new(addr, 8000))
    }
}

pub fn start(num_nodes: usize) {
    let mut addrs = AddressSpace::new();
    let mut nodes = Vec::new();

    for i in 0..num_nodes {
        let contact = match i {
            0 => None,
            _ => Some(addrs.pick())
        };

        let gossip = gossip::Gossip::new(addrs.generate().into(), contact.into(), format!("Node {}", i));

        /*if i == 0 {
            let writer = gossip.writer();

            let writer = timer::Interval::new(Instant::now() + Duration::from_millis(1000),Duration::from_millis(1000))
                .for_each(move |_| {
                    writer.push(vec![0]);
                    Ok(())
                }).map_err(|_| ()).into_future();

            rt.spawn(writer);
        }*/

        let gossip = gossip.for_each(|block| {
                println!("New block: {:?}", block);

                Ok(())
            }).into_future().map_err(|_| ());


        nodes.push(gossip);
    }

    tokio::run(future::join_all(nodes).map(|_| ()).map_err(|_| ()));
}
