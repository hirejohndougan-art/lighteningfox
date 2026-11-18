//! Shared memory for the pictures a site process sends (process.rs): instead of every frame of
//! a page crossing the pipe pixel by pixel, the process copies it into a free slot of a memory
//! area the browser shares with it at start, and sends only which slot it's in. The browser
//! copies it out and frees the slot. No free slot, or a picture too big for one: the pipe, as
//! before.
//!
//! The area starts with one flag per slot (0 free, 1 holds a picture for the browser). The
//! process can write anything into it, so the browser checks every slot number and size it's
//! given, and only ever copies out (a picture changing while it's copied is just a bad picture).

#![cfg_attr(not(target_os = "linux"), allow(dead_code))]

use std::sync::atomic::{AtomicU32, Ordering};

pub const SLOTS: usize = 4;
/// Room for a 2048×2048 picture (a full-HD window takes about half).
pub const SLOT_BYTES: usize = 16 << 20;
const HEADER: usize = 4096;
const TOTAL: usize = HEADER + SLOTS * SLOT_BYTES;

pub struct Shm {
    ptr: *mut u8,
}

// SAFETY: the mapping lives as long as the Shm; slots are handed over with atomic flags.
unsafe impl Send for Shm {}
unsafe impl Sync for Shm {}

impl Shm {
    fn map(fd: i32) -> Option<Shm> {
        // SAFETY: mapping a descriptor of at least TOTAL bytes, checked by the callers.
        let p = unsafe { libc::mmap(std::ptr::null_mut(), TOTAL, libc::PROT_READ | libc::PROT_WRITE, libc::MAP_SHARED, fd, 0) };
        (p != libc::MAP_FAILED).then(|| Shm { ptr: p as *mut u8 })
    }

    /// A new area (in the browser), and the descriptor to give the site process.
    #[cfg(target_os = "linux")]
    pub fn create() -> Option<(Shm, std::os::fd::OwnedFd)> {
        use std::os::fd::FromRawFd;
        // SAFETY: plain system calls; the descriptor is owned from here on.
        let fd = unsafe { libc::memfd_create(c"ironswift-pictures".as_ptr(), libc::MFD_CLOEXEC) };
        if fd < 0 {
            return None;
        }
        let owned = unsafe { std::os::fd::OwnedFd::from_raw_fd(fd) };
        if unsafe { libc::ftruncate(fd, TOTAL as libc::off_t) } != 0 {
            return None;
        }
        Some((Shm::map(fd)?, owned))
    }

    #[cfg(not(target_os = "linux"))]
    pub fn create() -> Option<(Shm, std::fs::File)> {
        None
    }

    /// The area the browser handed this site process (`IRONSWIFT_SHM_FD`), if any.
    pub fn inherited() -> Option<&'static Shm> {
        static SHM: std::sync::OnceLock<Option<Shm>> = std::sync::OnceLock::new();
        SHM.get_or_init(|| {
            let fd: i32 = std::env::var("IRONSWIFT_SHM_FD").ok()?.parse().ok()?;
            // SAFETY: fstat on a descriptor number; a wrong one just fails.
            let mut st: libc::stat = unsafe { std::mem::zeroed() };
            if unsafe { libc::fstat(fd, &mut st) } != 0 || (st.st_size as usize) < TOTAL {
                return None;
            }
            Shm::map(fd)
        })
        .as_ref()
    }

    fn flag(&self, slot: usize) -> &AtomicU32 {
        // SAFETY: the header holds SLOTS aligned u32s; slot < SLOTS at every caller.
        unsafe { &*(self.ptr.add(slot * 4) as *const AtomicU32) }
    }

    fn slot(&self, slot: usize) -> *mut u8 {
        // SAFETY: inside the mapping (slot < SLOTS).
        unsafe { self.ptr.add(HEADER + slot * SLOT_BYTES) }
    }

    /// (In the site process) put a picture in a free slot: its number, or None (send it whole).
    pub fn put(&self, px: &[u8]) -> Option<usize> {
        if px.len() > SLOT_BYTES {
            return None;
        }
        let slot = (0..SLOTS).find(|&i| self.flag(i).compare_exchange(0, 2, Ordering::Acquire, Ordering::Relaxed).is_ok())?;
        // SAFETY: the slot is ours (flag 2: being written) and px fits.
        unsafe { std::ptr::copy_nonoverlapping(px.as_ptr(), self.slot(slot), px.len()) };
        self.flag(slot).store(1, Ordering::Release);
        Some(slot)
    }

    /// (In the browser) copy out the `len` bytes the process put in `slot`, and free it.
    pub fn take(&self, slot: usize, len: usize) -> Option<Vec<u8>> {
        if slot >= SLOTS || len > SLOT_BYTES || self.flag(slot).load(Ordering::Acquire) != 1 {
            return None;
        }
        let mut out = vec![0u8; len];
        // SAFETY: slot and len checked above; the bytes may change under us, which only
        // spoils this picture.
        unsafe { std::ptr::copy_nonoverlapping(self.slot(slot), out.as_mut_ptr(), len) };
        self.flag(slot).store(0, Ordering::Release);
        Some(out)
    }
}

/// Make a new area and hand it to the site process `cmd` will start (as IRONSWIFT_SHM_FD).
/// Keep the second value until the process has started.
#[cfg(target_os = "linux")]
pub fn share_with(cmd: &mut std::process::Command) -> Option<(Shm, std::os::fd::OwnedFd)> {
    use std::os::fd::AsRawFd;
    use std::os::unix::process::CommandExt;
    let (shm, fd) = Shm::create()?;
    let raw = fd.as_raw_fd();
    cmd.env("IRONSWIFT_SHM_FD", raw.to_string());
    // SAFETY: only fcntl between fork and exec, on a descriptor that exists in the child.
    unsafe {
        cmd.pre_exec(move || {
            if libc::fcntl(raw, libc::F_SETFD, 0) == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    Some((shm, fd))
}

#[cfg(not(target_os = "linux"))]
pub fn share_with(_cmd: &mut std::process::Command) -> Option<(Shm, ())> {
    None
}

impl Drop for Shm {
    fn drop(&mut self) {
        // SAFETY: unmapping what map() mapped.
        unsafe { libc::munmap(self.ptr as *mut libc::c_void, TOTAL) };
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    use std::os::fd::AsRawFd;

    #[test]
    fn pictures_pass_through_slots() {
        let (browser, fd) = Shm::create().unwrap();
        let process = Shm::map(fd.as_raw_fd()).unwrap();
        let px: Vec<u8> = (0..4000u32).map(|i| i as u8).collect();
        let slots: Vec<usize> = (0..SLOTS).map(|_| process.put(&px).unwrap()).collect();
        assert_eq!(slots, vec![0, 1, 2, 3]);
        assert!(process.put(&px).is_none(), "all slots taken: send it whole");
        assert_eq!(browser.take(2, px.len()).unwrap(), px);
        assert!(browser.take(2, px.len()).is_none(), "taken once");
        assert_eq!(process.put(&px), Some(2), "a freed slot is used again");
        assert!(browser.take(SLOTS, 10).is_none() && browser.take(0, SLOT_BYTES + 1).is_none(), "checked");
        assert!(process.put(&vec![0; SLOT_BYTES + 1]).is_none(), "too big for a slot");
    }
}
