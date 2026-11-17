//! Operating-system sandboxing (Linux), so that a page that finds a bug in Ironswift still
//! can't do much with it:
//!
//! - **The browser** runs with `no_new_privs` and a seccomp filter that refuses system calls
//!   a browser never needs and attackers love: debugging other processes, loading kernel
//!   modules, mounting, BPF, keyrings, creating namespaces, rebooting.
//! - **Cross-site iframes** (ads, embedded players, widgets from other sites) are drawn in a
//!   separate process (`ironswift --frame-renderer`). On top of the filter above it can't
//!   start programs, and with Landlock (Linux 5.13+) it can only read system files and the
//!   profile, and write nothing at all — not your documents, not your SSH keys.
//!
//! Everything here degrades gracefully: on a kernel without Landlock the frame process still
//! gets the seccomp filter, and if a filter can't be installed Ironswift logs it and goes on.

#[cfg(target_os = "linux")]
mod imp {
    use std::ffi::CString;

    // Classic BPF opcodes (linux/filter.h).
    const BPF_LD_W_ABS: u16 = 0x20;
    const BPF_JEQ_K: u16 = 0x15;
    const BPF_JGE_K: u16 = 0x35;
    const BPF_JSET_K: u16 = 0x45;
    const BPF_RET_K: u16 = 0x06;
    const SECCOMP_RET_ALLOW: u32 = 0x7fff_0000;
    const SECCOMP_RET_ERRNO: u32 = 0x0005_0000;

    #[cfg(target_arch = "x86_64")]
    const AUDIT_ARCH: u32 = 0xC000_003E;
    #[cfg(target_arch = "aarch64")]
    const AUDIT_ARCH: u32 = 0xC000_00B7;

    /// Namespace flags a page has no business creating (clone/unshare).
    const NS_FLAGS: u32 = (libc::CLONE_NEWUSER | libc::CLONE_NEWNS | libc::CLONE_NEWPID | libc::CLONE_NEWNET | libc::CLONE_NEWIPC | libc::CLONE_NEWUTS | libc::CLONE_NEWCGROUP) as u32;

    fn ins(code: u16, jt: u8, jf: u8, k: u32) -> libc::sock_filter {
        libc::sock_filter { code, jt, jf, k }
    }

    /// System calls refused everywhere.
    fn denied_always() -> Vec<libc::c_long> {
        let mut v = vec![
            libc::SYS_ptrace,
            libc::SYS_process_vm_readv,
            libc::SYS_process_vm_writev,
            libc::SYS_mount,
            libc::SYS_umount2,
            libc::SYS_pivot_root,
            libc::SYS_chroot,
            libc::SYS_kexec_load,
            libc::SYS_kexec_file_load,
            libc::SYS_init_module,
            libc::SYS_finit_module,
            libc::SYS_delete_module,
            libc::SYS_bpf,
            libc::SYS_perf_event_open,
            libc::SYS_keyctl,
            libc::SYS_add_key,
            libc::SYS_request_key,
            libc::SYS_userfaultfd,
            libc::SYS_swapon,
            libc::SYS_swapoff,
            libc::SYS_reboot,
            libc::SYS_acct,
            libc::SYS_settimeofday,
            libc::SYS_clock_settime,
            libc::SYS_quotactl,
            libc::SYS_setns,
            libc::SYS_unshare,
            libc::SYS_open_by_handle_at,
            libc::SYS_name_to_handle_at,
            libc::SYS_fanotify_init,
            libc::SYS_move_mount,
            libc::SYS_fsopen,
            libc::SYS_fsmount,
            libc::SYS_open_tree,
        ];
        #[cfg(target_arch = "x86_64")]
        v.extend([libc::SYS_iopl, libc::SYS_ioperm, libc::SYS_uselib, libc::SYS_lookup_dcookie]);
        v
    }

