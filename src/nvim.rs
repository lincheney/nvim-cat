extern crate rmp;
extern crate rmp_serde;
extern crate serde;

use std::collections::HashMap;
use std::ascii::AsciiExt;
use std::io::{stderr, Write, Error};
use std::cell::RefCell;
use std::process::{Command, Child, Stdio, ChildStdout, ChildStdin};

use self::rmp_serde::{Serializer, Deserializer};
use self::serde::{Serialize, Deserialize};

const BUFNUM: usize = 1;

#[derive(Clone)]
struct SynAttr {
    fg: String,
    bg: String,
    bold: String,
    reverse: String,
    italic: String,
    underline: String,
}

lazy_static! {
    static ref COLOUR_MAP: HashMap<&'static str, usize> = {
        let mut m = HashMap::new();
        m.insert("black", 0);
        m.insert("darkblue", 4);
        m.insert("darkgreen", 2);
        m.insert("darkcyan", 6);
        m.insert("darkred", 1);
        m.insert("darkmagenta", 5);
        m.insert("darkyellow", 3);
        m.insert("brown", 3);
        m.insert("lightgray", 7);
        m.insert("lightgrey", 7);
        m.insert("gray", 7);
        m.insert("grey", 7);
        m.insert("darkgray", 8);
        m.insert("darkgrey", 8);
        m.insert("blue", 12);
        m.insert("lightblue", 12);
        m.insert("green", 10);
        m.insert("lightgreen", 10);
        m.insert("cyan", 14);
        m.insert("lightcyan", 14);
        m.insert("red", 9);
        m.insert("lightred", 9);
        m.insert("magenta", 13);
        m.insert("lightmagenta", 13);
        m.insert("yellow", 11);
        m.insert("lightyellow", 11);
        m.insert("white", 15);
        m
    };

    static ref DEFAULT_ATTR: SynAttr = SynAttr{
        fg: "".to_string(),
        bg: "".to_string(),
        bold: "".to_string(),
        reverse: "".to_string(),
        italic: "".to_string(),
        underline: "".to_string(),
    };
}

fn parse_colour(string: &str) -> Option<String> {
    if string.is_empty() { return None; }

    if string.chars().next() == Some('#') {
        // rgb
        let i = i64::from_str_radix(&string[1..], 16).unwrap();
        return Some(format!("2;{};{};{}", i>>16, (i>>8)&0xff, i&0xff));
    }

    // named colour
    let string = string.to_ascii_lowercase();
    COLOUR_MAP.get(&string[..]).map(|i| format!("5;{}", i))
}

pub struct Nvim<'a> {
    deserializer:   Deserializer<ChildStdout>,
    serializer:     RefCell<Serializer<'a, rmp_serde::encode::StructArrayWriter> >,
    syn_attr_cache: HashMap<usize, SynAttr>,
}

impl<'a> Nvim<'a> {
    pub fn start_process() -> Child {
        let command = "set scrolloff=0 mouse= showtabline=0 | NoMatchParen";

        Command::new("nvim")
            .arg("--embed")
            .arg("-n")
            .arg("-c").arg(command)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .expect("could not find nvim")
    }

    pub fn new(stdin: &'a mut ChildStdin, stdout: ChildStdout) -> Self {
        let serializer = Serializer::new(stdin);
        let deserializer = Deserializer::new(stdout);

        Nvim {
            deserializer: deserializer,
            serializer: RefCell::new(serializer),
            syn_attr_cache: HashMap::new(),
        }
    }

    pub fn nvim_command(&mut self, id: u64, command: &str) -> Result<(), self::rmp_serde::encode::Error> {
        let value = ( 0, id, "nvim_command", (command,) );
        value.serialize(&mut *self.serializer.borrow_mut())
    }

    pub fn set_filetype(&mut self, filetype: &str) -> Result<(), self::rmp_serde::encode::Error> {
        self.nvim_command(50, &format!("set ft={}", filetype))?;
        self.wait_for_response(50).unwrap();
        Ok(())
    }

    pub fn quit(&mut self) -> Result<(), self::rmp_serde::encode::Error> {
        self.nvim_command(100, "qa!")?;
        // don't wait for response, nvim will have quit by then
        // self.wait_for_response(100);
        Ok(())
    }

    // add @line to vim
    pub fn add_line(&mut self, line: &String) -> Result<(), self::rmp_serde::encode::Error> {
        // insert the line
        let value = ( 0, 200, "buffer_insert", (BUFNUM, -1, &[line]) );
        value.serialize(&mut *self.serializer.borrow_mut())?;
        self.wait_for_response(200).unwrap();
        Ok(())
    }

