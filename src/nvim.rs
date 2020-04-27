extern crate rmp;
extern crate rmpv;
extern crate rmp_serde;
extern crate serde;

use std::collections::{HashMap, HashSet, VecDeque};
use std::io::{stdout, Write, Cursor};
use std::process::{Command, Child, Stdio, ChildStdout, ChildStdin};
use std::default::Default;

use self::rmp_serde::Serializer;
use self::serde::Serialize;
use synattr::SynAttr;
use rpc::{Reader, Writer, MsgId};

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
    pub pending: HashSet<usize>,
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
    Result(SynAttr),
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
    queue:          VecDeque<Option<Line>>,
    pub lineno:     usize,
    default_attr:   SynAttr,
    normal_attr:    SynAttr,
    scratch_space:  Vec<u8>,
    termguicolors:  bool,
    hi_linenr:      Option<SynAttr>,
}

impl Nvim {
    pub fn start_process(vimrc: Option<&str>, colorscheme: Option<&str>, options: NvimOptions) -> Child {
        let mut command = Command::new("nvim");
        command.arg("--embed");
        command.arg("-nm");
        // command.arg("--headless");
        command.arg("-c").arg(INIT_COMMAND);

        if let Some(vimrc) = vimrc {
            command.arg("-u").arg(vimrc);
        }
        if let Some(colorscheme) = colorscheme {
            command.arg("-c").arg(format!("colorscheme {}", colorscheme));
        }
        if options.restricted_mode {
            command.arg("-Z");
        }

        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn().expect("could not find nvim")
    }

    pub fn new(stdin: ChildStdin, stdout: ChildStdout, options: NvimOptions) -> NvimResult<Self> {
        let writer = Writer::new(Serializer::new(stdin));
        let reader = Reader::new(stdout);

        let mut nvim = Nvim {
            reader,
            writer,
            syn_attr_cache: HashMap::new(),
            callbacks: HashMap::new(),
            queue: VecDeque::new(),
            lineno: 0,
            termguicolors: false,
            default_attr: Default::default(),
            normal_attr: Default::default(),
            scratch_space: vec![],
            hi_linenr: None,
        };

        // neovim pauses for 1s if there are errors and no ui
        nvim.ui_attach(100, 100)?;
        nvim.press_enter()?; // press enter now and then to get past blocking error messages
        nvim.ui_detach()?;

        let id = nvim.request("nvim_get_option", ("termguicolors",))?;
        nvim.termguicolors = nvim.wait_for_response(id)?.as_bool().expect("expected a bool");

        // get synattr of Normal
        let normal = nvim._get_synattr("Normal")?;
        nvim.syn_attr_cache.insert(0, FutureSynAttr::Result(normal.clone()));
        nvim.normal_attr = normal;

        if options.numbered {
            nvim.hi_linenr = Some(nvim._get_synattr("LineNR")?);
        }

        Ok(nvim)
    }

