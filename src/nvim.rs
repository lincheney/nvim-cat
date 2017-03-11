extern crate rmp;
extern crate rmp_serde;
extern crate serde;

use std;
use std::collections::HashMap;
use std::io::{stdout, Write};
use std::cell::RefCell;
use std::process::{Command, Child, Stdio, ChildStdout, ChildStdin};

use std::sync::mpsc::{Sender, Receiver, channel};
use std::sync::{Arc, Barrier};
use std::thread;

use self::rmp_serde::{Serializer, Deserializer};
use self::serde::{Serialize, Deserialize};
use synattr::{SynAttr, DEFAULT_ATTR};
use rpc::{Transport, MsgId};

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

pub struct Nvim<'a> {
    transport:      RefCell<Transport<'a>>,
    syn_attr_cache: HashMap<usize, SynAttr>,
    barrier:        Arc<Barrier>,
}

pub struct Handle {
    tx:             Sender<Payload>,
    handle:         thread::JoinHandle<()>,
    barrier:        Arc<Barrier>,
}

pub enum Payload {
    Data(String, usize),
    Command(String),
    Reset,
    Quit,
}

impl Handle {
    pub fn add_line(&self, line: String, lineno: usize) {
        self.tx.send(Payload::Data(line, lineno)).unwrap();
    }

    pub fn quit(self) {
        self.tx.send(Payload::Quit).unwrap();
        self.handle.join().unwrap();
    }

    pub fn nvim_command(&self, command: String) {
        self.tx.send(Payload::Command(command)).unwrap();
        self.barrier.wait();
    }

    pub fn reset(&self) {
        self.tx.send(Payload::Reset).unwrap();
        self.barrier.wait();
    }
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

    fn new(stdin: &'a mut ChildStdin, stdout: ChildStdout, barrier: Arc<Barrier>) -> Self {
        let serializer = Serializer::new(stdin);
        let deserializer = Deserializer::new(stdout);
        let transport = Transport::new(serializer, deserializer);

        Nvim {
            transport: RefCell::new(transport),
            syn_attr_cache: HashMap::new(),
            barrier: barrier,
        }
    }

    pub fn start_thread() -> Handle {
        let (tx, rx) = channel();
        let barrier = Arc::new(Barrier::new(2));

        let barrier_copy = barrier.clone();
        let handle = thread::spawn(move || {
            let process = Self::start_process();
            let stdout = process.stdout.unwrap();
            let mut stdin = process.stdin.unwrap();
            let mut nvim = Nvim::new(&mut stdin, stdout, barrier_copy);
            nvim.main_loop(rx);
        });

        Handle{tx: tx, handle: handle, barrier: barrier.clone()}
    }

    fn main_loop(&mut self, rx: Receiver<Payload>) -> Result<(), NvimError> {
        loop {
            match rx.recv() {
                Ok(Payload::Data(line, lineno)) => {
                    self.add_line(&line, lineno)?;
                    let line = self.get_line(&line, lineno)?;
                    stdout().write(line.as_bytes())?;
                    stdout().write(b"\x1b[0m\n")?;
                },
                Ok(Payload::Quit) => {
                    return self.quit();
                },
                Ok(Payload::Reset) => {
                    self.reset();
                    self.barrier.wait();
                },
                Ok(Payload::Command(command)) => {
                    self.nvim_command(&command)?;
                    self.barrier.wait();
                },
                Err(_) => break,
            }
        }
        Ok(())
    }

    fn nvim_command(&self, command: &str) -> Result<(), NvimError> {
        self.request("nvim_command", (command,))?;
        Ok(())
    }

    fn quit(&self) -> Result<(), NvimError> {
        // don't wait for response, nvim will have quit by then
        self.send_request("nvim_command", ("qa!",))?;
        Ok(())
    }

    // add @line to vim
    fn add_line(&self, line: &String, lineno: usize) -> Result<(), NvimError> {
        self.request("buffer_insert", (BUFNUM, -1, &[line]))?;
        Ok(())
    }

    // get @line from vim
    fn get_line(&mut self, line: &String, lineno: usize) -> Result<String, NvimError> {
        // get syntax ids for each char in line
        let synids = self.get_synid(lineno, line.len())?;
        let synids = synids.as_array().expect("expected an array");

        let mut parts = String::with_capacity(line.len());
        let mut prev: SynAttr = DEFAULT_ATTR.clone();
        let mut start = 0;
        for (i, end) in synids.into_iter().zip(0..line.len()) {
            let i = i.as_u64().expect("expected int") as usize;
            // get syntax attr
            let attr = self.get_synattr(i)?;

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

    // get syn ids for line @lineno which has length @length
    fn get_synid(&self, lineno: usize, length: usize) -> Result<rmp::Value, NvimError> {
        // use map to reduce rpc calls
        let range: Vec<usize> = (1..length+1).collect();
        let args = (range, format!("synIDtrans(synID({}, v:val, 0))", lineno));
        self.request("vim_call_function", ("map", args))
    }

    // get the syn attr for @synid (cached)
    fn get_synattr(&mut self, synid: usize) -> Result<&SynAttr, NvimError> {
        if ! self.syn_attr_cache.contains_key(&synid) {
            // use map to reduce rpc calls
            let attrs = ("fg", "bg", "bold", "reverse", "italic", "underline");
            let attrs = self.request("vim_call_function", ("map", (attrs, format!("synIDattr({}, v:val, 'gui')", synid)) ))?;

            let attrs = attrs.as_array().expect("expected an array");
            let attrs = SynAttr::new(
                attrs[0].as_str().expect("expected a string"),
                attrs[1].as_str().expect("expected a string"),
                attrs[2].as_str().expect("expected a string"),
                attrs[3].as_str().expect("expected a string"),
                attrs[4].as_str().expect("expected a string"),
                attrs[5].as_str().expect("expected a string"),
            );

            self.syn_attr_cache.insert(synid, attrs);
        }

        Ok(self.syn_attr_cache.get(&synid).unwrap())
    }

    fn request<T>(&self, command: &str, args: T) -> Result<rmp::Value, NvimError> where T: Serialize {
        let id = self.send_request(command, args)?;
        self.wait_for_response(id)
    }

    fn send_request<T>(&self, command: &str, args: T) -> Result<MsgId, NvimError>
            where T: Serialize {
        self.transport.borrow_mut().send(command, args)
    }

    fn wait_for_response(&self, id: MsgId) -> Result<rmp::Value, NvimError> {
        loop {
            if let Some((got_id, value)) = self.transport.borrow_mut().recv()? {
                if got_id == id {
                    return Ok(value)
                }
            }
        }
    }

    fn reset(&mut self) -> Result<(), NvimError> {
        // self.syn_attr_cache.clear();

        // clear vim buffer
        let lines: [&str; 0] = [];
        self.request("buffer_set_line_slice", (BUFNUM, 0, -1, true, true, lines))?;
        Ok(())
    }
}
