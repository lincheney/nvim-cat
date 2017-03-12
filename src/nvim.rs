extern crate rmp;
extern crate rmp_serde;
extern crate serde;

use std;
use std::collections::{HashMap, BinaryHeap};
use std::io::{stdout, Write};
use std::cell::RefCell;
use std::process::{Command, Child, Stdio, ChildStdout, ChildStdin};

use self::rmp_serde::{Serializer, Deserializer};
use self::serde::{Serialize, Deserialize};
use synattr::{SynAttr, DEFAULT_ATTR};
use rpc::{Reader, Writer, MsgId};

const BUFNUM: usize = 1;
const INIT_COMMAND: &'static str =
    "set scrolloff=0 mouse= showtabline=0 | NoMatchParen";

quick_error! {
    #[derive(Debug)]
    pub enum NvimError {
        RpcError(x: String) { }
        EncodeError(x: rmp_serde::encode::Error) { from() }
        DecodeError(x: rmp_serde::decode::Error) { from() }
        IOError(x: std::io::Error) { from() }
    }
}

#[derive(PartialEq, Eq, Ord)]
struct Line(usize, String);

impl PartialOrd for Line {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        other.0.partial_cmp(&self.0)
    }
}

pub enum Callback {
    AddLine(Line),
    GetSynId(Line),
    GetSynAttr(Line, usize, Vec<SynAttr>, std::vec::IntoIter<rmp::Value>),
    // Command(String),
    // Reset,
    // Quit,
}

pub struct Nvim<'a> {
    reader:         Reader,
    writer:         RefCell<Writer<'a>>,
    syn_attr_cache: RefCell<HashMap<usize, SynAttr>>,
    callbacks:      HashMap<MsgId, Callback>,
    buffer:         BinaryHeap<Line>,
    pub lineno:     usize,
}

impl<'a> Nvim<'a> {
    pub fn start_process() -> Child {
        Command::new("nvim")
            .arg("--embed")
            .arg("-n")
            .arg("-c").arg(INIT_COMMAND)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .expect("could not find nvim")
    }

    pub fn new(stdin: &'a mut ChildStdin, stdout: ChildStdout) -> Self {
        let writer = Writer::new(Serializer::new(stdin));
        let reader = Reader::new(Deserializer::new(stdout));

        Nvim {
            reader: reader,
            writer: RefCell::new(writer),
            syn_attr_cache: RefCell::new(HashMap::new()),
            callbacks: HashMap::new(),
            buffer: BinaryHeap::new(),
            lineno: 2,
        }
    }

    fn nvim_command(&self, command: &str) -> Result<(), NvimError> {
        self.request("nvim_command", (command,))?;
        Ok(())
    }

    fn quit(&self) -> Result<(), NvimError> {
        // don't wait for response, nvim will have quit by then
        self.request("nvim_command", ("qa!",))?;
        Ok(())
    }

    // add @line to vim
    pub fn add_line(&mut self, line: String, lineno: usize) -> Result<(), NvimError> {
        let id = self.request("buffer_insert", (BUFNUM, -1, &[&line]))?;
        self.callbacks.insert(id, Callback::AddLine(Line(lineno, line)));
        Ok(())
    }

    // get syn ids for line @lineno which has length @length
    fn get_synid(&self, lineno: usize, length: usize) -> Result<MsgId, NvimError> {
        // use map to reduce rpc calls
        let range: Vec<usize> = (1..length+1).collect();
        let args = (range, format!("synID({}, v:val, 0)", lineno));
        self.request("vim_call_function", ("map", args))
    }

    // get @line from vim
    fn get_line(&self, line: &String, synattrs: Vec<SynAttr>) -> Result<String, NvimError> {
        let mut parts = String::with_capacity(line.len());
        let mut prev: SynAttr = DEFAULT_ATTR.clone();
        let mut start = 0;
        for (attr, end) in synattrs.into_iter().zip(0..line.len()) {
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

            prev = attr;

            if ! ansi.is_empty() {
                parts.push_str(&line[start..end]);
                parts.push_str("\x1b[");
                parts.push_str(&ansi);
                parts.push_str("m");
                start = end;
            }
        }

        parts.push_str(&line[start..]);
        Ok(parts)
    }

