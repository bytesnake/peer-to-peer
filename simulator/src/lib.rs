extern crate libc;
extern crate priority_queue;
extern crate errno;

#[macro_use]
extern crate lazy_static;

#[macro_use]
extern crate redhook;

mod state;

use std::ptr;
use std::net::{Ipv4Addr, SocketAddrV4};
use libc::{c_int, c_void, size_t, ssize_t, sockaddr, socklen_t,AF_INET, sockaddr_in, epoll_event, EPOLL_CTL_ADD, EPOLLIN, EPOLLOUT, EPOLLRDHUP, EPOLLPRI, EPOLLERR, EPOLLHUP, EPOLLET, EPOLLONESHOT, EPOLLWAKEUP, EPOLLEXCLUSIVE, c_uint, mode_t, EAGAIN, EWOULDBLOCK, iovec};
use errno::{set_errno, Errno};

use std::sync::{Condvar, Mutex, atomic::AtomicBool, atomic::Ordering};
use std::slice;
use state::State;
use std::time::Duration;

lazy_static! {
    static ref STATE: Mutex<State> = Mutex::new(State::new());
    static ref SYNC: (Mutex<bool>, Condvar) = (Mutex::new(false), Condvar::new());
    static ref PENDING: AtomicBool= AtomicBool::new(false);
}

hook! {
    unsafe fn get_state() -> State => fake_get_state {
        STATE.lock().unwrap().clone()
    }
}

hook! {
    unsafe fn readv(fd: c_int, iov: *mut iovec, iovcnt: c_int) -> ssize_t => fake_readv {
        if fd < 2 {
            return real!(readv)(fd, iov, iovcnt);
        }

        print!("R");
        if let Some(arr) = STATE.lock().unwrap().recv_from(fd) {
            (*iov).iov_len = arr.len();
            let buf_ptr = arr.as_ptr();

            ptr::copy(buf_ptr, (*iov).iov_base as *mut u8, arr.len());

            arr.len() as ssize_t
        } else {
            //println!("invalid read in {}!", fd);
            PENDING.store(false, Ordering::Relaxed);
            SYNC.1.notify_one();

            set_errno(Errno(EWOULDBLOCK));

            -1
        }
    }
}

hook! {
    unsafe fn send(fd: c_int, buf: *const c_void, len: size_t, _flags: c_int) -> ssize_t => fake_send {
        print!("W");
        let buf = slice::from_raw_parts(buf as *const u8, len);

        STATE.lock().unwrap().send_to(fd, buf);

        if !PENDING.load(Ordering::Relaxed) {
            // wake up the epoll_wait thread
            SYNC.1.notify_one();
        }

        len as ssize_t
    }
}

hook! {
    unsafe fn bind(ssocket: c_int, address: *const sockaddr, _address_len: socklen_t) -> c_int => fake_bind {
        if (*address).sa_family == AF_INET as u16 {
            let addr = address as *const sockaddr_in;

            // convert host and port to SocketAddrV4
            let port = (*addr).sin_port.to_be();
            let addr = Ipv4Addr::from(((*addr).sin_addr.s_addr as u32).to_be());

            STATE.lock().unwrap().add_node(ssocket, SocketAddrV4::new(addr, port));
        } else {
            panic!("We're only supporting the IPv4 address space");
        }

        0
    }
}

hook! {
    unsafe fn connect(ssocket: c_int, address: *const sockaddr, _address_len: socklen_t) -> c_int => fake_connect {
        print!("C");
        if (*address).sa_family == AF_INET as u16 {
            let addr = address as *const sockaddr_in;

            // convert host and port to SocketAddrV4
            let port = (*addr).sin_port.to_be();
            let addr = Ipv4Addr::from(((*addr).sin_addr.s_addr as u32).to_be());

            STATE.lock().unwrap().connect_to_node(ssocket, SocketAddrV4::new(addr, port));

            if !PENDING.load(Ordering::Relaxed) {
                // wake up the epoll_wait thread
                SYNC.1.notify_one();
            }
        } else {
            panic!("We're only supporting the IPv4 address space");
        }

        0
    }
}

