extern crate rmp;
extern crate rmp_serde;
extern crate serde;

use std;
use std::collections::{HashMap, HashSet, BinaryHeap};
use std::io::{stdout, Write};
use std::cell::RefCell;
use std::rc::Rc;
use std::process::{Command, Child, Stdio, ChildStdout, ChildStdin};

use self::rmp_serde::{Serializer, Deserializer};
use self::serde::Serialize;
use synattr::{SynAttr, default_attr};
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
pub type NvimResult<T> = Result<T, NvimError>;

#[derive(PartialEq, Eq, Debug)]
struct Line {
    pub lineno: usize,
    pub line: String,
    pub synids: Vec<usize>,
    pub pending: RefCell<HashSet<usize>>,
}

impl Ord for Line {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        other.lineno.cmp(&self.lineno)
    }
}

impl PartialOrd for Line {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(&other))
    }
}

pub enum Callback {
    AddLine(usize, String),
    GetSynId(usize, String),
    GetSynAttr(usize),
}

enum FutureSynAttr {
    Result(Rc<SynAttr>),
    Pending,
}

fn char_is_control(c: char) -> bool {
    c != '\t' && c.is_control()
}

fn push_print_str(base: &mut String, string: &str) {
    let mut start = 0;
    for (i, c) in string.match_indices(char_is_control) {
        base.push_str(&string[start..i]);
        let c = c.chars().next().unwrap() as u8;
        let c = if c == 0x7f { '?' } else { (c+0x40) as char };
        base.push('^');
        base.push(c);
        start = i;
    }
    base.push_str(&string[start..]);
}

pub struct Nvim<'a> {
    reader:         Reader,
    writer:         RefCell<Writer<'a>>,
    syn_attr_cache: RefCell<HashMap<usize, FutureSynAttr>>,
    callbacks:      HashMap<MsgId, Callback>,
    queue:          BinaryHeap<Line>,
    pub lineno:     usize,
    default_attr:   Rc<SynAttr>,
}

impl<'a> Nvim<'a> {
    pub fn start_process(vimrc: Option<&str>) -> Child {
        let mut args = vec![];
        if let Some(vimrc) = vimrc {
            args.push("-u"); args.push(vimrc);
        }

        Command::new("nvim")
            .arg("--embed")
            .arg("-nZ")
            .arg("-c").arg(INIT_COMMAND)
            .args(&args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn().expect("could not find nvim")
    }

    pub fn new(stdin: &'a mut ChildStdin, stdout: ChildStdout) -> Self {
        let writer = Writer::new(Serializer::new(stdin));
        let reader = Reader::new(Deserializer::new(stdout));

        let default_attr = Rc::new(default_attr());
        let mut syn_attr_cache = HashMap::new();
        syn_attr_cache.insert(0, FutureSynAttr::Result(default_attr.clone()));

        Nvim {
            reader: reader,
            writer: RefCell::new(writer),
            syn_attr_cache: RefCell::new(syn_attr_cache),
            callbacks: HashMap::new(),
            queue: BinaryHeap::new(),
            lineno: 2,
            default_attr: default_attr,
        }
    }

    pub fn nvim_command(&mut self, command: &str) -> NvimResult<()> {
        let id = self.request("nvim_command", (command,))?;
        self.wait_for_response(id)?;
        Ok(())
    }

    pub fn quit(&self) -> NvimResult<()> {
        // don't wait for response, nvim will have quit by then
        self.request("nvim_command", ("qa!",))?;
        Ok(())
    }

    // add @line to vim
    pub fn add_line(&mut self, line: String, lineno: usize) -> NvimResult<()> {
        let id = self.request("buffer_insert", (BUFNUM, -1, &[&line]))?;
        self.callbacks.insert(id, Callback::AddLine(lineno, line));
        Ok(())
    }

    // get syn ids for line @lineno which has length @length
    fn get_synid(&self, lineno: usize, length: usize) -> NvimResult<MsgId> {
        // use map to reduce rpc calls
        let range: Vec<usize> = (1..length+1).collect();
        let args = (range, format!("synID({}, v:val, 0)", lineno));
        self.request("vim_call_function", ("map", args))
    }

    // get @line from vim
    fn get_line(&self, line: String, synids: Vec<usize>) -> NvimResult<String> {
        let mut parts = String::with_capacity(line.len());
        let mut prev = self.default_attr.clone();
        let mut start = 0;
        for (synid, end) in synids.into_iter().zip(0..line.len()) {
            let attr = match self.syn_attr_cache.borrow().get(&synid) {
                Some(&FutureSynAttr::Result(ref attr)) => attr.clone(),
                _ => unreachable!(),
            };

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
                push_print_str(&mut parts, &line[start..end]);
                parts.push_str("\x1b[");
                parts.push_str(&ansi);
                parts.push_str("m");
                start = end;
            }
        }

        push_print_str(&mut parts, &line[start..]);
        Ok(parts)
    }

