extern crate libc;
extern crate clap;

use std::fs::File;
use std::io::{Read, BufRead};
use std::os::unix::io::{AsRawFd, RawFd};
use clap::{Arg, App};

mod highlight;
mod nvim;
mod epoll;

fn dump_file(file: &str, poller: &mut epoll::Poller, nvim: &mut nvim::Nvim, stdout_fd: RawFd) {
    let file = if file == "-" { "/dev/stdin" } else { file };
    println!("{}", file);
    let file = File::open(file).unwrap();
    let stdin_fd = file.as_raw_fd();

    let mut regular_file = match poller.add_fd(stdin_fd) {
        Ok(_) => false,
        // EPERM: cannot epoll this file
        Err(ref e) if e.kind() == std::io::ErrorKind::PermissionDenied => true,
        Err(e) => { panic!(e.to_string()); },
    };

    let mut reader = std::io::BufReader::new(file);
    let mut buf = String::new();

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
            buf.clear();
            if reader.read_line(&mut buf).unwrap() > 0 {
                nvim.add_lines(&[&buf.trim_right_matches("\n"), ""], 1).unwrap();
            } else {
                if ! regular_file {
                    poller.del_fd(stdin_fd).unwrap();
                }
                nvim.set_eof();
                // trigger dummy final input
                nvim.add_lines(&["a"], 0).unwrap();
                regular_file = false;
            }
        }
    }
}

fn main() {
    let matches = App::new("nvim-cat")
        .about("TODO")
        .arg(Arg::with_name("u")
             .short("u")
             .value_name("<vimrc>")
             .help("Use <vimrc> instead of the default")
             .takes_value(true))
        .arg(Arg::with_name("FILE")
             .multiple(true))
        .get_matches();

    let files = match matches.values_of("FILE") {
        Some(values) => {
            values.collect::<Vec<&str>>()
        },
        None => vec!["-"],
    };

    let mut poller = epoll::Poller::new(2);

    let process = nvim::Nvim::start_process("src/main.rs");
    let stdout = process.stdout.unwrap();
    let stdout_fd = stdout.as_raw_fd();
    let mut stdin = process.stdin.unwrap();
    let mut nvim = nvim::Nvim::new(&mut stdin, stdout);
    poller.add_fd(stdout_fd).unwrap();
    nvim.attach().unwrap();

    for &file in files.iter() {
        dump_file(file, &mut poller, &mut nvim, stdout_fd);
    }
}