    fn _get_synattr(&mut self, name: &str) -> NvimResult<SynAttr> {
        let attrs = ("fg", "bg", "bold", "reverse", "italic", "underline");
        let func = format!("synIDattr(synIDtrans(hlID('{}')), v:val, &termguicolors ? 'gui' : 'cterm')", name);
        let id = self.request("vim_call_function", ("map", (attrs, func) ))?;

        let value = self.wait_for_response(id)?;
        let attrs = value.as_array().expect("expected an array");
        Ok(SynAttr::new(
            attrs[0].as_str().expect("expected a string"),
            attrs[1].as_str().expect("expected a string"),
            attrs[2].as_str().expect("expected a string"),
            attrs[3].as_str().expect("expected a string"),
            attrs[4].as_str().expect("expected a string"),
            attrs[5].as_str().expect("expected a string"),
            &self.normal_attr,
            self.termguicolors,
        ))
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

    pub fn buf_set_name(&mut self, name: &str) -> NvimResult<()> {
        self.request("nvim_buf_set_name", (0, name))?;
        Ok(())
    }

    pub fn filetype_detect(&mut self) -> NvimResult<()> {
        self.request("nvim_command", ("if &ft == '' | silent! filetype detect | endif",))?;
        Ok(())
    }

    // add @line to vim
    pub fn add_line(&mut self, line: String, lineno: usize) -> NvimResult<()> {
        let id = self.request("buffer_insert", (0, lineno, &[&line]))?;
        let line: Vec<u8> = Vec::from(&line[..]);
        self.callbacks.insert(id, Callback::AddLine(lineno, line));
        Ok(())
    }

    // get syn ids for line @lineno which has length @length
    fn get_synid(&mut self, lineno: usize, length: usize) -> NvimResult<MsgId> {
        // use map to reduce rpc calls
        let expr = format!("map(range(1, {}), \"synID({}, v:val, 0)\")", length+1, lineno+1);
        self.request("nvim_eval", (expr,))
    }

    // get @line from vim
    fn get_line(&mut self, line: Vec<u8>, synids: Vec<usize>) -> NvimResult<&[u8]> {
        if line.len() > self.scratch_space.capacity() {
            self.scratch_space.reserve(line.len() - self.scratch_space.capacity());
        }
        self.scratch_space.clear();

        let mut ansi = [0u8; 256];
        macro_rules! ansi_write {
            ($buf:ident, $prev:ident, $attr:ident, $field:ident) => ({
                if $prev.$field != $attr.$field {
                    $buf.write_all(b";").unwrap();
                    $buf.write_all($attr.$field.as_bytes()).unwrap();
                }
            })
        }

        let mut prev_synid = synids.get(0).unwrap_or(&0) + 1;
        let mut prev_attr = &self.default_attr;
        let mut start = 0;

        let synids = synids.iter().chain(std::iter::once(&0));
        for (end, &synid) in synids.enumerate() {
            if synid == prev_synid {
                continue
            }

            let attr = match self.syn_attr_cache.get(&synid) {
                Some(&FutureSynAttr::Result(ref attr)) => attr,
                _ => unreachable!(),
            };

            let mut ansi = Cursor::new(&mut ansi as &mut [u8]);
            ansi_write!(ansi, prev_attr, attr, fg);
            ansi_write!(ansi, prev_attr, attr, bg);
            ansi_write!(ansi, prev_attr, attr, bold);
            ansi_write!(ansi, prev_attr, attr, reverse);
            ansi_write!(ansi, prev_attr, attr, italic);
            ansi_write!(ansi, prev_attr, attr, underline);
            prev_attr = attr;
            prev_synid = synid;

            let ansi = &ansi.get_ref()[..ansi.position() as usize];
            if ! ansi.is_empty() {
                push_print_str(&mut self.scratch_space, &line[start..end]);
                self.scratch_space.extend_from_slice(b"\x1b[");
                self.scratch_space.extend_from_slice(&ansi[1..]);
                self.scratch_space.push(b'm');
                start = end;
            }
        }

        push_print_str(&mut self.scratch_space, &line[start..]);
        Ok(&self.scratch_space)
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
        let stdout = stdout();
        let mut stdout = stdout.lock();
        loop {
            match self.queue.get(0) {
                Some(Some(l)) if l.lineno == self.lineno && l.pending.is_empty() => (),
                _ => break,
            }

            if let Some(ref attr) = self.hi_linenr {
                write!(
                    stdout,
                    "\x1b[{fg};{bg};{bold};{reverse};{italic};{underline}m{lineno:6}  \x1b[0m",
                    fg=attr.fg,
                    bg=attr.bg,
                    bold=attr.bold,
                    reverse=attr.reverse,
                    italic=attr.italic,
                    underline=attr.underline,
                    lineno=self.lineno+1,
                )?;
            }

            let line = self.queue.pop_front().unwrap().unwrap();
            let line = self.get_line(line.line, line.synids)?;
            stdout.write_all(line)?;
            stdout.write_all(b"\x1b[K\x1b[0m\n")?;
            self.lineno += 1;
        }
        Ok(())
    }

    pub fn request<T: Serialize>(&mut self, command: &str, args: T) -> NvimResult<MsgId> {
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
        self.nvim_command("bwipe!")?;
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
                        for &id in synids.iter() {
                            if ! self.get_synattr(id)? {
                                set.insert(id);
                            }
                        }
                        let should_print = lineno == self.lineno && set.is_empty();

                        let index = lineno - self.lineno;
                        for _ in self.queue.len()..=index {
                            self.queue.push_back(None);
                        }
                        let line = Line{lineno, line, synids, pending: set};
                        self.queue[index] = Some(line);

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
                            &self.normal_attr,
                            self.termguicolors,
                        );
                        self.syn_attr_cache.insert(synid, FutureSynAttr::Result(attrs));

                        let mut should_print = false;
                        for line in self.queue.iter_mut() {
                            if let Some(line) = line {
                                if line.pending.remove(&synid) && line.lineno == self.lineno && line.pending.is_empty() {
                                    should_print = true;
                                }
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
