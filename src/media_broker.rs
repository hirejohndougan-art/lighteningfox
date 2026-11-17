//! Media decoders for site processes.
//!
//! Video and audio are decoded by the system's `ffmpeg`/`ffprobe` and played by a sound
//! server's player (media.rs). A site process can't start programs (sandbox.rs), so it asks the
//! browser: each site process gets a Unix socket to the browser (`IRONSWIFT_MEDIA_FD`), and a
//! request names one of those three programs (never a path: the browser looks them up), gives
//! the decoder's arguments and, for a downloaded file, an in-memory file holding the media as
//! the decoder's standard input. The browser starts the program and sends its pipes back over
//! the socket (SCM_RIGHTS), so media.rs reads frames and sound exactly as it does in-process.
//!
//! The decoders read untrusted files, so the browser starts them through
//! `ironswift --media-decoder`, which locks itself down (no network, no writing, read-only
//! system files: `sandbox::protect_media_decoder`) and then runs the program. The sound player
//! only receives decoded samples and runs as is. When a site process ends, its programs are
//! stopped.
//!
//! `Proc` is what media.rs holds either way: a local child process, or one the browser runs.

use std::collections::HashMap;
use std::fs::File;
use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::process::{Child, Command, Stdio};
use std::sync::{Mutex, OnceLock};

/// The programs a site process may ask for.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Prog {
    Ffmpeg,
    Ffprobe,
    Sink,
}

impl Prog {
    fn name(self) -> &'static str {
        match self {
            Prog::Ffmpeg => "ffmpeg",
            Prog::Ffprobe => "ffprobe",
            Prog::Sink => "sink",
        }
    }

    fn from_name(n: &str) -> Option<Prog> {
        Some(match n {
            "ffmpeg" => Prog::Ffmpeg,
            "ffprobe" => Prog::Ffprobe,
            "sink" => Prog::Sink,
            _ => return None,
        })
    }
}

/// What a started program's standard input is.
pub enum Input<'a> {
    Null,
    /// A pipe the caller writes into (`Proc::stdin`).
    Pipe,
    /// This file (an in-memory copy of the media; the decoder opens `/dev/stdin`).
    File(&'a File),
}

/// A running decoder or sound player.
pub struct Proc {
    pub stdin: Option<File>,
    pub stdout: Option<File>,
    inner: Inner,
}

enum Inner {
    Local(Child),
    Remote(u32),
}

impl Proc {
    pub fn kill(&mut self) {
        match &mut self.inner {
            Inner::Local(c) => {
                let _ = c.kill();
                let _ = c.wait();
            }
            Inner::Remote(id) => {
                let _ = request(&format!("kill\0{id}"), None);
            }
        }
    }

    /// Has it ended?
    pub fn exited(&mut self) -> bool {
        match &mut self.inner {
            Inner::Local(c) => matches!(c.try_wait(), Ok(Some(_))),
            Inner::Remote(id) => request(&format!("wait\0{id}"), None).is_none_or(|(r, _)| r == "done\01"),
        }
    }

    /// Read all of its output and wait for it to end (ffprobe).
    pub fn output(mut self) -> Vec<u8> {
        let mut out = Vec::new();
        if let Some(mut o) = self.stdout.take() {
            let _ = io::Read::read_to_end(&mut o, &mut out);
        }
        if let Inner::Local(c) = &mut self.inner {
            let _ = c.wait();
        }
        out
    }
}

/// Start `prog` with `args` (the program's own arguments). In a site process the browser
/// starts it; elsewhere it's started here, with `local` building the command.
pub fn spawn(prog: Prog, args: &[String], input: Input, want_stdout: bool, local: impl FnOnce() -> Option<Command>) -> Option<Proc> {
    if broker_fd().is_some() {
        return remote(prog, args, input, want_stdout);
    }
    let mut cmd = local()?;
    cmd.args(args).stderr(Stdio::null());
    cmd.stdin(match input {
        Input::Null => Stdio::null(),
        Input::Pipe => Stdio::piped(),
        Input::File(f) => Stdio::from(f.try_clone().ok()?),
    });
    cmd.stdout(if want_stdout { Stdio::piped() } else { Stdio::null() });
    let mut child = cmd.spawn().ok()?;
    let stdin = child.stdin.take().map(|s| File::from(OwnedFd::from(s)));
    let stdout = child.stdout.take().map(|s| File::from(OwnedFd::from(s)));
    Some(Proc { stdin, stdout, inner: Inner::Local(child) })
}

