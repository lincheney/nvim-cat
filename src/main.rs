extern crate rmp;
extern crate rmp_serde;
extern crate serde;

use std::io::{stdout, stderr, Write, Error, ErrorKind};
use std::cell::RefCell;
use std::process::{Command, Stdio, ChildStdout, ChildStdin};
use std::collections::BTreeMap;

use rmp_serde::{Serializer, Deserializer};
use serde::{Serialize, Deserialize};

const HEIGHT : usize = 100;
const WIDTH : usize = 100;
const RESET : &'static str = "\x1b[0m";

enum Attr {
    BOLD = 1,
    ITALIC = 2,
    UNDERLINE = 4,
    REVERSE = 8,
}

#[derive(Debug, Clone)]
struct Highlight {
    fg: String,
    bg: String,
    attrs: u8,
}

impl Highlight {
    pub fn new() -> Self {
        Highlight{ fg: "255;255;255".to_string(), bg: "0;0;0".to_string(), attrs: 0 }
    }

    pub fn rgb_to_string(val: u32) -> String {
        format!("{};{};{}",
            val >> 16,
            (val & 0x00ff00) >> 8,
            val & 0xff,
        )
    }

    pub fn to_string(&self) -> String {
        format!("\x1b[0;{bold}{italic}{underline}38;2;{fg};48;2;{bg}m",
            bold = if self.attrs & (Attr::BOLD as u8) != 0 { "1;" } else { "" },
            italic = if self.attrs & (Attr::ITALIC as u8) != 0 { "3;" } else { "" },
            underline = if self.attrs & (Attr::UNDERLINE as u8) != 0 { "4;" } else { "" },
            fg = if self.attrs & (Attr::REVERSE as u8) == 0 { &self.fg } else { &self.bg },
            bg = if self.attrs & (Attr::REVERSE as u8) == 0 { &self.bg } else { &self.fg },
        )
    }
}

struct Printer<'a> {
    deserializer:   Deserializer<ChildStdout>,
    serializer:     RefCell<Serializer<'a, rmp_serde::encode::StructArrayWriter> >,
    cursor:         [usize; 2],
    eof:            bool,
    modeline:       bool,
    offset:         usize,
    lineno:         usize,
    hl:             Highlight,
    default_hl:     Highlight,
}

impl<'a> Printer<'a> {
    pub fn new(stdin: &'a mut ChildStdin, stdout: ChildStdout) -> Self {
        let serializer = Serializer::new(stdin);
        let deserializer = Deserializer::new(stdout);
        Printer {
            deserializer: deserializer,
            serializer: RefCell::new(serializer),
            cursor: [0, 0],
            eof: false,
            modeline: false,
            offset: 0,
            lineno: 0,
            hl: Highlight::new(),
            default_hl: Highlight::new(),
        }
    }

    pub fn nvim_command(&self, command: &str) {
        let value = ( 0, 300, "nvim_command", (command,) );
        value.serialize(&mut *self.serializer.borrow_mut()).unwrap();
    }

    pub fn attach(&self) {
        let mut kwargs = BTreeMap::new();
        kwargs.insert("rgb", true);
        let value = ( 0, 100, "nvim_ui_attach", (WIDTH, HEIGHT, kwargs) );
        value.serialize(&mut *self.serializer.borrow_mut()).unwrap();
    }

    pub fn quit(&self) {
        self.nvim_command("qa!");
    }

    fn scroll(&self, line: usize) {
        self.nvim_command(format!("normal {}z\n", line).as_str());
    }

    fn handle_put(&mut self, args: &[rmp::Value]) -> Result<(), Error> {
        if self.eof || self.modeline {
            return Ok(());
        }

        let eofstr = format!("~{1:0$}", WIDTH - 1, "");

        let parts : Vec<_> = args
            .iter()
            .flat_map(|x| x.as_array().unwrap())
            .map(|x| x.as_str().unwrap())
            .collect()
            ;
        let string = parts.join("");
        // println!("{:?} {}", string, self.offset);

        if string == eofstr {
            self.quit();
            self.eof = true;
        } else {
            if self.offset != 0 {
                stdout().write(self.default_hl.to_string().as_bytes())?;
                write!(&mut stdout(), "{1:0$}", self.offset, "")?;
            }
            stdout().write(self.hl.to_string().as_bytes())?;
            stdout().write(string.as_bytes())?;

            self.cursor[1] += self.offset + string.len();
            self.offset = 0;
        }

        Ok(())
    }