    // get the syn attr for @synid (cached)
    fn get_synattr(&mut self, synid: usize) -> NvimResult<bool> {
        if self.syn_attr_cache.borrow().contains_key(&synid) {
            return Ok(true)
        }

        // use map to reduce rpc calls
        let attrs = ("fg", "bg", "bold", "reverse", "italic", "underline");
        let id = self.request("vim_call_function", ("map", (attrs, format!("synIDattr(synIDtrans({}), v:val, 'gui')", synid)) ))?;
        self.syn_attr_cache.borrow_mut().insert(synid, FutureSynAttr::Pending);
        self.callbacks.insert(id, Callback::GetSynAttr(synid));
        Ok(false)
    }

    fn print_lines(&mut self) -> NvimResult<()> {
        while self.queue.peek().map(|l| l.lineno == self.lineno && l.pending.borrow().is_empty()) == Some(true) {
            let line = self.queue.pop().unwrap();
            let line = self.get_line(line.line, line.synids)?;
            stdout().write(line.as_bytes())?;
            stdout().write(b"\x1b[0m\n")?;
            self.lineno += 1;
        }
        Ok(())
    }


    fn request<T>(&self, command: &str, args: T) -> NvimResult<MsgId>
            where T: Serialize {
        self.writer.borrow_mut().write(command, args)
    }

    fn wait_for_response(&mut self, id: MsgId) -> NvimResult<rmp::Value> {
        loop {
            if let Some((got_id, value)) = self.reader.read()? {
                if got_id == id {
                    return Ok(value)
                }
            }
        }
    }

    pub fn reset(&mut self) -> NvimResult<()> {
        // self.syn_attr_cache.clear();
        self.queue.clear();
        self.lineno = 2;

        // clear vim buffer
        let lines: [&str; 0] = [];
        let id = self.request("buffer_set_line_slice", (BUFNUM, 0, -1, true, true, lines))?;
        self.wait_for_response(id)?;
        Ok(())
    }

    pub fn process_event(&mut self) -> NvimResult<()> {
        if let Some((id, value)) = self.reader.read()? {
            if let Some(cb) = self.callbacks.remove(&id) {
                match cb {
                    Callback::AddLine(lineno, line) => {
                        let id = self.get_synid(lineno, line.len())?;
                        self.callbacks.insert(id, Callback::GetSynId(lineno, line));
                    },
                    Callback::GetSynId(lineno, line) => {
                        let synids: Vec<usize> = value
                            .as_array()
                            .expect("expected an array")
                            .iter()
                            .zip(line.chars())
                            // highlight control chars with 1 (specialkey)
                            .map(|(id, c)| if char_is_control(c) { 1 } else { id.as_u64().expect("expected int") as usize } )
                            .collect();

                        let mut set = HashSet::new();
                        for id in synids.iter() {
                            if ! self.get_synattr(*id)? {
                                set.insert(*id);
                            }
                        }
                        let should_print = lineno == self.lineno && set.is_empty();

                        self.queue.push(Line{lineno: lineno, line: line, synids: synids, pending: RefCell::new(set)});
                        if should_print {
                            self.print_lines()?;
                        }
                    },
                    Callback::GetSynAttr(synid) => {
                        let attrs = value.as_array().expect("expected an array");
                        let attrs = SynAttr::new(
                            attrs[0].as_str().expect("expected a string"),
                            attrs[1].as_str().expect("expected a string"),
                            attrs[2].as_str().expect("expected a string"),
                            attrs[3].as_str().expect("expected a string"),
                            attrs[4].as_str().expect("expected a string"),
                            attrs[5].as_str().expect("expected a string"),
                        );
                        self.syn_attr_cache.borrow_mut().insert(synid, FutureSynAttr::Result(Rc::new(attrs)));

                        let mut should_print = false;
                        for line in self.queue.iter() {
                            let removed = line.pending.borrow_mut().remove(&synid);
                            if removed && line.lineno == self.lineno && line.pending.borrow().is_empty() {
                                should_print = true;
                            }
                        }

                        if should_print {
                            self.print_lines()?;
                        }
                    },
                }
            } else {
                unreachable!();
            }
        }
        Ok(())
    }
}