// ---------------------------------------------------------------- the site process's side

/// The socket to the browser, in a site process.
fn broker_fd() -> Option<&'static Mutex<OwnedFd>> {
    static FD: OnceLock<Option<Mutex<OwnedFd>>> = OnceLock::new();
    FD.get_or_init(|| {
        let n: RawFd = std::env::var("IRONSWIFT_MEDIA_FD").ok()?.parse().ok()?;
        // SAFETY: the browser handed this descriptor to the process for this use only.
        Some(Mutex::new(unsafe { OwnedFd::from_raw_fd(n) }))
    })
    .as_ref()
}

/// One request and its answer (with any descriptors that came with it).
fn request(msg: &str, fd: Option<RawFd>) -> Option<(String, Vec<OwnedFd>)> {
    let sock = broker_fd()?.lock().ok()?;
    send(sock.as_raw_fd(), msg.as_bytes(), fd.as_slice()).ok()?;
    let (bytes, fds) = recv(sock.as_raw_fd()).ok()?;
    Some((String::from_utf8_lossy(&bytes).into_owned(), fds))
}

fn remote(prog: Prog, args: &[String], input: Input, want_stdout: bool) -> Option<Proc> {
    let (stdin_kind, fd) = match input {
        Input::Null => ("null", None),
        Input::Pipe => ("pipe", None),
        Input::File(f) => ("file", Some(f.as_raw_fd())),
    };
    let mut msg = format!("spawn\0{}\0{stdin_kind}\0{}", prog.name(), want_stdout as u8);
    for a in args {
        msg.push('\0');
        msg.push_str(a);
    }
    let (answer, mut fds) = request(&msg, fd)?;
    let mut parts = answer.split('\0');
    if parts.next() != Some("ok") {
        log!("[media] the browser couldn't start {}: {}", prog.name(), parts.next().unwrap_or(""));
        return None;
    }
    let id: u32 = parts.next()?.parse().ok()?;
    let has_in = parts.next() == Some("1");
    let has_out = parts.next() == Some("1");
    let mut it = fds.drain(..);
    let stdin = if has_in { it.next().map(File::from) } else { None };
    let stdout = if has_out { it.next().map(File::from) } else { None };
    Some(Proc { stdin, stdout, inner: Inner::Remote(id) })
}

// ---------------------------------------------------------------- the browser's side

/// A socket pair for a new site process: (the browser's end, the child's end). The child's end
/// is passed down with `pass_to`.
pub fn pair() -> Option<(OwnedFd, OwnedFd)> {
    let mut fds = [0 as RawFd; 2];
    // SAFETY: socketpair writes two descriptors into the array.
    let r = unsafe { libc::socketpair(libc::AF_UNIX, libc::SOCK_SEQPACKET | libc::SOCK_CLOEXEC, 0, fds.as_mut_ptr()) };
    if r != 0 {
        return None;
    }
    // SAFETY: both descriptors were just created and are owned here.
    Some(unsafe { (OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1])) })
}

/// Let the site process started by `cmd` inherit `child_end` and tell it the number.
pub fn pass_to(cmd: &mut Command, child_end: &OwnedFd) {
    use std::os::unix::process::CommandExt;
    let fd = child_end.as_raw_fd();
    cmd.env("IRONSWIFT_MEDIA_FD", fd.to_string());
    // SAFETY: only fcntl between fork and exec, on a descriptor that exists in the child.
    unsafe {
        cmd.pre_exec(move || {
            if libc::fcntl(fd, libc::F_SETFD, 0) == -1 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        });
    }
}