    /// Install a seccomp filter: the listed calls fail with EPERM, clone3 says "not
    /// supported" (so the C library falls back to clone, whose flags can be checked), and
    /// clone can't create namespaces. With `no_exec`, starting programs is refused too; with
    /// `no_net`, so is opening any socket but a local (Unix) one.
    pub fn seccomp(no_exec: bool, no_net: bool) -> Result<(), String> {
        let mut deny = denied_always();
        if no_exec {
            deny.extend([libc::SYS_execve, libc::SYS_execveat]);
        }
        if no_net {
            // io_uring can open connections without the socket call.
            deny.extend([libc::SYS_io_uring_setup, libc::SYS_io_uring_enter, libc::SYS_io_uring_register]);
        }
        let errno = |e: i32| SECCOMP_RET_ERRNO | (e as u32 & 0xffff);
        let mut prog = vec![
            // Wrong architecture (or the x32 ABI): refuse everything.
            ins(BPF_LD_W_ABS, 0, 0, 4),
            ins(BPF_JEQ_K, 1, 0, AUDIT_ARCH),
            ins(BPF_RET_K, 0, 0, errno(libc::EPERM)),
            ins(BPF_LD_W_ABS, 0, 0, 0),
        ];
        #[cfg(target_arch = "x86_64")]
        {
            prog.push(ins(BPF_JGE_K, 0, 1, 0x4000_0000));
            prog.push(ins(BPF_RET_K, 0, 0, errno(libc::EPERM)));
        }
        for nr in &deny {
            prog.push(ins(BPF_JEQ_K, 0, 1, *nr as u32));
            prog.push(ins(BPF_RET_K, 0, 0, errno(libc::EPERM)));
        }
        if no_net {
            // socket(domain, …): only AF_UNIX; then reload the call number.
            prog.push(ins(BPF_JEQ_K, 0, 4, libc::SYS_socket as u32));
            prog.push(ins(BPF_LD_W_ABS, 0, 0, 16));
            prog.push(ins(BPF_JEQ_K, 1, 0, libc::AF_UNIX as u32));
            prog.push(ins(BPF_RET_K, 0, 0, errno(libc::EACCES)));
            prog.push(ins(BPF_LD_W_ABS, 0, 0, 0));
        }
        prog.push(ins(BPF_JEQ_K, 0, 1, libc::SYS_clone3 as u32));
        prog.push(ins(BPF_RET_K, 0, 0, errno(libc::ENOSYS)));
        // clone(flags, …): flags are the first argument (offset 16, low 32 bits).
        prog.push(ins(BPF_JEQ_K, 0, 3, libc::SYS_clone as u32));
        prog.push(ins(BPF_LD_W_ABS, 0, 0, 16));
        prog.push(ins(BPF_JSET_K, 0, 1, NS_FLAGS));
        prog.push(ins(BPF_RET_K, 0, 0, errno(libc::EPERM)));
        prog.push(ins(BPF_RET_K, 0, 0, SECCOMP_RET_ALLOW));
        let fprog = libc::sock_fprog { len: prog.len() as u16, filter: prog.as_mut_ptr() };
        // SAFETY: plain prctl calls with a filter program that outlives the call.
        unsafe {
            if libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) != 0 {
                return Err(format!("no_new_privs: {}", std::io::Error::last_os_error()));
            }
            if libc::prctl(libc::PR_SET_SECCOMP, libc::SECCOMP_MODE_FILTER, &fprog as *const libc::sock_fprog, 0, 0) != 0 {
                return Err(format!("seccomp: {}", std::io::Error::last_os_error()));
            }
        }
        Ok(())
    }

    // Landlock (linux/landlock.h).
    const LANDLOCK_CREATE_RULESET_VERSION: u32 = 1;
    const LANDLOCK_RULE_PATH_BENEATH: u32 = 1;
    const EXECUTE: u64 = 1;
    const WRITE_FILE: u64 = 1 << 1;
    const READ_FILE: u64 = 1 << 2;
    const READ_DIR: u64 = 1 << 3;
    const IOCTL_DEV: u64 = 1 << 15;

    #[repr(C)]
    struct RulesetAttr {
        handled_access_fs: u64,
    }

    #[repr(C, packed)]
    struct PathBeneathAttr {
        allowed_access: u64,
        parent_fd: i32,
    }

    /// Every filesystem access right this kernel's Landlock knows (by ABI version).
    fn all_rights(abi: i64) -> u64 {
        let mut r = (1u64 << 13) - 1; // ABI 1: execute … make_sym
        if abi >= 2 {
            r |= 1 << 13; // refer
        }
        if abi >= 3 {
            r |= 1 << 14; // truncate
        }
        if abi >= 5 {
            r |= IOCTL_DEV;
        }
        r
    }

    /// Restrict this process (and anything it starts) to reading `read` paths and also
    /// writing `write` paths. Paths that don't exist are skipped.
    pub fn landlock(read: &[String], write: &[String]) -> Result<i64, String> {
        // SAFETY: Landlock system calls with pointers to live, correctly laid out structs.
        unsafe {
            let abi = libc::syscall(libc::SYS_landlock_create_ruleset, std::ptr::null::<RulesetAttr>(), 0usize, LANDLOCK_CREATE_RULESET_VERSION);
            if abi < 1 {
                return Err(format!("Landlock isn't available ({})", std::io::Error::last_os_error()));
            }
            let handled = all_rights(abi);
            let attr = RulesetAttr { handled_access_fs: handled };
            let fd = libc::syscall(libc::SYS_landlock_create_ruleset, &attr as *const RulesetAttr, std::mem::size_of::<RulesetAttr>(), 0u32) as i32;
            if fd < 0 {
                return Err(format!("landlock ruleset: {}", std::io::Error::last_os_error()));
            }
            let add = |path: &str, rights: u64| {
                let Ok(c) = CString::new(path) else { return };
                let pfd = libc::open(c.as_ptr(), libc::O_PATH | libc::O_CLOEXEC);
                if pfd < 0 {
                    return;
                }
                // A file (not a folder) can only carry file rights.
                let is_dir = std::fs::metadata(path).is_ok_and(|m| m.is_dir());
                let rights = if is_dir { rights } else { rights & (EXECUTE | WRITE_FILE | READ_FILE | (1 << 14) | IOCTL_DEV) };
                let rule = PathBeneathAttr { allowed_access: rights & handled, parent_fd: pfd };
                libc::syscall(libc::SYS_landlock_add_rule, fd, LANDLOCK_RULE_PATH_BENEATH, &rule as *const PathBeneathAttr, 0u32);
                libc::close(pfd);
            };
            for p in read {
                add(p, READ_FILE | READ_DIR | EXECUTE);
            }
            for p in write {
                add(p, handled);
            }
            if libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) != 0 {
                libc::close(fd);
                return Err("no_new_privs failed".into());
            }
            let r = libc::syscall(libc::SYS_landlock_restrict_self, fd, 0u32);
            libc::close(fd);
            if r != 0 {
                return Err(format!("landlock_restrict_self: {}", std::io::Error::last_os_error()));
            }
            Ok(abi)
        }
    }
}