    fn handle_cursor_goto(&mut self, args: &[rmp::Value]) -> Result<(), Error> {
        let pos = match args.last() {
            Some(a) => a.as_array().unwrap(),
            None => return Ok(())
        };

        let row = pos[0].as_u64().unwrap() as usize;
        let col = pos[1].as_u64().unwrap() as usize;
        self.modeline = false;
        self.offset = col;

        // println!("{:?}--{:?}", (row, col), self.cursor);
        if row >= HEIGHT - 2 {
            // end of page, jumped to modelines
            self.modeline = true;
            self.lineno += HEIGHT - 1;
            self.scroll(self.lineno);

            self.cursor = [0, 0];
            self.offset = 0;

        } else if row == self.cursor[0]+1 {
            // new line
            self.cursor = [row, 0];

        } else if row == self.cursor[0] && col > self.cursor[1] {
            // moved right on same line
            self.offset -= self.cursor[1];
            self.cursor[0] = row;
            return Ok(())

        }

        if !self.eof {
            stdout().write(self.default_hl.to_string().as_bytes())?;
            stdout().write(b"\n")?;
        }
        Ok(())
    }

    fn handle_highlight_set(&mut self, args: &[rmp::Value]) {
        let hl = match args.last().and_then(|x| x.as_array().unwrap().last()) {
            Some(a) => a.as_map().unwrap(),
            None => {
                // self.hl = self.default_hl.clone();
                self.hl.fg = self.default_hl.fg.clone();
                self.hl.bg = self.default_hl.bg.clone();
                self.hl.attrs = self.default_hl.attrs;
                return
            },
        };

        let mut fg : Option<String> = None;
        let mut bg : Option<String> = None;
        let mut attrs = self.default_hl.attrs;

        for &(ref key, ref value) in hl.iter() {
            let mut bit : Option<Attr> = None;

            match key.as_str().unwrap() {
                "foreground" => {
                    fg = Some( Highlight::rgb_to_string(value.as_u64().unwrap() as u32) );
                },
                "background" => {
                    bg = Some( Highlight::rgb_to_string(value.as_u64().unwrap() as u32) );
                },
                "reverse" => {
                    bit = Some(Attr::REVERSE);
                }
                "bold" => {
                    bit = Some(Attr::BOLD);
                },
                "italic" => {
                    bit = Some(Attr::ITALIC);
                },
                "underline" => {
                    bit = Some(Attr::UNDERLINE);
                },
                _ => (),
            }

            match bit {
                Some(bit) => {
                    if value.as_bool().unwrap() {
                        attrs |= bit as u8;
                    } else {
                        attrs &= !( bit as u8 );
                    }
                },
                None => (),
            }
        }

        self.hl.fg = fg.unwrap_or_else(|| self.default_hl.fg.clone());
        self.hl.bg = bg.unwrap_or_else(|| self.default_hl.bg.clone());
        self.hl.attrs = attrs;
    }

    fn handle_update(&mut self, update: &rmp::Value) -> Result<(), Error> {
        let update = update.as_array().unwrap();
        // println!("\n{:?}", update);
        match update[0].as_str().unwrap() {
            "put" => {
                self.handle_put(&update[1..])?;
            },
            "cursor_goto" => {
                self.handle_cursor_goto(&update[1..])?;
            },
            "highlight_set" => {
                self.handle_highlight_set(&update[1..]);
            },
            "update_fg" => {
                match update[1..].last().and_then(|x| x.as_array().unwrap().last()) {
                    Some(x) => {
                        self.default_hl.fg = Highlight::rgb_to_string(x.as_u64().unwrap() as u32);
                    },
                    None => ()
                };
            },
            "update_bg" => {
                match update[1..].last().and_then(|x| x.as_array().unwrap().last()) {
                    Some(x) => {
                        self.default_hl.bg = Highlight::rgb_to_string(x.as_u64().unwrap() as u32);
                    },
                    None => ()
                };
            },
            _ => (),
        }

        Ok(())
    }

    pub fn run_loop(&mut self) -> Result<(), Error> {
        while !self.eof {
            let value : rmp_serde::Value = Deserialize::deserialize(&mut self.deserializer).unwrap();
            let value = value.as_array().unwrap();
            match value[0].as_u64().unwrap() {
                2 => {
                    // notification
                    let method = value[1].as_str().unwrap();
                    if method == "redraw" {
                        let params = value[2].as_array().unwrap();
                        for update in params {
                            self.handle_update(update)?;
                        }
                    }
                },
                1 => {
                    // response
                },
                _ => (),
            }
        }
        Ok(())
    }
}


fn main() {
    let process = Command::new("nvim")
        .arg("--embed")
        .arg("-nRZ")
        .arg("+0")
        .arg("-c").arg("set scrolloff=0 mouse= showtabline=0")
        .arg("--")
        // .arg("Cargo.toml")
        .arg("src/main.rs")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("could not find nvim")
        ;

    let stdout = process.stdout.unwrap();
    let mut stdin = process.stdin.unwrap();

    let mut printer = Printer::new(&mut stdin, stdout);
    printer.attach();

    if let Err(e) = printer.run_loop() {
        match e.kind() {
            ErrorKind::BrokenPipe => (),
            _ => { panic!("{:?}", e); }
        }
    }

    print!("{}", RESET);
}
