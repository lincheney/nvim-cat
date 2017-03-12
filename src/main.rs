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

fn dump_file(
        filename: &str,
        nvim: &mut nvim::Nvim,
        poller: &mut epoll::Poller,
        stdout_fd: RawFd,
        filetype: Option<&str>,
        ) -> Result<(), nvim::NvimError> {

    let file = if filename == "-" { "/dev/stdin" } else { filename };
    // println!("{}", file);

    match filetype {
        Some(filetype) => {
            nvim.nvim_command(&format!("set ft={}", filetype))?;
        },
        None => {
            nvim.nvim_command(&format!("set ft= | doautocmd BufRead {}", file))?;
        }
    }

    let file = File::open(file)?;
    let stdin_fd = file.as_raw_fd();
    let file = BufReader::new(&file);

    let timeout = match poller.add_fd(stdin_fd) {
        Ok(_) => -1,
        // EPERM: cannot epoll this file
        Err(ref e) if e.kind() == ErrorKind::PermissionDenied => 0,
        Err(e) => return Err(nvim::NvimError::IOError(e)),
    };

    let mut lines = file.lines();
    let mut lineno = 2;

    loop {
        let read_line = match poller.next(timeout)? {
            Some(fd) if fd == stdout_fd => {
                nvim.process_event()?;
                false
            },
            Some(fd) if fd == stdin_fd => true,
            None if timeout == 0 => true,
            Some(_) => unreachable!(),
            None => unreachable!(),
        };

        if read_line {
            if let Some(line) = lines.next() {
                let line = line?;
                nvim.add_line(line, lineno)?;
                lineno += 1;
            } else {
                if timeout == -1 { poller.del_fd(stdin_fd)?; }
                break;
            }
        }
    }

    while nvim.lineno < lineno {
        nvim.process_event()?;
    }

    nvim.reset()?;
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

    let mut poller = epoll::Poller::new(2);

    let process = nvim::Nvim::start_process();
    let stdout = process.stdout.unwrap();
    let stdout_fd = stdout.as_raw_fd();
    let mut stdin = process.stdin.unwrap();

    let mut nvim = nvim::Nvim::new(&mut stdin, stdout);
    poller.add_fd(stdout_fd).unwrap();

    for &file in files.iter() {
        match dump_file(file, &mut nvim, &mut poller, stdout_fd, filetype) {
            Err(nvim::NvimError::IOError(ref e)) if e.kind() == ErrorKind::BrokenPipe => break,
            Err(nvim::NvimError::IOError(e)) => {
                // get friendly error message
                let e = nix::errno::Errno::from_i32(e.raw_os_error().unwrap());
                print_error!("{}: {}", file, e.desc());
                // try to continue on ioerrors
            },
            Err(e) => {
                print_error!("{}: {:?}", file, e);
                return Ok(false);
            },
            _ => (),
        }
    }

    nvim.quit()?;
    Ok(true)
}

fn main() {
    match entrypoint() {
        Ok(_) => (),
        Err(e) => { print_error!("{}", e); },
    }
}
