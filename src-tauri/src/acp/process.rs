pub(crate) fn terminate_pid(pid: u32) {
    #[cfg(unix)]
    unsafe {
        let pgid = -(pid as i32);
        let _ = libc::kill(pgid, libc::SIGTERM);
        let _ = libc::kill(pgid, libc::SIGKILL);
        let _ = libc::kill(pid as i32, libc::SIGTERM);
        let _ = libc::kill(pid as i32, libc::SIGKILL);
    }
    #[cfg(not(unix))]
    {
        let _ = std::process::Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/T", "/F"])
            .output();
    }
}