    // get @line from vim
    pub fn get_line(&mut self, line: &String, lineno: usize) -> Result<String, self::rmp_serde::encode::Error> {
        // get syntax ids for each char in line
        let synids = self.get_synid(lineno, line.len()).unwrap();
        let synids = synids.as_array().unwrap();

        let mut parts: Vec<String> = vec![];
        let mut prev: SynAttr = DEFAULT_ATTR.clone();
        for (i, c) in synids.into_iter().zip(line.chars()) {
            let i = i.as_u64().unwrap() as usize;
            // get syntax attr
            let attr = self.get_synattr(i).unwrap();

            let ansi = {
                let mut ansi: Vec<&str> = vec![];
                if attr.fg != prev.fg { ansi.push(&attr.fg) }
                if attr.bg != prev.bg { ansi.push(&attr.bg) }
                if attr.bold != prev.bold { ansi.push(&attr.bold) }
                if attr.reverse != prev.reverse { ansi.push(&attr.reverse) }
                if attr.italic != prev.italic { ansi.push(&attr.italic) }
                if attr.reverse != prev.reverse { ansi.push(&attr.reverse) }
                ansi.join(";")
            };

            prev = attr.clone();

            if ! ansi.is_empty() {
                parts.push("\x1b[".to_string());
                parts.push(ansi);
                parts.push("m".to_string());
            }
            parts.push(c.to_string());
        }

        Ok(parts.join(""))
    }

    // get syn ids for line @lineno which has length @length
    fn get_synid(&mut self, lineno: usize, length: usize) -> Result<rmp::Value, Error> {
        let id = 30;
        let range: Vec<usize> = (1..length+1).collect();
        // use map to reduce rpc calls
        let args = (range, format!("synIDtrans(synID({}, v:val, 0))", lineno));
        let value = ( 0, id, "vim_call_function", ("map", args) );
        value.serialize(&mut *self.serializer.borrow_mut()).unwrap();
        self.wait_for_response(id)
    }

    // get the syn attr for @synid (cached)
    fn get_synattr(&mut self, synid: usize) -> Result<&SynAttr, Error> {
        if ! self.syn_attr_cache.contains_key(&synid) {
            let id = 31;
            // use map to reduce rpc calls
            let attrs = ("fg", "bg", "bold", "reverse", "italic", "underline");
            let value = ( 0, id, "vim_call_function", ("map", (attrs, format!("synIDattr({}, v:val, 'gui')", synid)) ) );
            value.serialize(&mut *self.serializer.borrow_mut()).unwrap();

            let attrs = match self.wait_for_response(id) {
                Err(e) => { return Err(e) },
                Ok(response) => {
                    let attrs = response.as_array().unwrap();
                    let fg = parse_colour(attrs[0].as_str().unwrap());
                    let bg = parse_colour(attrs[1].as_str().unwrap());
                    let bold = attrs[2].as_str().unwrap();
                    let reverse = attrs[3].as_str().unwrap();
                    let italic = attrs[4].as_str().unwrap();
                    let underline = attrs[5].as_str().unwrap();

                    SynAttr{
                        fg: if let Some(fg) = fg { format!("38;{}", fg) } else { "39".to_string() },
                        bg: if let Some(bg) = bg { format!("48;{}", bg) } else { "49".to_string() },
                        bold: (if bold.is_empty() { "21" } else { "1" }).to_string(),
                        reverse: (if reverse.is_empty() { "27" } else { "7" }).to_string(),
                        italic: (if italic.is_empty() { "23" } else { "3" }).to_string(),
                        underline: (if underline.is_empty() { "24" } else { "4" }).to_string(),
                    }
                },
            };

            self.syn_attr_cache.insert(synid, attrs);
        }

        Ok(self.syn_attr_cache.get(&synid).unwrap())
    }

    pub fn wait_for_response(&mut self, id: u64) -> Result<rmp::Value, Error> {
        loop {
            let value : rmp_serde::Value = Deserialize::deserialize(&mut self.deserializer).unwrap();
            let value = value.as_array().unwrap();
            // println!("\n{:?}", value);
            match value[0].as_u64().unwrap() {
                1 => {
                    // response
                    if ! value[2].is_nil() {
                        let msg = value[2].as_array().unwrap()[1].as_str().unwrap();
                        writeln!(stderr(), "ERROR: {}", msg)?;
                    }
                    if value[1].as_u64().unwrap() == id {
                        return Ok(value[3].clone());
                    }
                },
                2 => {
                    // notification
                },
                _ => (),
            }
        }
    }
}
