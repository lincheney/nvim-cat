extern crate rmp;
extern crate rmpv;
extern crate rmp_serde;
extern crate serde;

use std;
use std::collections::{HashMap, HashSet, BinaryHeap};
use std::io::{stdout, Write};
use std::cell::RefCell;
use std::rc::Rc;
use std::process::{Command, Child, Stdio, ChildStdout, ChildStdin};

use self::rmp_serde::Serializer;
use self::serde::Serialize;
use synattr::{SynAttr, default_attr};
use rpc::{Reader, Writer, MsgId};

const BUFNUM: usize = 1;
const INIT_COMMAND: &str = "set scrolloff=0 mouse= showtabline=0 | NoMatchParen";

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
    pub line: Vec<u8>,
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
    AddLine(usize, Vec<u8>),
    GetSynId(usize, Vec<u8>),
    GetSynAttr(usize),
}

enum FutureSynAttr {
    Result(Rc<SynAttr>),
    Pending,
}

fn char_is_control(c: u8) -> bool {
    match c {
        0x09 => false, // tab
        0x7f => true,
        0..=31 => true,
        _ => false,
    }
}

fn push_print_str(vec: &mut Vec<u8>, bytes: &[u8]) {
    let mut start = 0;
    for (i, c) in bytes.iter().enumerate() {
        if char_is_control(*c) {
            vec.extend_from_slice(&bytes[start..i]);
            let c = if *c == 0x7f { b'?' } else { c+0x40 };
            vec.push(b'^');
            vec.push(c);
            start = i+1;
        }
    }
    vec.extend_from_slice(&bytes[start..]);
}

#[derive(Copy, Clone)]
pub struct NvimOptions {
    pub numbered: bool,
    pub restricted_mode: bool,
}

pub struct Nvim {
    reader:         Reader,
    writer:         Writer,
    syn_attr_cache: HashMap<usize, FutureSynAttr>,
    callbacks:      HashMap<MsgId, Callback>,
    queue:          BinaryHeap<Line>,
    pub lineno:     usize,
    options:        NvimOptions,
    default_attr:   Rc<SynAttr>,
}

