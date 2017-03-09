extern crate rmp;
extern crate rmp_serde;
extern crate serde;

use std;
use std::collections::HashMap;
use std::io::{stderr, Write};
use std::cell::RefCell;
use std::process::{Command, Child, Stdio, ChildStdout, ChildStdin};

use self::rmp_serde::{Serializer, Deserializer};
use self::serde::{Serialize, Deserialize};
use synattr::{SynAttr, DEFAULT_ATTR};

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
    deserializer:   Deserializer<ChildStdout>,
    serializer:     RefCell<Serializer<'a, rmp_serde::encode::StructArrayWriter> >,
    syn_attr_cache: HashMap<usize, SynAttr>,
    rpc_id:         usize,
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
        let serializer = Serializer::new(stdin);
        let deserializer = Deserializer::new(stdout);

        Nvim {
            deserializer: deserializer,
            serializer: RefCell::new(serializer),
            syn_attr_cache: HashMap::new(),
            rpc_id: 100,
        }
    }

    pub fn nvim_command(&mut self, command: &str) -> Result<(), NvimError> {
        self.request("nvim_command", (command,))?;
        Ok(())
    }

    pub fn set_filetype(&mut self, filetype: &str) -> Result<(), NvimError> {
        self.nvim_command(&format!("set ft={}", filetype))
    }

    pub fn quit(&mut self) -> Result<(), NvimError> {
        // don't wait for response, nvim will have quit by then
        self.send_request("nvim_command", ("qa!",))?;
        Ok(())
    }

    // add @line to vim
    pub fn add_line(&mut self, line: &String) -> Result<(), NvimError> {
        self.request("buffer_insert", (BUFNUM, -1, &[line]))?;
        Ok(())
    }

    // get @line from vim
    pub fn get_line(&mut self, line: &String, lineno: usize) -> Result<String, NvimError> {
        // get syntax ids for each char in line
        let synids = self.get_synid(lineno, line.len())?;
        let synids = synids.as_array().expect("expected an array");

        let mut parts: Vec<String> = vec![];
        let mut prev: SynAttr = DEFAULT_ATTR.clone();
        for (i, c) in synids.into_iter().zip(line.chars()) {
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
                parts.push("\x1b[".to_string());
                parts.push(ansi);
                parts.push("m".to_string());
            }
            parts.push(c.to_string());
        }

        Ok(parts.join(""))
    }

    // get syn ids for line @lineno which has length @length
    fn get_synid(&mut self, lineno: usize, length: usize) -> Result<rmp::Value, NvimError> {
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

    pub fn request<T>(&mut self, command: &str, args: T) -> Result<rmp::Value, NvimError> where T: Serialize {
        let id = self.send_request(command, args)?;
        self.wait_for_response(id)
    }

    fn send_request<T>(&mut self, command: &str, args: T) -> Result<usize, NvimError> where T: Serialize {
        self.rpc_id += 1;
        let value = ( 0, self.rpc_id, command, args );
        value.serialize(&mut *self.serializer.borrow_mut())?;
        Ok(self.rpc_id)
    }

    fn wait_for_response(&mut self, id: usize) -> Result<rmp::Value, NvimError> {
        let id = id as u64;
        loop {
            let value : rmp_serde::Value = Deserialize::deserialize(&mut self.deserializer)?;
            let value = value.as_array().expect("expected an array");
            // println!("\n{:?}", value);
            match value[0].as_u64().expect("expected an int") {
                1 => {
                    // response
                    let err_msg = if ! value[2].is_nil() {
                        Some(value[2].as_array().expect("expected an array")[1].as_str().expect("expected a string"))
                    } else {
                        None
                    };

                    if value[1].as_u64().expect("expected an int") == id {
                        if let Some(err_msg) = err_msg {
                            return Err(NvimError::RpcError(err_msg.to_string()));
                        }
                        return Ok(value[3].clone());
                    }

                    if let Some(err_msg) = err_msg {
                        // ignore problems with printing errors
                        writeln!(stderr(), "ERROR: {}", err_msg).unwrap_or(());
                    }
                },
                2 => {
                    // notification
                },
                _ => (),
            }
        }
    }

    pub fn reset(&mut self) -> Result<(), NvimError> {
        // self.syn_attr_cache.clear();

        // clear vim buffer
        let lines: [&str; 0] = [];
        self.request("buffer_set_line_slice", (BUFNUM, 0, -1, true, true, lines))?;
        Ok(())
    }
}
