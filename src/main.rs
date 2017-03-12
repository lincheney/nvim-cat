#[macro_use]
extern crate lazy_static;
#[macro_use]
extern crate quick_error;

extern crate nix;
extern crate libc;
extern crate clap;

use std::fs::File;
use std::io::{stderr, Write, BufReader, BufRead, ErrorKind};
use std::os::unix::io::{AsRawFd, RawFd};
use clap::{Arg, App};

macro_rules! print_error(
    ($fmt:expr) => ({
        writeln!(stderr(), concat!("ERROR: ", $fmt)).ok()
    });
    ($fmt:expr, $($arg:tt)*) => ({
        writeln!(stderr(), concat!("ERROR: ", $fmt), $($arg)*).ok()
    })
);

mod rpc;
mod nvim;
mod epoll;
mod synattr;

struct Poller {
    poller: epoll::Poller,
    timeout: i32,
    stdout_fd: RawFd,
    stdin_fd: Option<RawFd>,
}

enum PollResult { Stdout, Stdin }

impl Poller {
    pub fn new(stdout_fd: RawFd) -> Result<Self, nvim::NvimError> {
        let mut poller = epoll::Poller::new(2)?;
        poller.add_fd(stdout_fd)?;

        Ok(Poller {
            poller: poller,
            timeout: -1,
            stdout_fd: stdout_fd,
            stdin_fd: None,
        })
    }

    pub fn add_stdin(&mut self, stdin_fd: RawFd) -> Result<(), nvim::NvimError> {
        self.timeout = match self.poller.add_fd(stdin_fd) {
            Ok(_) => {
                self.stdin_fd = Some(stdin_fd);
                -1
            },
            // EPERM: cannot epoll this file
            Err(ref e) if e.kind() == ErrorKind::PermissionDenied => 0,
            Err(e) => return Err(nvim::NvimError::IOError(e)),
        };
        self.stdin_fd = None;
        Ok(())
    }

    pub fn rm_stdin(&mut self) -> Result<(), nvim::NvimError> {
        if let Some(fd) = self.stdin_fd.take() {
            self.poller.del_fd(fd)?;
        }
        Ok(())
    }

    pub fn next(&mut self) -> Result<PollResult, nvim::NvimError> {
        let result = match self.poller.next(self.timeout)? {
            fd if fd == self.stdin_fd => PollResult::Stdin,
            Some(fd) if fd == self.stdout_fd => PollResult::Stdout,
            Some(_) => unreachable!(),
            None => unreachable!(),
        };
        Ok(result)
    }
}

fn dump_file(
        filename: &str,
        nvim: &mut nvim::Nvim,
        poller: &mut Poller,
        filetype: Option<&str>,
        ) -> Result<(), nvim::NvimError> {

    let file = if filename == "-" { "/dev/stdin" } else { filename };
    // println!("{}", file);

    if let Some(filetype) = filetype {
        nvim.nvim_command(&format!("set ft={}", filetype))?;
    } else {
        nvim.nvim_command(&format!("set ft= | doautocmd BufRead {}", file))?;
    }

    let file = File::open(file)?;
    poller.add_stdin(file.as_raw_fd())?;
    let file = BufReader::new(&file);

    let mut lines = file.lines();
    let mut lineno = 2;

    loop {
        match poller.next()? {
            PollResult::Stdout => {
                nvim.process_event()?;
            },
            PollResult::Stdin => {
                if let Some(line) = lines.next() {
                    let line = line?;
                    nvim.add_line(line, lineno)?;
                    lineno += 1;
                } else {
                    poller.rm_stdin()?;
                    break;
                }
            },
        }
    }

    while nvim.lineno < lineno {
        nvim.process_event()?;
    }
    Ok(())
}

fn entrypoint() -> Result<bool, nvim::NvimError> {
    let matches = App::new("nvim-cat")
        .about("TODO")
        .arg(Arg::with_name("u")
             .short("u")
             .value_name("vimrc")
             .help("Use <vimrc> instead of the default")
             .takes_value(true))
        .arg(Arg::with_name("ft")
             .short("f")
             .long("-ft")
             .value_name("ft")
             .help("Set the filetype to <ft>")
             .takes_value(true))
        .arg(Arg::with_name("FILE")
             .multiple(true))
        .get_matches();

    let filetype = matches.value_of("ft");
    let files = match matches.values_of("FILE") {
        Some(values) => {
            values.collect::<Vec<&str>>()
        },
        None => vec!["-"],
    };

    let process = nvim::Nvim::start_process();
    let stdout = process.stdout.unwrap();
    let mut stdin = process.stdin.unwrap();

    let mut poller = Poller::new(stdout.as_raw_fd())?;
    let mut nvim = nvim::Nvim::new(&mut stdin, stdout);

    let mut success = true;
    for (i, &file) in files.iter().enumerate() {
        match dump_file(file, &mut nvim, &mut poller, filetype) {
            Err(nvim::NvimError::IOError(ref e)) if e.kind() == ErrorKind::BrokenPipe => break,
            Err(nvim::NvimError::IOError(e)) => {
                // get friendly error message
                let e = nix::errno::Errno::from_i32(e.raw_os_error().unwrap());
                print_error!("{}: {}", file, e.desc());
                success = false;
                // try to continue on ioerrors
            },
            Err(e) => {
                print_error!("{}: {:?}", file, e);
                success = false;
                break;
            },
            _ => (),
        }

        if i != files.len() { nvim.reset()?; }
    }

    nvim.quit()?;
    Ok(success)
}

fn main() {
    let exit_code = match entrypoint() {
        Ok(true) => 0,
        Ok(false) => 1,
        Err(e) => { print_error!("{}", e); 1 },
    };
    std::process::exit(exit_code);
}
