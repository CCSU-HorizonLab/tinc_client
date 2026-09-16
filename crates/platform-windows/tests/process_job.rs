#![cfg(windows)]
use std::{
    os::windows::{io::AsRawHandle, process::CommandExt},
    process::{Command, Stdio},
    time::{Duration, Instant},
};
#[test]
fn job_fixture() {
    if std::env::var_os("TINC_JOB_TEST_HELPER").is_some() {
        std::thread::sleep(Duration::from_secs(30));
    }
}
#[test]
fn closing_job_reclaims_owned_process() {
    let job = platform_windows::ProcessJob::new().unwrap();
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "job_fixture"])
        .env("TINC_JOB_TEST_HELPER", "1")
        .creation_flags(0x08000000)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    unsafe {
        job.assign(child.as_raw_handle()).unwrap();
    }
    assert!(child.try_wait().unwrap().is_none());
    drop(job);
    let start = Instant::now();
    while child.try_wait().unwrap().is_none() {
        if start.elapsed() > Duration::from_secs(5) {
            child.kill().unwrap();
            panic!("job failed to reclaim child");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}