/// Answer one site process's requests until it goes away; then stop its programs.
pub fn serve(sock: OwnedFd, site: String) {
    let _ = std::thread::Builder::new().name(format!("media-{site}")).spawn(move || {
        let mut procs: HashMap<u32, Child> = HashMap::new();
        let mut next = 1u32;
        while let Ok((bytes, mut fds)) = recv(sock.as_raw_fd()) {
            if bytes.is_empty() {
                break;
            }
            let msg = String::from_utf8_lossy(&bytes).into_owned();
            let mut parts = msg.split('\0');
            match parts.next() {
                Some("spawn") => {
                    let answer = start(&mut parts, fds.pop());
                    let (text, out_fds) = match answer {
                        Ok((child, stdin, stdout)) => {
                            let id = next;
                            next += 1;
                            procs.insert(id, child);
                            let text = format!("ok\0{id}\0{}\0{}", stdin.is_some() as u8, stdout.is_some() as u8);
                            (text, [stdin, stdout].into_iter().flatten().collect::<Vec<_>>())
                        }
                        Err(e) => (format!("err\0{e}"), Vec::new()),
                    };
                    let raw: Vec<RawFd> = out_fds.iter().map(|f| f.as_raw_fd()).collect();
                    if send(sock.as_raw_fd(), text.as_bytes(), &raw).is_err() {
                        break;
                    }
                }
                Some("kill") => {
                    if let Some(mut c) = parts.next().and_then(|i| i.parse().ok()).and_then(|i: u32| procs.remove(&i)) {
                        let _ = c.kill();
                        let _ = c.wait();
                    }
                    if send(sock.as_raw_fd(), b"ok", &[]).is_err() {
                        break;
                    }
                }
                Some("wait") => {
                    let id: u32 = parts.next().and_then(|i| i.parse().ok()).unwrap_or(0);
                    let done = procs.get_mut(&id).is_none_or(|c| matches!(c.try_wait(), Ok(Some(_))));
                    if done {
                        if let Some(mut c) = procs.remove(&id) {
                            let _ = c.wait();
                        }
                    }
                    if send(sock.as_raw_fd(), if done { b"done\x001" } else { b"done\x000" }, &[]).is_err() {
                        break;
                    }
                }
                _ => break,
            }
            // Programs that ended on their own are collected as we go.
            procs.retain(|_, c| !matches!(c.try_wait(), Ok(Some(_))));
        }
        for (_, mut c) in procs {
            let _ = c.kill();
            let _ = c.wait();
        }
    });
}

type Started = (Child, Option<OwnedFd>, Option<OwnedFd>);

fn start(parts: &mut std::str::Split<'_, char>, file: Option<OwnedFd>) -> Result<Started, String> {
    let prog = parts.next().and_then(Prog::from_name).ok_or("unknown program")?;
    let stdin_kind = parts.next().unwrap_or("null");
    let want_stdout = parts.next() == Some("1");
    let args: Vec<String> = parts.map(str::to_string).collect();
    let mut cmd = match prog {
        Prog::Sink => crate::media::sound_sink().ok_or("no sound player")?,
        Prog::Ffmpeg | Prog::Ffprobe => {
            let tool = if prog == Prog::Ffmpeg { crate::media::ffmpeg_path() } else { crate::media::ffprobe_path() }.ok_or("not installed")?;
            let mut c = Command::new(std::env::current_exe().map_err(|e| e.to_string())?);
            c.arg("--media-decoder").arg(tool);
            c.args(&args);
            c
        }
    };
    cmd.stdin(match (stdin_kind, file) {
        ("file", Some(f)) => Stdio::from(f),
        ("pipe", _) => Stdio::piped(),
        _ => Stdio::null(),
    });
    cmd.stdout(if want_stdout { Stdio::piped() } else { Stdio::null() });
    cmd.stderr(Stdio::null());
    let mut child = cmd.spawn().map_err(|e| e.to_string())?;
    let stdin = child.stdin.take().map(OwnedFd::from);
    let stdout = child.stdout.take().map(OwnedFd::from);
    Ok((child, stdin, stdout))
}

/// `ironswift --media-decoder <program> <args…>`: lock down, then become the decoder.
pub fn run_decoder(args: &[String]) -> ! {
    use std::os::unix::process::CommandExt;
    let Some((prog, rest)) = args.split_first() else { std::process::exit(2) };
    if !crate::sandbox::protect_media_decoder() {
        std::process::exit(3);
    }
    let err = Command::new(prog).args(rest).exec();
    log!("[media] couldn't run {prog}: {err}");
    std::process::exit(4)
}

// ---------------------------------------------------------------- messages with descriptors

const MAX_MSG: usize = 64 * 1024;
const MAX_FDS: usize = 4;