    // get the syn attr for @synid (cached)
    fn get_synattr(&self, synid: usize) -> Result<Option<MsgId>, NvimError> {
        if self.syn_attr_cache.borrow().contains_key(&synid) {
            return Ok(None)
        }

        // use map to reduce rpc calls
        let attrs = ("fg", "bg", "bold", "reverse", "italic", "underline");
        let id = self.request("vim_call_function", ("map", (attrs, format!("synIDattr(synIDtrans({}), v:val, 'gui')", synid)) ))?;
        Ok(Some(id))
    }

    fn get_next_synattr(&mut self, line: Line, mut synattrs: Vec<SynAttr>, mut synids: std::vec::IntoIter<rmp::Value>) -> Result<(), NvimError> {
        for id in synids.next() {
            let id = id.as_u64().expect("expected int") as usize;
            if let Some(id) = self.get_synattr(id)? {
                self.callbacks.insert(id, Callback::GetSynAttr(line, id as usize, synattrs, synids));
                return Ok(())
            }
            synattrs.push(self.syn_attr_cache.borrow().get(&id).unwrap().clone());
        }

        let string = self.get_line(&line.1, synattrs)?;
        if line.0 == self.lineno {
            stdout().write(string.as_bytes())?;
            stdout().write(b"\x1b[0m\n")?;
            let mut lineno = self.lineno;
            lineno += 1;
            while self.buffer.peek().map(|l| l.0 == lineno) == Some(true) {
                let line = self.buffer.pop().unwrap();
                stdout().write(line.1.as_bytes())?;
                stdout().write(b"\x1b[0m\n")?;
                lineno += 1;
            }
            self.lineno = lineno;
        } else {
            self.buffer.push(Line(line.0, string));
        }
        Ok(())
    }


    fn request<T>(&self, command: &str, args: T) -> Result<MsgId, NvimError>
            where T: Serialize {
        self.writer.borrow_mut().write(command, args)
    }

    fn wait_for_response(&mut self, id: MsgId) -> Result<rmp::Value, NvimError> {
        loop {
            if let Some((got_id, value)) = self.reader.read()? {
                if got_id == id {
                    return Ok(value)
                }
            }
        }
    }

    fn reset(&self) -> Result<(), NvimError> {
        // self.syn_attr_cache.clear();

        // clear vim buffer
        let lines: [&str; 0] = [];
        self.request("buffer_set_line_slice", (BUFNUM, 0, -1, true, true, lines))?;
        Ok(())
    }

    pub fn process_event(&mut self) -> Result<(), NvimError> {
        if let Some((id, value)) = self.reader.read()? {
            if let Some(cb) = self.callbacks.remove(&id) {
                match cb {
                    Callback::AddLine(line) => {
                        let id = self.get_synid(line.0, line.1.len())?;
                        self.callbacks.insert(id, Callback::GetSynId(line));
                    },
                    Callback::GetSynId(line) => {
                        let synids = value.as_array().expect("expected an array").clone();
                        let mut synids = synids.into_iter();
                        let synattrs = vec![];
                        self.get_next_synattr(line, synattrs, synids)?;
                    },
                    Callback::GetSynAttr(line, id, mut synattrs, synids) => {
                        let attrs = value.as_array().expect("expected an array");
                        let attrs = SynAttr::new(
                            attrs[0].as_str().expect("expected a string"),
                            attrs[1].as_str().expect("expected a string"),
                            attrs[2].as_str().expect("expected a string"),
                            attrs[3].as_str().expect("expected a string"),
                            attrs[4].as_str().expect("expected a string"),
                            attrs[5].as_str().expect("expected a string"),
                            );

                        self.syn_attr_cache.borrow_mut().insert(id, attrs);
                        synattrs.push(self.syn_attr_cache.borrow().get(&id).unwrap().clone());
                        self.get_next_synattr(line, synattrs, synids)?;
                    },
                }
            }
        }
        Ok(())
    }
}