impl Nvim {
    pub fn start_process(vimrc: Option<&str>, options: NvimOptions) -> Child {
        let mut args = vec![];
        if let Some(vimrc) = vimrc {
            args.push("-u"); args.push(vimrc);
        }
        if options.restricted_mode {
            args.push("-Z");
        }

        Command::new("nvim")
            .arg("--embed")
            // .arg("--headless")
            .arg("-nm")
            .arg("-c").arg(INIT_COMMAND)
            .args(&args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn().expect("could not find nvim")
    }

    pub fn new(stdin: ChildStdin, stdout: ChildStdout, options: NvimOptions) -> Self {
        let writer = Writer::new(Serializer::new(stdin));
        let reader = Reader::new(stdout);

        let default_attr = Rc::new(default_attr());
        let mut syn_attr_cache = HashMap::new();
        syn_attr_cache.insert(0, FutureSynAttr::Result(default_attr.clone()));

        Nvim {
            reader,
            writer,
            syn_attr_cache,
            callbacks: HashMap::new(),
            queue: BinaryHeap::new(),
            lineno: 0,
            default_attr,
            options,
        }
    }

    pub fn ui_attach(&mut self, width: isize, height: isize) -> NvimResult<()> {
        let opts: rmpv::Value = vec![("rgb".into(), true.into())].into();
        let id = self.request("nvim_ui_attach", (width, height, opts))?;
        self.wait_for_response(id)?;
        Ok(())
    }

    pub fn ui_detach(&mut self) -> NvimResult<()> {
        let id = self.request("nvim_ui_detach", &[0; 0])?;
        self.wait_for_response(id)?;
        Ok(())
    }

    pub fn press_enter(&mut self) -> NvimResult<()> {
        self.request("vim_input", ("<CR>",))?;
        Ok(())
    }

    pub fn nvim_command(&mut self, command: &str) -> NvimResult<()> {
        let id = self.request("nvim_command", (command,))?;
        self.wait_for_response(id)?;
        Ok(())
    }

    pub fn quit(&mut self) -> NvimResult<()> {
        // don't wait for response, nvim will have quit by then
        self.request("nvim_command", ("qa!",))?;
        Ok(())
    }

    pub fn filetype_detect(&mut self) -> NvimResult<()> {
        self.request("nvim_command", ("if &ft == '' | silent! filetype detect | endif",))?;
        Ok(())
    }

    // add @line to vim
    pub fn add_line(&mut self, line: String, lineno: usize) -> NvimResult<()> {
        let id = self.request("buffer_insert", (BUFNUM, lineno, &[&line]))?;
        let line: Vec<u8> = Vec::from(&line[..]);
        self.callbacks.insert(id, Callback::AddLine(lineno, line));
        Ok(())
    }

    // get syn ids for line @lineno which has length @length
    fn get_synid(&mut self, lineno: usize, length: usize) -> NvimResult<MsgId> {
        // use map to reduce rpc calls
        let range: Vec<usize> = (1..=length).collect();
        let args = (range, format!("synID({}, v:val, 0)", lineno+1));
        self.request("vim_call_function", ("map", args))
    }

    // get @line from vim
    fn get_line(&self, line: Vec<u8>, synids: Vec<usize>) -> NvimResult<Vec<u8>> {
        let mut parts: Vec<u8> = Vec::with_capacity(line.len());
        let mut prev = self.default_attr.clone();
        let mut start = 0;
        for (synid, end) in synids.into_iter().zip(0..line.len()) {
            let attr = match self.syn_attr_cache.get(&synid) {
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
                if attr.underline != prev.underline { ansi.push(&attr.underline) }
                ansi.join(";")
            };

            prev = attr;

            if ! ansi.is_empty() {
                push_print_str(&mut parts, &line[start..end]);
                parts.extend_from_slice(b"\x1b[");
                parts.extend_from_slice(&ansi.as_bytes());
                parts.push(b'm');
                start = end;
            }
        }

        push_print_str(&mut parts, &line[start..]);
        Ok(parts)
    }

    // get the syn attr for @synid (cached)
    fn get_synattr(&mut self, synid: usize) -> NvimResult<bool> {
        if self.syn_attr_cache.contains_key(&synid) {
            return Ok(true)
        }

        // use map to reduce rpc calls
        let attrs = ("fg", "bg", "bold", "reverse", "italic", "underline");
        let id = self.request("vim_call_function", ("map", (attrs, format!("synIDattr(synIDtrans({}), v:val, &termguicolors ? 'gui' : 'cterm')", synid)) ))?;
        self.syn_attr_cache.insert(synid, FutureSynAttr::Pending);
        self.callbacks.insert(id, Callback::GetSynAttr(synid));
        Ok(false)
    }

    fn print_lines(&mut self) -> NvimResult<()> {
        while self.queue.peek().map(|l| l.lineno == self.lineno && l.pending.borrow().is_empty()) == Some(true) {
            let line = self.queue.pop().unwrap();
            let line = self.get_line(line.line, line.synids)?;

            if self.options.numbered {
                stdout().write_all(format!("{:6}  ", self.lineno+1).as_bytes())?;
            }

            stdout().write_all(&line)?;
            stdout().write_all(b"\x1b[0m\n")?;
            self.lineno += 1;
        }
        Ok(())
    }

    fn request<T>(&mut self, command: &str, args: T) -> NvimResult<MsgId>
            where T: Serialize {
        self.writer.write(command, args)
    }

    fn wait_for_response(&mut self, id: MsgId) -> NvimResult<rmpv::Value> {
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
        self.lineno = 0;

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
                            .zip(&line)
                            // highlight control chars with 1 (specialkey)
                            .map(|(id, c)| if char_is_control(*c) { 1 } else { id.as_u64().expect("expected int") as usize } )
                            .collect();

                        let mut set = HashSet::new();
                        for id in synids.iter() {
                            if ! self.get_synattr(*id)? {
                                set.insert(*id);
                            }
                        }
                        let should_print = lineno == self.lineno && set.is_empty();

                        self.queue.push(Line{lineno, line, synids, pending: RefCell::new(set)});
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
                        self.syn_attr_cache.insert(synid, FutureSynAttr::Result(Rc::new(attrs)));

                        let mut should_print = false;
                        for line in self.queue.iter() {
                            let mut pending = line.pending.borrow_mut();
                            if pending.remove(&synid) && line.lineno == self.lineno && pending.is_empty() {
                                should_print = true;
                            }
                        }

                        if should_print {
                            self.print_lines()?;
                        }
                    },
                }
            }
        }
        Ok(())
    }
}

impl Drop for Nvim {
    fn drop(&mut self) {
        // ignore errors
        self.quit().ok();
    }
}
