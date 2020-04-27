#[macro_use]
extern crate lazy_static;
#[macro_use]
extern crate quick_error;

extern crate nix;
extern crate libc;
extern crate clap;

use std::fs::File;
use std::io::ErrorKind;
use std::os::unix::io::AsRawFd;
use clap::{Arg, App};

macro_rules! print_error(
    ($fmt:expr) => ({
        use ::std::io::Write;
        writeln!(::std::io::stderr(), concat!("ERROR: ", $fmt)).ok()
    });
    ($fmt:expr, $($arg:tt)*) => ({
        use ::std::io::Write;
        writeln!(::std::io::stderr(), concat!("ERROR: ", $fmt), $($arg)*).ok()
    })
);

mod rpc;
mod nvim;
mod epoll;
mod synattr;
mod poller;

fn dump_file(
        filename: &str,
        nvim: &mut nvim::Nvim,
        poller: &mut poller::Poller,
        filetype: Option<&str>,
        ) -> nvim::NvimResult<()> {

    let file = if filename == "-" { "/dev/stdin" } else { filename };
    // println!("{}", file);

    if let Some(filetype) = filetype {
        nvim.nvim_command(&format!("set ft={}", filetype))?;
    } else {
        nvim.nvim_command(&format!("set ft= | doautocmd BufRead {}", file))?;
    }
    nvim.press_enter()?; // press enter now and then to get past blocking error messages

    let file = File::open(file)?;
    poller.add_stdin(file.as_raw_fd())?;
    let mut file = poller::NBBufReader::new(file);

    let mut lineno = 0;
    loop {
        match poller.next()? {
            poller::PollResult::Stdout => {
                nvim.process_event()?;
            },
            poller::PollResult::Stdin => {
                nvim.press_enter()?; // press enter now and then to get past blocking error messages
                match file.read_lines()? {
                    Some(lines) => {
                        for line in lines {
                            nvim.add_line(line, lineno)?;
                            // run filetype detect over first 10 lines
                            if filetype.is_none() && lineno < 10 {
                                nvim.filetype_detect()?;
                            }
                            lineno += 1;
                        }
                    },
                    None => {
                        poller.rm_stdin()?;
                        break;
                    }
                }
            },
        }
    }

    while nvim.lineno < lineno {
        nvim.process_event()?;
    }
    Ok(())
}

fn entrypoint() -> nvim::NvimResult<bool> {
    let matches = App::new("nvim-cat")
        .about("TODO")
        .arg(Arg::with_name("vimrc")
             .short("u")
             .value_name("vimrc")
             .help("Use <vimrc> instead of the default")
             .takes_value(true))
        .arg(Arg::with_name("filetype")
             .short("f")
             .long("ft")
             .value_name("ft")
             .help("Set the filetype to <ft>")
             .takes_value(true))
        .arg(Arg::with_name("numbered")
             .short("n")
             .long("number")
             .help("Number output lines"))
        .arg(Arg::with_name("restricted_mode")
             .short("Z")
             .help("Restricted mode"))
        .arg(Arg::with_name("colorscheme")
            .value_name("colorscheme")
            .short("s")
            .help("Colorscheme"))
        .arg(Arg::with_name("FILE")
             .multiple(true))
        .get_matches();

    let filetype = matches.value_of("filetype");
    let vimrc = matches.value_of("vimrc");
    let colorscheme = matches.value_of("colorscheme");
    let files: Vec<&str> = match matches.values_of("FILE") {
        Some(values) => values.collect(),
        None => vec!["-"],
    };

    let options = nvim::NvimOptions{
        numbered: matches.is_present("numbered"),
        restricted_mode: matches.is_present("restricted_mode"),
    };

    let process = nvim::Nvim::start_process(vimrc, colorscheme, options);
    let stdout = process.stdout.unwrap();
    let stdin = process.stdin.unwrap();

    let mut poller = poller::Poller::new(stdout.as_raw_fd())?;
    let mut nvim = nvim::Nvim::new(stdin, stdout, options)?;

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

        if i < files.len()-1 { nvim.reset()?; }
    }

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
