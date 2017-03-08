#[macro_use]
extern crate lazy_static;

extern crate libc;
extern crate clap;

use std::fs::File;
use std::io::{stdout, stderr, Write, BufReader, BufRead, ErrorKind};
use clap::{Arg, App};

mod nvim;

macro_rules! print_error(
    ($fmt:expr) => ({
        writeln!(&mut stderr(), concat!("error: ", $fmt)).ok()
    });
    ($fmt:expr, $($arg:tt)*) => ({
        writeln!(&mut stderr(), concat!("error: ", $fmt), $($arg)*).ok()
    })
);

fn dump_file(
        filename: &str,
        nvim: &mut nvim::Nvim,
        filetype: Option<&str>,
        ) -> Result<(), std::io::Error> {

    let file = if filename == "-" { "/dev/stdin" } else { filename };
    println!("{}", file);

    match filetype {
        Some(filetype) => {
            nvim.nvim_command(50, &format!("set ft={}", filetype)).unwrap();
            nvim.wait_for_response(50).unwrap();
        },
        None => {
            nvim.nvim_command(51, &format!("set ft= | doautocmd BufRead {}", file)).unwrap();
        }
    }

    let file = File::open(file)?;
    let file = BufReader::new(&file);
    for line in file.lines() {
        let line = line.unwrap();
        let line = nvim.add_line(&line).unwrap();
        stdout().write(line.as_bytes())?;
        stdout().write(b"\n")?;
    }

    // nvim.reset();
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
        if let Err(e) = dump_file(file, &mut nvim, filetype) {
            match e.kind() {
                ErrorKind::BrokenPipe => break,
                _ => { print_error!("{}: {}", file, e); },
            }
        }
    }
    nvim.quit().unwrap();
}
