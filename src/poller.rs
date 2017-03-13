use epoll;
use nvim;
use std;
use std::os::unix::io::RawFd;
use std::io::{Read, ErrorKind, BufReader, BufRead};

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

struct NBFile<R> {
    inner: R,
    fake_eof: bool,
}

impl<R> Read for NBFile<R> where R: Read {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.fake_eof {
            return Ok(0)
        }
        self.fake_eof = true;
        self.inner.read(buf)
    }
}

pub struct NBBufReader<R> {
    inner: BufReader<NBFile<R>>,
    buf: Vec<u8>,
    leftover: Option<String>,
}

impl<R> NBBufReader<R> where R: Read {
    pub fn new(file: R) -> Self {
        let file = NBFile{ inner: file, fake_eof: false };
        let reader = std::io::BufReader::new(file);

        NBBufReader{ inner: reader, buf: vec![], leftover: None }
    }

    pub fn read_lines(&mut self) -> std::io::Result<Option<Vec<String>>> {
        let mut lines: Vec<String> = vec![];
        let mut eof = true;
        loop {
            self.buf.clear();
            let len = self.inner.read_until(b'\n', &mut self.buf)?;
            if len == 0 { break; }

            eof = false;
            let has_newline = self.buf[len - 1] == b'\n';
            if has_newline {
                self.buf.pop();
                if len > 1 && self.buf[len - 2] == b'\r' {
                    self.buf.pop();
                }
            }

            let string = std::str::from_utf8(&self.buf).unwrap();
            let string = if let Some(mut leftover) = self.leftover.take() {
                leftover.push_str(string);
                leftover
            } else {
                string.to_string()
            };

            if has_newline {
                lines.push(string);
            } else {
                self.leftover = Some(string);
            }
        }

        self.inner.get_mut().fake_eof = eof;
        if eof {
            if let Some(leftover) = self.leftover.take() {
                lines.push(leftover);
            }
        }

        if lines.is_empty() { return Ok(None) }
        Ok(Some(lines))
    }
}