/// Sandbox the browser process itself (called once the window is open).
pub fn protect_browser() {
    if std::env::var("IRONSWIFT_NO_SANDBOX").is_ok_and(|v| v == "1") {
        log!("[sandbox] off (IRONSWIFT_NO_SANDBOX=1)");
        return;
    }
    #[cfg(target_os = "linux")]
    match imp::seccomp(false, false) {
        Ok(()) => log!("[sandbox] system-call filter on"),
        Err(e) => log!("[sandbox] couldn't install the system-call filter: {e}"),
    }
}

/// Lock down a frame-renderer process: no programs, no network, no writing anywhere, and
/// reading only system files, fonts and the profile's settings (never its cookies or storage).
pub fn protect_frame_renderer() -> bool {
    if std::env::var("IRONSWIFT_NO_SANDBOX").is_ok_and(|v| v == "1") {
        return true;
    }
    #[cfg(target_os = "linux")]
    {
        let mut read: Vec<String> = ["/usr", "/lib", "/lib64", "/lib32", "/etc", "/opt", "/proc", "/sys", "/dev/urandom", "/dev/null", "/run/systemd/resolve", "/nix/store"].iter().map(|s| s.to_string()).collect();
        if let Ok(home) = std::env::var("HOME") {
            for p in [".fonts", ".local/share/fonts", ".config/fontconfig"] {
                read.push(format!("{home}/{p}"));
            }
        }
        // Of the profile only the settings it needs to draw pages: cookies, storage, the
        // cache and history stay with the browser, which makes requests for it.
        if let Some(p) = crate::cookies::profile_dir() {
            for f in ["settings.tsv", "zoom.txt", "hsts.txt", "filters", "permissions.tsv", "font-index.tsv"] {
                read.push(p.join(f).display().to_string());
            }
        }
        // Certificate stores named in the environment (they can live outside /etc).
        for var in ["SSL_CERT_FILE", "SSL_CERT_DIR"] {
            if let Ok(v) = std::env::var(var) {
                read.extend(v.split(':').filter(|p| !p.is_empty()).map(String::from));
            }
        }
        // File frames (a local page framing a local file) can be read where they are.
        if let Ok(d) = std::env::var("IRONSWIFT_FRAME_FILES") {
            read.push(d);
        }
        match imp::landlock(&read, &[]) {
            Ok(abi) => log!("[sandbox] frame renderer: Landlock ABI {abi}, read-only system files"),
            Err(e) => log!("[sandbox] frame renderer: {e}; continuing with the system-call filter only"),
        }
        // No network either: the browser makes its requests and opens its WebSockets
        // (ws_relay.rs).
        if let Err(e) = imp::seccomp(true, true) {
            log!("[sandbox] frame renderer: {e}");
            return false;
        }
    }
    #[cfg(not(target_os = "linux"))]
    log!("[sandbox] no operating-system sandbox on this system yet: the process is separate, but not locked down");
    true
}

