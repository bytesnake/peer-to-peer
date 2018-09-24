use std::collections::VecDeque;
use libc::{c_uint, c_int, EPOLLIN, EPOLLOUT, socket, AF_INET, sockaddr_in, in_addr};
use std::net::SocketAddrV4;
use std::collections::HashMap;
use std::cmp::Reverse;

use priority_queue::PriorityQueue;

type Addr = SocketAddrV4;
type Fd = c_int;
type EpollId = u64;

#[derive(Clone)]
pub struct Node {
    fd: Fd,
    addr: Addr,
    latency: u64
}

#[derive(Hash, PartialEq, Eq, Clone, Debug)]
pub enum Event {
    SendPacket(Fd, Vec<u8>),
    Connect(Fd, Fd)
}

#[derive(Clone)]
pub enum Log {
    AddNode(Addr, u32),
    SendPacket(Fd, Vec<u8>),
    Connect(Fd, Addr)
}


#[derive(Clone)]
pub struct State {
    nodes: HashMap<Addr, Node>,
    connections: HashMap<Fd, Fd>,
    events: PriorityQueue<Event, Reverse<u64>>,
    epoll_notify: VecDeque<(Fd, c_int)>,
    epoll: Vec<(Fd, c_int, EpollId)>,
    timer: u64,
    logs: Vec<Log>
}

impl State {
    pub fn new() -> State {
        State { 
            nodes: HashMap::new(),
            connections: HashMap::new(),
            events: PriorityQueue::new(),
            epoll_notify: VecDeque::new(),
            epoll: Vec::new(),
            timer: 0,
            logs: Vec::new()
        }
    }

    pub fn add_node(&mut self, fd: Fd, addr: Addr) {
        //println!(" ===> a new node was created with addr {} ({})", addr, fd);

        self.logs.push(Log::AddNode(addr.clone(), 200));
        self.nodes.insert(
            addr.clone(), 
            Node { fd, addr, latency: 200 }
        );
    }

    pub fn add_epoll_fd(&mut self, fd: Fd, events: c_uint, id: EpollId) {
        self.epoll.push((fd, events as i32, id));
    }

    pub fn find_epoll_fd(&self, fd: Fd) -> Option<(c_int, EpollId)> {
        self.epoll.iter().filter_map(|x| {
            if x.0 == fd {
                Some((x.1, x.2))
            } else {
                None
            }
        }).next()
    }

    pub fn addr_by_fd(&self, fd: Fd) -> Addr {
        self.nodes.values().filter_map(|x| {
            if x.fd == fd {
                Some(x.addr.clone())
            } else {
                None
            }
        }).next().unwrap()
    }

    pub fn connect_to_node(&mut self, fd: Fd, addr: Addr) {
        //println!(" ===> try to connect to addr {} ({})", addr, fd);

        let (latency, to_fd) = self.nodes.get(&addr)
            .map(|x| (x.latency, x.fd)).unwrap();

        // push event with file descriptors (later used by accept)
        self.events.push(Event::Connect(fd, to_fd), Reverse(self.timer + latency));
        self.logs.push(Log::Connect(fd, addr));
    }

    pub fn accept(&mut self, fd: Fd) -> Option<Fd> {
        self.find_connect_event(fd).map(|(origin, dest)| {
            //println!(" ===> accept connect from {} to {}", origin, dest);

            // create a new valid Fd (pseudo connection)
            let new_fd = unsafe { socket(AF_INET, 524289, 0) };

            // add the new connection
            self.connections.insert(origin, new_fd);
            self.connections.insert(new_fd, origin);

            self.epoll_notify.push_back((origin, EPOLLOUT));
            self.epoll_notify.push_back((new_fd, EPOLLOUT));

            // increase timer
            self.timer += 200;
            
            new_fd
        })
        }

    pub fn get_sockname(&self, fd: Fd) -> Addr {
        self.addr_by_fd(fd)
    }

    pub fn recv_from(&mut self, fd: Fd) -> Option<Vec<u8>> {
        //println!("RECV! {}", fd);
        self.find_send_event(fd).map(|buf| {
            self.timer += 200;
            //println!(" ===> recv packets in {} {:?}", fd, buf);

            buf
        })
    }

