#[macro_use]
extern crate lazy_static;
#[macro_use]
extern crate quick_error;

extern crate libc;
extern crate clap;

use std::fs::File;
use std::io::{stdout, stderr, Write, BufReader, BufRead, ErrorKind};
use clap::{Arg, App};

macro_rules! print_error(
    ($fmt:expr) => ({
        writeln!(stderr(), concat!("ERROR: ", $fmt)).ok()
    });
    ($fmt:expr, $($arg:tt)*) => ({
        writeln!(stderr(), concat!("ERROR: ", $fmt), $($arg)*).ok()
    })
);

mod nvim;
mod synattr;

fn dump_file(
        filename: &str,
        nvim: &mut nvim::Nvim,
        filetype: Option<&str>,
        ) -> Result<(), nvim::NvimError> {

    let file = if filename == "-" { "/dev/stdin" } else { filename };
    println!("{}", file);

    match filetype {
        Some(filetype) => {
            nvim.nvim_command(&format!("set ft={}", filetype))?;
        },
        None => {
            nvim.nvim_command(&format!("set ft= | doautocmd BufRead {}", file))?;
        }
    }

    let file = File::open(file)?;
    let file = BufReader::new(&file);
    let mut lineno = 2;

    for line in file.lines() {
        let line = line?;
        nvim.add_line(&line)?;
        let line = nvim.get_line(&line, lineno)?;
        stdout().write(line.as_bytes())?;
        stdout().write(b"\x1b[0m\n")?;
        lineno += 1;
    }

    nvim.reset()?;
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

    let process = nvim::Nvim::start_process();
    let stdout = process.stdout.unwrap();
    let mut stdin = process.stdin.unwrap();

    let mut nvim = nvim::Nvim::new(&mut stdin, stdout);

    for &file in files.iter() {
        match dump_file(file, &mut nvim, filetype) {
            Ok(_) => (),
            Err(nvim::NvimError::IOError(ref e)) if e.kind() == ErrorKind::BrokenPipe => break,
            Err(e) => { print_error!("{}: {}", file, e); },
        }
    }
    nvim.quit().unwrap();
}