/// Lock down a site process (process.rs), which runs one top-level site's pages: no
/// programs, no network (the browser fetches for it), no writing, and of the profile only the
/// settings it needs to draw pages — not the cookies, the cache, history or other sites' storage.
pub fn protect_site_renderer() -> bool {
    if std::env::var("IRONSWIFT_NO_SANDBOX").is_ok_and(|v| v == "1") {
        return true;
    }
    #[cfg(target_os = "linux")]
    {
        let mut read: Vec<String> = ["/usr", "/lib", "/lib64", "/lib32", "/etc", "/opt", "/proc/self", "/sys/devices/system/cpu", "/dev/urandom", "/dev/null", "/nix/store"].iter().map(|s| s.to_string()).collect();
        if let Ok(home) = std::env::var("HOME") {
            for p in [".fonts", ".local/share/fonts", ".config/fontconfig", ".cache/fontconfig"] {
                read.push(format!("{home}/{p}"));
            }
        }
        if let Some(p) = crate::cookies::profile_dir() {
            for f in ["settings.tsv", "zoom.txt", "hsts.txt", "filters", "permissions.tsv", "font-index.tsv"] {
                read.push(p.join(f).display().to_string());
            }
        }
        if let Ok(d) = std::env::var("IRONSWIFT_FRAME_FILES") {
            read.push(d);
        }
        match imp::landlock(&read, &[]) {
            Ok(abi) => log!("[sandbox] site process: Landlock ABI {abi}, read-only system files"),
            Err(e) => log!("[sandbox] site process: {e}; continuing with the system-call filter only"),
        }
        if let Err(e) = imp::seccomp(true, true) {
            log!("[sandbox] site process: {e}");
            return false;
        }
    }
    #[cfg(not(target_os = "linux"))]
    log!("[sandbox] no operating-system sandbox on this system yet: the process is separate, but not locked down");
    true
}

