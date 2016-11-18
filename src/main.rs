extern crate rmp;
extern crate rmp_serde;
extern crate serde;

use std::process::{Command, Stdio, ChildStdout, ChildStdin};
use std::collections::BTreeMap;

use rmp_serde::{Serializer, Deserializer};
use serde::{Serialize, Deserialize};

const HEIGHT : u64 = 100;
const WIDTH : u64 = 100;

struct Printer<'a> {
    deserializer:   Deserializer<ChildStdout>,
    serializer:     Serializer<'a, rmp_serde::encode::StructArrayWriter>,
    cursor:         [u64; 2],
    eof:            bool,
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
        }
    }

    pub fn attach(&mut self) {
        let mut kwargs = BTreeMap::new();
        kwargs.insert("rgb", true);
        let value = ( 0, 100, "nvim_ui_attach", (HEIGHT, WIDTH, kwargs) );
        value.serialize(&mut self.serializer).unwrap();
    }

    pub fn quit(&mut self) {
        let value = ( 0, 200, "nvim_command", ("qa!",) );
        value.serialize(&mut self.serializer).unwrap();
    }

    fn handle_put(&mut self, args: &[rmp::Value]) {
        if self.eof || self.cursor[0] < 1 {
            return
        }

        let eofstr = format!("~{1:0$}", (WIDTH-1) as usize, "");

        let parts : Vec<_> = args
            .iter()
            .flat_map(|x| x.as_array().unwrap())
            .map(|x| x.as_str().unwrap())
            .collect()
            ;
        let string = parts.join("");

        if string == eofstr {
            self.quit();
            self.eof = true;
        } else {
            print!("{}", string);
            self.cursor[1] += string.len() as u64;
        }
    }

    fn handle_cursor_goto(&mut self, args: &[rmp::Value]) {
        let pos = match args.last() {
            Some(a) => a.as_array().unwrap(),
            None => return
        };

        let row = pos[0].as_u64().unwrap();
        let col = pos[1].as_u64().unwrap();

        if row >= HEIGHT - 2 {
        } else if row == self.cursor[0]+1 && col == 0 {
            if !self.eof && self.cursor[0] != 0 {
                println!("");
            }
            self.cursor = [row, col];
        } else if row == self.cursor[0] && col > self.cursor[1] {
            print!("{1:0$}", (col - self.cursor[1]) as usize, "");
            self.cursor = [row, col];
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
        .arg("--")
        .arg("Cargo.toml")
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
