extern crate libc;

use std::io;
use std::os::unix::io::RawFd;

fn epoll_create() -> io::Result<RawFd> {
    let epfd = unsafe { libc::epoll_create(1) };
    if epfd < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(epfd)
    }
}

fn epoll_ctl(epfd: RawFd, op: i32, fd: RawFd, events: u32) -> io::Result<()> {
    let mut event = libc::epoll_event{ events, u64: fd as u64 };
    let code = unsafe { libc::epoll_ctl(epfd, op, fd, &mut event as *mut libc::epoll_event) };
    if code < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

fn epoll_wait(epfd: RawFd, timeout: i32, buf: &mut [libc::epoll_event]) -> io::Result<usize> {
    let num_events = unsafe { libc::epoll_wait(epfd, buf.as_mut_ptr(), buf.len() as i32, timeout) };
    if num_events < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(num_events as usize)
    }
}

pub struct Poller{
    epfd: RawFd,
    buffer: Vec<libc::epoll_event>,
    index: usize,
    length: usize,
}

impl Poller {
    pub fn new(bufsize: usize) -> io::Result<Self> {
        let epfd = epoll_create()?;
        let mut buffer = Vec::with_capacity(bufsize);

        for _ in 0..bufsize {
            buffer.push( libc::epoll_event{events: 0, u64: 0} );
        }

        Ok(Poller {
            epfd,
            buffer,
            index: 0,
            length: 0,
        })
    }

    pub fn add_fd(&mut self, fd: RawFd) -> io::Result<()> {
        epoll_ctl(self.epfd, libc::EPOLL_CTL_ADD, fd, libc::EPOLLIN as u32)
    }

    pub fn del_fd(&mut self, fd: RawFd) -> io::Result<()> {
        epoll_ctl(self.epfd, libc::EPOLL_CTL_DEL, fd, libc::EPOLLIN as u32)
    }

    pub fn next(&mut self, timeout: i32) -> io::Result<Option<RawFd>> {
        self.index += 1;
        if self.index >= self.length {
            self.length = epoll_wait(self.epfd, timeout, &mut self.buffer)?;
            self.index = 0;
        }

        if self.length != 0 {
            Ok(Some(self.buffer[self.index].u64 as RawFd))
        } else {
            Ok(None)
        }
    }
}
