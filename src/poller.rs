use epoll;
use nvim;
use std::os::unix::io::RawFd;
use std::io::ErrorKind;

pub struct Poller {
    poller: epoll::Poller,
    timeout: i32,
    stdout_fd: RawFd,
    stdin_fd: Option<RawFd>,
}

pub enum PollResult { Stdout, Stdin }

impl Poller {
    pub fn new(stdout_fd: RawFd) -> nvim::NvimResult<Self> {
        let mut poller = epoll::Poller::new(2)?;
        poller.add_fd(stdout_fd)?;

        Ok(Poller {
            poller: poller,
            timeout: -1,
            stdout_fd: stdout_fd,
            stdin_fd: None,
        })
    }

    pub fn add_stdin(&mut self, stdin_fd: RawFd) -> nvim::NvimResult<()> {
        self.stdin_fd.take();
        self.timeout = match self.poller.add_fd(stdin_fd) {
            Ok(_) => {
                self.stdin_fd = Some(stdin_fd);
                -1
            },
            // EPERM: cannot epoll this file
            Err(ref e) if e.kind() == ErrorKind::PermissionDenied => 0,
            Err(e) => return Err(nvim::NvimError::IOError(e)),
        };
        Ok(())
    }

    pub fn rm_stdin(&mut self) -> nvim::NvimResult<()> {
        if let Some(fd) = self.stdin_fd.take() {
            self.poller.del_fd(fd)?;
        }
        Ok(())
    }

    pub fn next(&mut self) -> nvim::NvimResult<PollResult> {
        let result = match self.poller.next(self.timeout)? {
            fd if fd == self.stdin_fd => PollResult::Stdin,
            Some(fd) if fd == self.stdout_fd => PollResult::Stdout,
            Some(_) => unreachable!(),
            None => unreachable!(),
        };
        Ok(result)
    }
}