hook! {
    unsafe fn accept4(ssocket: c_int, address: *mut sockaddr, address_len: *mut socklen_t, flg: c_int) -> c_int => fake_accept {
        //real!(accept4)(ssocket, address, address_len, flg);
        //

        let mut ret_fd = 0;
        if let Some(fd) = STATE.lock().unwrap().accept(ssocket) {
            println!("accept");
            (*address_len) = 16;
            let addr = state::empty_addr();

            ptr::write(address as *mut sockaddr_in, addr);
            //
            //println!("{:?}", STATE.lock().unwrap().get_epoll());

            ret_fd = fd;
        } else {
            println!("invalid accept");

            set_errno(Errno(EAGAIN));

            ret_fd = -1
        }

        if ret_fd >= 0 {
            PENDING.store(true, Ordering::Relaxed);
        } else {
            PENDING.store(false, Ordering::Relaxed);
        }

        SYNC.1.notify_one();

        ret_fd
    }
}


hook! {
    unsafe fn getsockname(fd: c_int, address: *mut sockaddr, address_len: *mut socklen_t) -> c_int => fake_getsockname {
        let addr = STATE.lock().unwrap().get_sockname(fd);

        *address_len = 16;
        let addr = state::to_sockaddr(addr);
        ptr::write(address as *mut sockaddr_in, addr);

        0
    }
}

hook! {
    unsafe fn getpeername(_fd: c_int, address: *mut sockaddr, address_len: *mut socklen_t) -> c_int => fake_getpeername {
        *address_len = 16;
        let addr = state::empty_addr();

        ptr::write(address as *mut sockaddr_in, addr);
        
        0
    }
}
fn events_to_string(events: c_uint) -> String {
    [
        ("EPOLLIN", EPOLLIN), 
        ("EPOLLOUT", EPOLLOUT),
        ("EPOLLRDHUP", EPOLLRDHUP),
        ("EPOLLPRI", EPOLLPRI),
        ("EPOLLERR", EPOLLERR),
        ("EPOLLHUP", EPOLLHUP),
        ("EPOLLET", EPOLLET),
        ("EPOLLONESHOT", EPOLLONESHOT),
        ("EPOLlWAKEUP", EPOLLWAKEUP),
        ("EPOLLEXCLUSIVE", EPOLLEXCLUSIVE)
    ].iter().filter_map(|(a,b)| {
        if (events & (*b) as u32) != 0 {
            return Some((*a).into());
        } else {
            return None;
        }
    }).collect::<Vec<String>>().join(", ")
}

hook! {
    unsafe fn epoll_ctl(epfd: c_int, op: c_int, fd: c_int, event: *mut epoll_event) -> c_int => fake_epoll_ctl {
        print!("E");
        if op == EPOLL_CTL_ADD {
            let events = (*event).events;

            //println!("Hook: register {} with id {}", fd, (*event).u64);

            // wake up the epoll_wait thread
            SYNC.1.notify_one();

            STATE.lock().unwrap().add_epoll_fd(fd, events, (*event).u64);
        }

        0
    }
}

hook! {
    unsafe fn epoll_wait(_epfd: c_int, events: *mut epoll_event, _maxevents: c_int, _timeout: c_int) -> c_int => fake_epoll_wait {
        let mut started = SYNC.0.lock().unwrap();
        loop {
            /*if let Some((fd_id, fd_events)) = STATE.lock().unwrap().next_epoll_notify() {
                //println!(" ===> Notify id {}", fd_id);
                let answ = epoll_event { 
                    events: fd_events as u32, 
                    u64: fd_id
                };

                ptr::write(events, answ);

                return 1;
            }*/

            loop {
                loop {
                    let lock = STATE.try_lock();
                    
                    let mut val = match lock {
                        Ok(val) => val,
                        Err(_) => {
                            println!("Would block");
                            continue
                        }
                    };

                    if let Some((fd_id, fd_events)) = val.next_epoll_notify() {
                        println!(" ===> Notify2 id {}", fd_id);

                        let answ = epoll_event { 
                            events: fd_events as u32, 
                            u64: fd_id
                        };

                        ptr::write(events, answ);

                        return 1;
                    }

                    break;
                }

                if PENDING.load(Ordering::Relaxed) {
                    started = SYNC.1.wait(started).unwrap();
                } else {
                    break;
                }
            }

            let next_id = STATE.lock().unwrap().next_epoll_id();
            if let Some((fd_id, fd_events)) = next_id {
                PENDING.store(true, Ordering::Relaxed);

                let answ = epoll_event { 
                    events: fd_events as u32, 
                    u64: fd_id
                };

                ptr::write(events, answ);

                return 1;
            }

        }
    }
}

hook! {
    unsafe fn creat(path: *const u8, mode: mode_t) -> c_int => fake_creat {
        let fd = real!(creat)(path, mode);

        println!("Hook: creat - ask for logs");

        //let buf = STATE.lock().unwrap().get_logs();
        //real!(write)(fd, buf.as_ptr() as *const c_void, buf.len());

        fd
    }
}