/// Lock down a media decoder (`ffmpeg`/`ffprobe` started for a site process, media_broker.rs),
/// which parses untrusted files: no network, no writing, read-only system files. It may still
/// start a program, once: itself (`--media-decoder` runs this, then the decoder).
pub fn protect_media_decoder() -> bool {
    if std::env::var("IRONSWIFT_NO_SANDBOX").is_ok_and(|v| v == "1") {
        return true;
    }
    #[cfg(target_os = "linux")]
    {
        let read: Vec<String> = ["/usr", "/lib", "/lib64", "/lib32", "/etc", "/opt", "/proc/self", "/sys/devices/system/cpu", "/dev/urandom", "/dev/null", "/nix/store"].iter().map(|s| s.to_string()).collect();
        if let Err(e) = imp::landlock(&read, &[]) {
            log!("[sandbox] media decoder: {e}; continuing with the system-call filter only");
        }
        if let Err(e) = imp::seccomp(false, true) {
            log!("[sandbox] media decoder: {e}");
            return false;
        }
    }
    true
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    /// Run the frame renderer's lockdown in a child process and check what it can still do.
    #[test]
    fn frame_renderer_lockdown() {
        let exe = std::env::current_exe().unwrap();
        if std::env::var("IRONSWIFT_SANDBOX_CHILD").is_ok() {
            return;
        }
        let out = std::process::Command::new(exe)
            .args(["--exact", "sandbox::tests::child_body", "--nocapture", "--include-ignored"])
            .env("IRONSWIFT_SANDBOX_CHILD", "1")
            .output()
            .unwrap();
        let text = String::from_utf8_lossy(&out.stdout).to_string() + &String::from_utf8_lossy(&out.stderr);
        assert!(out.status.success(), "{text}");
        assert!(text.contains("CHILD-OK"), "{text}");
    }

    /// The site process can't reach the network or the cookie jar.
    #[test]
    fn site_renderer_lockdown() {
        let exe = std::env::current_exe().unwrap();
        if std::env::var("IRONSWIFT_SANDBOX_CHILD").is_ok() {
            return;
        }
        let out = std::process::Command::new(exe)
            .args(["--exact", "sandbox::tests::site_child_body", "--nocapture", "--include-ignored"])
            .env("IRONSWIFT_SANDBOX_CHILD", "1")
            .output()
            .unwrap();
        let text = String::from_utf8_lossy(&out.stdout).to_string() + &String::from_utf8_lossy(&out.stderr);
        assert!(out.status.success(), "{text}");
        assert!(text.contains("SITE-CHILD-OK"), "{text}");
    }

    /// A media decoder can't write files or reach the network, but can read system files and
    /// start the decoder program.
    #[test]
    fn media_decoder_lockdown() {
        let exe = std::env::current_exe().unwrap();
        if std::env::var("IRONSWIFT_SANDBOX_CHILD").is_ok() {
            return;
        }
        let out = std::process::Command::new(exe)
            .args(["--exact", "sandbox::tests::media_child_body", "--nocapture", "--include-ignored"])
            .env("IRONSWIFT_SANDBOX_CHILD", "1")
            .output()
            .unwrap();
        let text = String::from_utf8_lossy(&out.stdout).to_string() + &String::from_utf8_lossy(&out.stderr);
        assert!(out.status.success(), "{text}");
        assert!(text.contains("MEDIA-CHILD-OK"), "{text}");
    }

    #[test]
    #[ignore]
    fn media_child_body() {
        if std::env::var("IRONSWIFT_SANDBOX_CHILD").is_err() {
            return;
        }
        let tmp = std::env::temp_dir().join(format!("ironswift-media-sbx-{}", std::process::id()));
        assert!(super::protect_media_decoder());
        assert!(std::fs::write(&tmp, b"x").is_err(), "writing should be refused");
        assert!(std::net::TcpStream::connect("127.0.0.1:9").is_err());
        assert!(std::net::UdpSocket::bind("127.0.0.1:0").is_err(), "UDP should be refused");
        assert!(std::fs::read_dir("/usr").is_ok(), "system files stay readable");
        assert!(std::process::Command::new("/bin/true").status().is_ok_and(|s| s.success()), "the decoder can be started");
        println!("MEDIA-CHILD-OK");
    }

    #[test]
    #[ignore]
    fn site_child_body() {
        if std::env::var("IRONSWIFT_SANDBOX_CHILD").is_err() {
            return;
        }
        assert!(super::protect_site_renderer());
        // No internet sockets (TCP or UDP)…
        assert!(std::net::TcpStream::connect("127.0.0.1:9").is_err());
        assert!(std::net::UdpSocket::bind("127.0.0.1:0").is_err(), "UDP should be refused");
        // …but local socket pairs (used by the runtime) still work.
        assert!(std::os::unix::net::UnixStream::pair().is_ok());
        assert!(std::process::Command::new("/bin/true").status().is_err(), "exec should be refused");
        assert_eq!(std::thread::spawn(|| 2 * 21).join().unwrap(), 42);
        println!("SITE-CHILD-OK");
    }

    #[test]
    #[ignore]
    fn child_body() {
        if std::env::var("IRONSWIFT_SANDBOX_CHILD").is_err() {
            return;
        }
        let tmp = std::env::temp_dir().join(format!("ironswift-sbx-{}", std::process::id()));
        let _ = std::fs::write(&tmp, b"x");
        // A profile with a cookie jar and settings.
        let profile = std::env::temp_dir().join(format!("ironswift-sbx-profile-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&profile);
        let _ = std::fs::write(profile.join("cookies.tsv"), b"bank.com\tsid\tsecret");
        let _ = std::fs::write(profile.join("settings.tsv"), b"site_processes\t0\n");
        // SAFETY: the child test runs alone in its process.
        unsafe { std::env::set_var("IRONSWIFT_PROFILE", &profile) };
        assert!(super::protect_frame_renderer());
        // System files are still readable…
        assert!(std::fs::read_to_string("/etc/hostname").is_ok() || std::fs::metadata("/etc/hostname").is_err());
        // …programs can't be started…
        assert!(std::process::Command::new("/bin/true").status().is_err(), "exec should be refused");
        // …and nothing can be debugged or written.
        assert_eq!(unsafe { libc::ptrace(libc::PTRACE_TRACEME, 0, 0, 0) }, -1);
        let landlocked = unsafe { libc::syscall(libc::SYS_landlock_create_ruleset, std::ptr::null::<u8>(), 0usize, 1u32) } >= 1;
        if landlocked {
            assert!(std::fs::write(&tmp, b"y").is_err(), "writes should be refused");
            assert!(std::fs::read(&tmp).is_err(), "the temp folder isn't readable");
            // The cookie jar stays with the browser; settings are readable.
            assert!(std::fs::read(profile.join("cookies.tsv")).is_err(), "cookies must not be readable");
            assert!(std::fs::read(profile.join("settings.tsv")).is_ok(), "settings are readable");
        }
        // No internet sockets: the browser opens connections for it.
        assert!(std::net::TcpStream::connect("127.0.0.1:9").is_err());
        assert!(std::net::UdpSocket::bind("127.0.0.1:0").is_err(), "UDP sockets should be refused");
        // Threads still work (clone without namespace flags).
        assert_eq!(std::thread::spawn(|| 41 + 1).join().unwrap(), 42);
        println!("CHILD-OK landlock={landlocked}");
    }
}
