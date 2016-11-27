extern crate libc;
extern crate clap;
extern crate term_size;

use std::fs::File;
use std::io::{stderr, Write, Read, BufRead};
use std::os::unix::io::{AsRawFd, RawFd};
use clap::{Arg, App};

mod highlight;
mod nvim;
mod epoll;

macro_rules! print_error(
    ($fmt:expr) => ({
        writeln!(&mut stderr(), concat!("error: ", $fmt)).ok()
    });
    ($fmt:expr, $($arg:tt)*) => ({
        writeln!(&mut stderr(), concat!("error: ", $fmt), $($arg)*).ok()
    })
);

struct HaltingFile<R> where R: Read {
    pub fake_eof: bool,
    file: R,
}

impl<R> Read for HaltingFile<R> where R: Read {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.fake_eof {
            return Ok(0)
        }
        self.fake_eof = true;
        self.file.read(buf)
    }
}

fn dump_file(
        filename: &str,
        poller: &mut epoll::Poller,
        nvim: &mut nvim::Nvim,
        stdout_fd: RawFd,
        filetype: Option<&str>,
        ) {

    let file = if filename == "-" { "/dev/stdin" } else { filename };
    println!("{}", file);

    match filetype {
        Some(filetype) => {
            nvim.nvim_command(&format!("set ft={}", filetype)).unwrap();
        },
        None => {
            nvim.nvim_command(&format!("set ft= | doautocmd BufRead {}", file)).unwrap();
        }
    }

    let file = match File::open(file) {
        Ok(file) => file,
        Err(e) => {
            print_error!("{}: {}", filename, e);
            return;
        }
    };
    let stdin_fd = file.as_raw_fd();

    let mut regular_file = match poller.add_fd(stdin_fd) {
        Ok(_) => false,
        // EPERM: cannot epoll this file
        Err(ref e) if e.kind() == std::io::ErrorKind::PermissionDenied => true,
        Err(e) => {
            print_error!("{}: {}", filename, e);
            return;
        },
    };

    let file = HaltingFile{file: file, fake_eof: false};
    let mut reader = std::io::BufReader::with_capacity(10, file);
    let mut lines = Vec::<String>::new();
    let mut leftover: Option<String> = None;

    loop {
        let has_stdin = match poller.next(if regular_file {0} else {-1}) {
            Some(fd) if fd == stdout_fd => {
                if nvim.process_event().unwrap() {
                    break;
                }
                false
            },

            Some(fd) if fd == stdin_fd => true,
            Some(_) => unreachable!(),
            None => true,
        };

        if has_stdin {
            lines.clear();
            loop {
                let mut buf = Vec::<u8>::new();
                match reader.read_until(b'\n', &mut buf) {
                    Ok(0) => break,
                    Ok(len) => {
                        let has_newline = buf[len - 1] == b'\n';

                        if has_newline {
                            buf.pop();
                            if len > 1 && buf[len - 2] == b'\r' {
                                buf.pop();
                            }
                        }

                        let string = unsafe{ std::str::from_utf8_unchecked(&buf) };
                        let string = if let Some(leftover_str) = leftover {
                            leftover = None;
                            leftover_str + string
                        } else {
                            string.to_string()
                        };

                        if has_newline {
                            lines.push(string);
                        } else {
                            leftover = Some(string);
                        }
                    }
                    Err(e) => {
                        print_error!("{}: {}", filename, e);
                        return;
                    }
                }
            }
            reader.get_mut().fake_eof = false;

            if ! lines.is_empty() {
                nvim.add_lines(&lines[..]).unwrap();
            } else if leftover.is_none() {
                if ! regular_file {
                    poller.del_fd(stdin_fd).unwrap();
                }
                if nvim.set_eof() {
                    break;
                }
                regular_file = false;
            }
        }
    }

    nvim.reset();
}

fn main() {
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

    let width = term_size::dimensions().map_or(100, |(w, _)| w);

    let mut poller = epoll::Poller::new(2);

    let process = nvim::Nvim::start_process();
    let stdout = process.stdout.unwrap();
    let stdout_fd = stdout.as_raw_fd();
    let mut stdin = process.stdin.unwrap();

    let mut nvim = nvim::Nvim::new(&mut stdin, stdout);
    poller.add_fd(stdout_fd).unwrap();
    nvim.attach(width).unwrap();

    for &file in files.iter() {
        dump_file(file, &mut poller, &mut nvim, stdout_fd, filetype);
    }
    nvim.quit().unwrap();
}