fn send(sock: RawFd, data: &[u8], fds: &[RawFd]) -> io::Result<()> {
    let mut iov = libc::iovec { iov_base: data.as_ptr() as *mut libc::c_void, iov_len: data.len() };
    // SAFETY: CMSG_SPACE only computes a size.
    let space = unsafe { libc::CMSG_SPACE((fds.len() * std::mem::size_of::<RawFd>()) as u32) } as usize;
    let mut cbuf = vec![0u8; space.max(1)];
    // SAFETY: an all-zero msghdr is valid; the pointers set below live through sendmsg.
    let mut msg: libc::msghdr = unsafe { std::mem::zeroed() };
    msg.msg_iov = &mut iov;
    msg.msg_iovlen = 1;
    if !fds.is_empty() {
        msg.msg_control = cbuf.as_mut_ptr() as *mut libc::c_void;
        msg.msg_controllen = space as _;
        // SAFETY: the control buffer is big enough for one header with these descriptors.
        unsafe {
            let c = libc::CMSG_FIRSTHDR(&msg);
            (*c).cmsg_level = libc::SOL_SOCKET;
            (*c).cmsg_type = libc::SCM_RIGHTS;
            (*c).cmsg_len = libc::CMSG_LEN((fds.len() * std::mem::size_of::<RawFd>()) as u32) as _;
            std::ptr::copy_nonoverlapping(fds.as_ptr(), libc::CMSG_DATA(c) as *mut RawFd, fds.len());
        }
    }
    // SAFETY: msg points at live buffers.
    let n = unsafe { libc::sendmsg(sock, &msg, libc::MSG_NOSIGNAL) };
    if n < 0 { Err(io::Error::last_os_error()) } else { Ok(()) }
}

fn recv(sock: RawFd) -> io::Result<(Vec<u8>, Vec<OwnedFd>)> {
    let mut buf = vec![0u8; MAX_MSG];
    let mut iov = libc::iovec { iov_base: buf.as_mut_ptr() as *mut libc::c_void, iov_len: buf.len() };
    // SAFETY: CMSG_SPACE only computes a size.
    let space = unsafe { libc::CMSG_SPACE((MAX_FDS * std::mem::size_of::<RawFd>()) as u32) } as usize;
    let mut cbuf = vec![0u8; space];
    // SAFETY: an all-zero msghdr is valid; the pointers set below live through recvmsg.
    let mut msg: libc::msghdr = unsafe { std::mem::zeroed() };
    msg.msg_iov = &mut iov;
    msg.msg_iovlen = 1;
    msg.msg_control = cbuf.as_mut_ptr() as *mut libc::c_void;
    msg.msg_controllen = space as _;
    // SAFETY: msg points at live buffers; received descriptors are closed on exec.
    let n = unsafe { libc::recvmsg(sock, &mut msg, libc::MSG_CMSG_CLOEXEC) };
    if n < 0 {
        return Err(io::Error::last_os_error());
    }
    buf.truncate(n as usize);
    let mut fds = Vec::new();
    // SAFETY: walking the control messages recvmsg filled in.
    unsafe {
        let mut c = libc::CMSG_FIRSTHDR(&msg);
        while !c.is_null() {
            if (*c).cmsg_level == libc::SOL_SOCKET && (*c).cmsg_type == libc::SCM_RIGHTS {
                let count = ((*c).cmsg_len as usize - libc::CMSG_LEN(0) as usize) / std::mem::size_of::<RawFd>();
                let data = libc::CMSG_DATA(c) as *const RawFd;
                for i in 0..count {
                    fds.push(OwnedFd::from_raw_fd(*data.add(i)));
                }
            }
            c = libc::CMSG_NXTHDR(&msg, c);
        }
    }
    Ok((buf, fds))
}

/// An in-memory file holding `bytes` (a site process can't write files).
pub fn mem_file(bytes: &[u8]) -> io::Result<File> {
    // SAFETY: memfd_create with a constant name; the descriptor is owned by the File.
    let fd = unsafe { libc::memfd_create(c"ironswift-media".as_ptr(), libc::MFD_CLOEXEC) };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: fd was just created.
    let mut f = unsafe { File::from_raw_fd(fd) };
    io::Write::write_all(&mut f, bytes)?;
    Ok(f)
}

/// Is this a site process whose decoders the browser starts?
pub fn remote_decoders() -> bool {
    broker_fd().is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Messages and descriptors cross the socket pair.
    #[test]
    fn passes_descriptors() {
        let (a, b) = pair().unwrap();
        let f = mem_file(b"hello").unwrap();
        send(a.as_raw_fd(), b"spawn\0x", &[f.as_raw_fd()]).unwrap();
        let (msg, fds) = recv(b.as_raw_fd()).unwrap();
        assert_eq!(msg, b"spawn\0x");
        assert_eq!(fds.len(), 1);
        let mut got = String::new();
        let mut g = File::from(fds.into_iter().next().unwrap());
        use std::io::{Read, Seek};
        g.seek(io::SeekFrom::Start(0)).unwrap();
        g.read_to_string(&mut got).unwrap();
        assert_eq!(got, "hello");
    }
}