    pub fn send_to(&mut self, fd: Fd, buf: &[u8]) -> Option<()> {
        if let Some(dest) = self.connections.get(&fd) {
            //println!(" ===> send packet from {} to {} {:?}", fd, dest, buf);

            self.logs.push(Log::SendPacket(*dest, buf.into()));

            // notify fd to be writable again
            self.epoll_notify.push_back((fd, EPOLLOUT));

            self.events.push(Event::SendPacket(*dest, buf.into()), Reverse(self.timer+200));

            return Some(());
        } else {
            return None;
        }
    }

    pub fn next_epoll_id(&mut self) -> Option<(EpollId, c_int)> {
        let mut ret = None;

        // wait till epoll_ctl was called and we have a epoll id
        match self.events.peek() {
            Some((Event::Connect(_, b),_)) => {
                if let Some(id) = self.find_epoll_fd(*b) {
                    ret = Some((id.1, EPOLLIN|EPOLLOUT));
                }
            },
            Some((Event::SendPacket(a, _),_)) => {
                if let Some(id) = self.find_epoll_fd(*a) {
                    ret = Some((id.1, EPOLLIN));
                }
            },
            None => {}
        }

        if let Some((id, _)) = ret {
            //println!(" ===> wake up {} with events {}", id, self.events().join(","));
        }

        ret
    }

    pub fn next_epoll_notify(&mut self) -> Option<(EpollId, c_int)> {
        let res = self.epoll_notify.front().and_then(|x| {
            self.find_epoll_fd(x.0).map(|y| (y.1, x.1))
        });

        if res.is_some() {
        //println!("{:?}", self.epoll_notify);
            self.epoll_notify.pop_front();
        }

        res
    }

    pub fn get_epoll(&self) -> VecDeque<(Fd, c_int)> {
        self.epoll_notify.clone()
    }

    pub fn events(&self) -> Vec<String> {
        self.events.clone().into_sorted_iter().map(|(x, time)| match x {
            Event::SendPacket(x,_) => format!("send_packet({},{})", x, time.0),
            Event::Connect(a,b) => format!("connect({},{})", a, b)
        }).collect::<Vec<String>>()
    }

    pub fn are_there_events(&self) -> bool {
        self.events.is_empty()
    }

    pub fn find_connect_event(&mut self, fd: Fd) -> Option<(Fd, Fd)> {
        let mut res = None;
        let mut removed = Vec::new();
        let mut first_time = match self.events.peek() {
            Some(x) => (x.1).0,
            None => return None
        };

        'outer: loop {
            if let Some((first, time)) = self.events.pop() {
                match first {
                    Event::Connect(a, b) => {
                        if first_time == time.0 && b == fd {
                            res = Some((a, b));

                            break 'outer;
                        }
                    },
                    _ => {}
                }

                removed.push((first, time));

                if first_time != time.0 {
                    break;
                }
            } else {
                break;
            }
        }

        for (elm, time) in removed {
            self.events.push(elm, time);
        }

        res
    }

    pub fn find_send_event(&mut self, fd: Fd) -> Option<Vec<u8>> {
        let mut res = None;
        let mut removed = Vec::new();
        let mut first_time = match self.events.peek() {
            Some(x) => (x.1).0,
            None => return None
        };

        'outer: loop {
            if let Some((first, time)) = self.events.pop() {
                match first {
                    Event::SendPacket(a, ref b) => {
                        if first_time == time.0 && a == fd {
                            res = Some(b.clone());

                            break 'outer;
                        }
                    },
                    _ => {}
                }

                removed.push((first, time));

                if first_time != time.0 {
                    break;
                }
            } else {
                break;
            }
        }

        for (elm, time) in removed {
            self.events.push(elm, time);
        }

        res
    }

    pub fn get_logs(&self) -> Vec<u8> {
        unsafe {
            any_as_u8_slice(&self.logs).into()
        }
    }
}

pub fn empty_addr() -> sockaddr_in {
    to_sockaddr(SocketAddrV4::new([127,0,0,1].into(), 8000))
}

pub fn to_sockaddr(addr: Addr) -> sockaddr_in{
    let ip_addr = u32::from(addr.ip().clone()).to_be();

    sockaddr_in {
        sin_family: AF_INET as u16,
        sin_port: addr.port().to_be(),
        sin_addr: in_addr { s_addr: ip_addr },
        sin_zero: [0u8; 8]
    }
}

unsafe fn any_as_u8_slice<T: Sized>(p: &T) -> &[u8] {
    ::std::slice::from_raw_parts(
        (p as *const T) as *const u8,
        ::std::mem::size_of::<T>(),
    )
}
