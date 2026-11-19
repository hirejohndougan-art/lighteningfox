//! A site or frame process's stderr reaches about:log (it went to a console the browser on
//! Windows doesn't have, so a site process that died left only "failed to fill whole buffer").

#[cfg(unix)]
#[test]
fn a_child_process_stderr_lands_in_the_log() {
    let mut child = std::process::Command::new("sh")
        .arg("-c")
        .arg("echo \"thread 'page' has overflowed its stack\" >&2")
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("sh");
    crate::process::relay_stderr(child.stderr.take().expect("stderr"), "relay-test.example");
    child.wait().expect("ends");
    let want = "[relay-test.example] thread 'page' has overflowed its stack";
    let t = std::time::Instant::now();
    while !crate::crashlog::lines().iter().any(|(_, l)| l == want) {
        assert!(t.elapsed().as_secs() < 5, "not in the log");
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}
