#[cfg(unix)]
const MIN_OPEN_FILES: libc::rlim_t = 8192;

#[cfg(unix)]
fn raise_open_file_limit() -> std::io::Result<(libc::rlim_t, libc::rlim_t)> {
    let mut limit = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    if unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &mut limit) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    let previous = limit.rlim_cur;
    let desired = MIN_OPEN_FILES.min(limit.rlim_max);
    if previous < desired {
        // Finder-launched apps commonly inherit 256. Only raise our soft limit;
        // children inherit it without changing launchd or the user's hard limit.
        limit.rlim_cur = desired;
        if unsafe { libc::setrlimit(libc::RLIMIT_NOFILE, &limit) } != 0 {
            return Err(std::io::Error::last_os_error());
        }
    }
    Ok((previous, limit.rlim_cur))
}

pub(crate) fn prepare() {
    #[cfg(unix)]
    match raise_open_file_limit() {
        Ok((previous, current)) => eprintln!("[jockey] open file limit: {previous} -> {current}"),
        Err(error) => eprintln!("[jockey] could not raise open file limit: {error}"),
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::process::CommandExt;

    #[test]
    fn low_gui_limit_is_raised_and_inherited_by_children() {
        const CHILD: &str = "JOCKEY_RESOURCE_LIMIT_TEST_CHILD";
        if std::env::var_os(CHILD).is_some() {
            let (previous, current) = raise_open_file_limit().unwrap();
            assert!(previous <= 256);
            let mut limit = libc::rlimit {
                rlim_cur: 0,
                rlim_max: 0,
            };
            assert_eq!(
                unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &mut limit) },
                0
            );
            assert_eq!(current, MIN_OPEN_FILES.min(limit.rlim_max).max(previous));
            let inherited = std::process::Command::new("/bin/sh")
                .args(["-c", "ulimit -n"])
                .output()
                .unwrap();
            assert!(inherited.status.success());
            assert_eq!(
                String::from_utf8(inherited.stdout)
                    .unwrap()
                    .trim()
                    .parse::<libc::rlim_t>()
                    .unwrap(),
                current
            );
            return;
        }
        let mut command = std::process::Command::new(std::env::current_exe().unwrap());
        command
            .args([
                "--exact",
                "resource_limits::tests::low_gui_limit_is_raised_and_inherited_by_children",
                "--nocapture",
            ])
            .env(CHILD, "1");
        // Lower the child only, keeping the parallel test runner's process limits intact.
        unsafe {
            command.pre_exec(|| {
                let mut limit = libc::rlimit {
                    rlim_cur: 0,
                    rlim_max: 0,
                };
                if libc::getrlimit(libc::RLIMIT_NOFILE, &mut limit) != 0 {
                    return Err(std::io::Error::last_os_error());
                }
                limit.rlim_cur = 256.min(limit.rlim_max);
                if libc::setrlimit(libc::RLIMIT_NOFILE, &limit) != 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let output = command.output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stdout)
        );
    }
}
