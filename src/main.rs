#[macro_use]
extern crate lazy_static;
#[macro_use]
extern crate quick_error;

extern crate libc;
extern crate clap;

use std::fs::File;
use std::io::{stderr, Write, BufReader, BufRead, ErrorKind};
use std::os::unix::io::{AsRawFd, RawFd};
use std::sync::mpsc::Sender;
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

use nvim::{Nvim, Handle};

fn dump_file(
        filename: &str,
        nvim: &Handle,
        filetype: Option<&str>,
        ) -> Result<(), nvim::NvimError> {

    let file = if filename == "-" { "/dev/stdin" } else { filename };
    // println!("{}", file);

    match filetype {
        Some(filetype) => {
            nvim.nvim_command(format!("set ft={}", filetype));
        },
        None => {
            nvim.nvim_command(format!("set ft= | doautocmd BufRead {}", file));
        }
    }

    let file = File::open(file)?;
    let file = BufReader::new(&file);

    for (i, line) in file.lines().enumerate() {
        let line = line?;
        nvim.add_line(line, i+2);
    }

    nvim.reset();
    Ok(())
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

    let nvim = Nvim::start_thread();

    for &file in files.iter() {
        match dump_file(file, &nvim, filetype) {
            Ok(_) => (),
            Err(nvim::NvimError::IOError(ref e)) if e.kind() == ErrorKind::BrokenPipe => break,
            Err(e) => { print_error!("{}: {}", file, e); },
        }
    }

    nvim.quit();
}
