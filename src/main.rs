extern crate libc;

mod highlight;
mod nvim;
mod epoll;

use std::fs::File;
use std::io::{Read, BufRead};
use std::os::unix::io::AsRawFd;

fn main() {
    let mut poller = epoll::Poller::new(2);

    let process = nvim::Nvim::start_process("src/main.rs");
    let stdout = process.stdout.unwrap();
    let stdout_fd = stdout.as_raw_fd();
    let mut stdin = process.stdin.unwrap();
    let mut nvim = nvim::Nvim::new(&mut stdin, stdout);
    poller.add_fd(stdout_fd).unwrap();
    nvim.attach().unwrap();

    let file = File::open("/dev/stdin").unwrap();
    let stdin_fd = file.as_raw_fd();
    poller.add_fd(stdin_fd).unwrap();
    let mut reader = std::io::BufReader::new(file);

    let num_fds = 2;
    let mut buf = String::new();

    while num_fds > 0 {
        match poller.next(-1) {
            Some(fd) if fd == stdout_fd => {
                if nvim.process_event().unwrap() {
                    poller.del_fd(fd).unwrap();
                    break;
                }
            },

            Some(fd) if fd == stdin_fd => {
                buf.clear();
                if reader.read_line(&mut buf).unwrap() > 0 {
                    nvim.add_lines(&[&buf.trim_right_matches("\n")]).unwrap();
                    // println!("{}", buf);
                } else {
                    poller.del_fd(fd).unwrap();
                    nvim.set_eof();
                }
            },

            Some(_) => unreachable!(),
            None => (),
        }
    }
    return;
    // use mio::*;

    // Setup the server socket
    // let mut file1 = File::open("/dev/stdin").unwrap();
    // let mut file2 = File::open(std::env::args().nth(1).unwrap()).unwrap();
    // let fd1 = file1.as_raw_fd();
    // let fd2 = file2.as_raw_fd();
    // // let mut reader1 = std::io::BufReader::new(file1);
    // // let mut reader2 = std::io::BufReader::new(file2);
//
    // unsafe {
    // let mut flags = libc::fcntl(fd1, libc::F_GETFD);
    // // println!("{}", flags);
    // // flags |= libc::O_NONBLOCK;
    // libc::fcntl(fd1, libc::F_SETFD, flags);
    // }
//
    // // epoll::ctl(epollfd, libc::EPOLL_CTL_ADD, fd1, libc::EPOLLIN as u32).unwrap();
    // // epoll::ctl(epollfd, libc::EPOLL_CTL_ADD, fd2, libc::EPOLLIN as u32).unwrap();
//
    // // let mut buf = String::new();
    // let mut buf = [0; 1024];
    // let mut mapping: HashMap<RawFd, File> = HashMap::new();
    // poller.add_file(&file1).unwrap();
    // poller.add_file(&file2).unwrap();
    // mapping.insert(fd1, file1);
    // mapping.insert(fd2, file2);
//
    // while ! mapping.is_empty() {
        // let fd = poller.next(0);
        // if fd.is_none() {
            // break;
        // }
//
        // // buf.clear();
        // let fd = fd.unwrap();
        // let mut remove = false;
        // match mapping.get_mut(&fd) {
            // None => (),
            // Some(file) => {
                // if file.read(&mut buf).unwrap() > 0 {
                    // std::io::stdout().write(b"1: ");
                    // std::io::stdout().write(&buf);
                    // std::io::stdout().write(b"\n");
                    // std::io::stdout().flush();
                // } else {
                    // remove = true;
                // }
            // }
        // }
//
        // if remove {
            // println!("deleted {}", fd);
            // poller.del_fd(fd).unwrap();
            // mapping.remove(&fd);
        // }
    // }

}
