extern crate rmp;
extern crate rmp_serde;
extern crate serde;

use std::process::{Command, Stdio, ChildStdout, ChildStdin};
use std::collections::BTreeMap;

use rmp_serde::{Serializer, Deserializer};
use serde::{Serialize, Deserialize};

const HEIGHT : usize = 100;
const WIDTH : usize = 100;

struct Printer<'a> {
    deserializer:   Deserializer<ChildStdout>,
    serializer:     Serializer<'a, rmp_serde::encode::StructArrayWriter>,
    cursor:         [usize; 2],
    eof:            bool,
    modeline:       bool,
    offset:         usize,
}

impl<'a> Printer<'a> {
    pub fn new(stdin: &'a mut ChildStdin, stdout: ChildStdout) -> Self {
        let serializer = Serializer::new(stdin);
        let deserializer = Deserializer::new(stdout);
        Printer {
            serializer: serializer,
            deserializer: deserializer,
            cursor: [0, 0],
            eof: false,
            modeline: false,
            offset: 0,
        }
    }

    pub fn attach(&mut self) {
        let mut kwargs = BTreeMap::new();
        kwargs.insert("rgb", true);
        let value = ( 0, 100, "nvim_ui_attach", (WIDTH, HEIGHT, kwargs) );
        value.serialize(&mut self.serializer).unwrap();
    }

    pub fn quit(&mut self) {
        let value = ( 0, 200, "nvim_command", ("qa!",) );
        value.serialize(&mut self.serializer).unwrap();
    }

    fn scroll(&mut self, line: usize) {
        let command = format!("normal {}z\n", line);
        let value = ( 0, 300, "nvim_command", (command,) );
        value.serialize(&mut self.serializer).unwrap();
    }

    fn handle_put(&mut self, args: &[rmp::Value]) {
        if self.eof || self.modeline {
            return
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
            print!("{1:0$}{2}", self.offset, "", string);
            self.cursor[1] += self.offset + string.len();
            self.offset = 0;
        }
    }

    fn handle_cursor_goto(&mut self, args: &[rmp::Value]) {
        let pos = match args.last() {
            Some(a) => a.as_array().unwrap(),
            None => return
        };

        let row = pos[0].as_u64().unwrap() as usize;
        let col = pos[1].as_u64().unwrap() as usize;
        self.modeline = false;
        self.offset = col;

        // println!("{:?}--{:?}", (row, col), self.cursor);
        if row >= HEIGHT - 2 {
            // end of page, jumped to modelines
            self.modeline = true;
            self.scroll(HEIGHT - 1);
            self.cursor = [0, 0];
            self.offset = 0;
            if !self.eof {
                println!("");
            }

        } else if row == self.cursor[0]+1 {
            // new line
            if !self.eof {
                println!("");
            }
            self.cursor = [row, 0];

        } else if row == self.cursor[0] && col > self.cursor[1] {
            // moved right on same line
            self.offset -= self.cursor[1];
            self.cursor[0] = row;

        }
    }

    fn handle_update(&mut self, update: &rmp::Value) {
        let update = update.as_array().unwrap();
        match update[0].as_str().unwrap() {
            "put" => {
                self.handle_put(&update[1..]);
            },
            "cursor_goto" => {
                self.handle_cursor_goto(&update[1..]);
            },
            _ => (),
        }
    }

    pub fn run_loop(&mut self) {
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
                            self.handle_update(update);
                        }
                    }
                },
                1 => {
                    // response
                },
                _ => (),
            }
        }
    }
}


fn main() {
    let process = Command::new("nvim")
        .arg("--embed")
        .arg("-nR")
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
    printer.run_loop();
}
