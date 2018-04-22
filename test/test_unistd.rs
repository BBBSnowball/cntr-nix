extern crate tempdir;

use nix::fcntl::{self, fcntl, FcntlArg, FdFlag, OFlag};
use nix::unistd::*;
use nix::unistd::ForkResult::*;
use nix::sys::signal::{SaFlags, SigAction, SigHandler, SigSet, Signal, sigaction};
use nix::sys::wait::*;
use nix::sys::stat::{self, Mode, SFlag};
use std::{self, env, iter};
use std::ffi::CString;
use std::fs::File;
use std::io::Write;
use std::os::unix::prelude::*;
use tempfile::tempfile;
use tempdir::TempDir;
use libc::{self, _exit, off_t};

#[test]
fn test_fork_and_waitpid() {
    #[allow(unused_variables)]
    let m = ::FORK_MTX.lock().expect("Mutex got poisoned by another test");

    // Safe: Child only calls `_exit`, which is signal-safe
    match fork().expect("Error: Fork Failed") {
        Child => unsafe { _exit(0) },
        Parent { child } => {
            // assert that child was created and pid > 0
            let child_raw: ::libc::pid_t = child.into();
            assert!(child_raw > 0);
            let wait_status = waitpid(child, None);
            match wait_status {
                // assert that waitpid returned correct status and the pid is the one of the child
                Ok(WaitStatus::Exited(pid_t, _)) =>  assert!(pid_t == child),

                // panic, must never happen
                s @ Ok(_) => panic!("Child exited {:?}, should never happen", s),

                // panic, waitpid should never fail
                Err(s) => panic!("Error: waitpid returned Err({:?}", s)
            }

        },
    }
}

#[test]
fn test_wait() {
    // Grab FORK_MTX so wait doesn't reap a different test's child process
    #[allow(unused_variables)]
    let m = ::FORK_MTX.lock().expect("Mutex got poisoned by another test");

    // Safe: Child only calls `_exit`, which is signal-safe
    match fork().expect("Error: Fork Failed") {
        Child => unsafe { _exit(0) },
        Parent { child } => {
            let wait_status = wait();

            // just assert that (any) one child returns with WaitStatus::Exited
            assert_eq!(wait_status, Ok(WaitStatus::Exited(child, 0)));
        },
    }
}

#[test]
fn test_mkstemp() {
    let mut path = env::temp_dir();
    path.push("nix_tempfile.XXXXXX");

    let result = mkstemp(&path);
    match result {
        Ok((fd, path)) => {
            close(fd).unwrap();
            unlink(path.as_path()).unwrap();
        },
        Err(e) => panic!("mkstemp failed: {}", e)
    }
}

#[test]
fn test_mkstemp_directory() {
    // mkstemp should fail if a directory is given
    assert!(mkstemp(&env::temp_dir()).is_err());
}

#[test]
fn test_mkfifo() {
    let tempdir = TempDir::new("nix-test_mkfifo").unwrap();
    let mkfifo_fifo = tempdir.path().join("mkfifo_fifo");

    mkfifo(&mkfifo_fifo, Mode::S_IRUSR).unwrap();

    let stats = stat::stat(&mkfifo_fifo).unwrap();
    let typ = stat::SFlag::from_bits_truncate(stats.st_mode);
    assert!(typ == SFlag::S_IFIFO);
}

#[test]
fn test_mkfifo_directory() {
    // mkfifo should fail if a directory is given
    assert!(mkfifo(&env::temp_dir(), Mode::S_IRUSR).is_err());
}

#[test]
fn test_getpid() {
    let pid: ::libc::pid_t = getpid().into();
    let ppid: ::libc::pid_t = getppid().into();
    assert!(pid > 0);
    assert!(ppid > 0);
}

#[test]
fn test_getsid() {
    let none_sid: ::libc::pid_t = getsid(None).unwrap().into();
    let pid_sid: ::libc::pid_t = getsid(Some(getpid())).unwrap().into();
    assert!(none_sid > 0);
    assert!(none_sid == pid_sid);
}

#[cfg(any(target_os = "linux", target_os = "android"))]
mod linux_android {
    use nix::unistd::gettid;

    #[test]
    fn test_gettid() {
        let tid: ::libc::pid_t = gettid().into();
        assert!(tid > 0);
    }
}

#[test]
// `getgroups()` and `setgroups()` do not behave as expected on Apple platforms
#[cfg(not(any(target_os = "ios", target_os = "macos")))]
fn test_setgroups() {
    // Skip this test when not run as root as `setgroups()` requires root.
    if !Uid::current().is_root() {
        let stderr = std::io::stderr();
        let mut handle = stderr.lock();
        writeln!(handle, "test_setgroups requires root privileges. Skipping test.").unwrap();
        return;
    }

    #[allow(unused_variables)]
    let m = ::GROUPS_MTX.lock().expect("Mutex got poisoned by another test");

    // Save the existing groups
    let old_groups = getgroups().unwrap();

    // Set some new made up groups
    let groups = [Gid::from_raw(123), Gid::from_raw(456)];
    setgroups(&groups).unwrap();

    let new_groups = getgroups().unwrap();
    assert_eq!(new_groups, groups);

    // Revert back to the old groups
    setgroups(&old_groups).unwrap();
}

#[test]
// `getgroups()` and `setgroups()` do not behave as expected on Apple platforms
#[cfg(not(any(target_os = "ios", target_os = "macos")))]
fn test_initgroups() {
    // Skip this test when not run as root as `initgroups()` and `setgroups()`
    // require root.
    if !Uid::current().is_root() {
        let stderr = std::io::stderr();
        let mut handle = stderr.lock();
        writeln!(handle, "test_initgroups requires root privileges. Skipping test.").unwrap();
        return;
    }

    #[allow(unused_variables)]
    let m = ::GROUPS_MTX.lock().expect("Mutex got poisoned by another test");

    // Save the existing groups
    let old_groups = getgroups().unwrap();

    // It doesn't matter if the root user is not called "root" or if a user
    // called "root" doesn't exist. We are just checking that the extra,
    // made-up group, `123`, is set.
    // FIXME: Test the other half of initgroups' functionality: whether the
    // groups that the user belongs to are also set.
    let user = CString::new("root").unwrap();
    let group = Gid::from_raw(123);
    let group_list = getgrouplist(&user, group).unwrap();
    assert!(group_list.contains(&group));

    initgroups(&user, group).unwrap();

    let new_groups = getgroups().unwrap();
    assert_eq!(new_groups, group_list);

    // Revert back to the old groups
    setgroups(&old_groups).unwrap();
}

#[test]
fn test_mkdirat() {
    let tempdir = TempDir::new("nix-test_mkdirat").unwrap();
    let path = tempdir.path().join("test_path");

    let dirfd = fcntl::open(tempdir.path(),
                            fcntl::OFlag::empty(),
                            stat::Mode::empty());

    mkdirat(dirfd.unwrap(),
            &path.file_name(),
            stat::Mode::empty()).unwrap();

    assert!(path.exists());
}

#[test]
fn test_access() {
    let tempdir = TempDir::new("nix-test_mkdirat").unwrap();

    let dirfd = fcntl::open(tempdir.path().parent().unwrap(),
                            fcntl::OFlag::empty(),
                            stat::Mode::empty());

    // if succeed, permissions are or ok
    access(tempdir.path(), AccessMode::R_OK | AccessMode::X_OK | AccessMode::W_OK).unwrap();

    faccessat(dirfd.unwrap(),
              &tempdir.path().file_name(),
              AccessMode::R_OK | AccessMode::X_OK | AccessMode::W_OK,
              fcntl::AtFlags::empty()).unwrap();

}

macro_rules! execve_test_factory(
    ($test_name:ident, $syscall:ident, $exe: expr $(, $pathname:expr, $flags:expr)*) => (
    #[test]
    fn $test_name() {
        #[allow(unused_variables)]
        let m = ::FORK_MTX.lock().expect("Mutex got poisoned by another test");
        // The `exec`d process will write to `writer`, and we'll read that
        // data from `reader`.
        let (reader, writer) = pipe().unwrap();

        // Safe: Child calls `exit`, `dup`, `close` and the provided `exec*` family function.
        // NOTE: Technically, this makes the macro unsafe to use because you could pass anything.
        //       The tests make sure not to do that, though.
        match fork().unwrap() {
            Child => {
                // Close stdout.
                close(1).unwrap();
                // Make `writer` be the stdout of the new process.
                dup(writer).unwrap();
                // exec!
                $syscall(
                    $exe,
                    $(&CString::new($pathname).unwrap(), )*
                    &[CString::new(b"".as_ref()).unwrap(),
                      CString::new(b"-c".as_ref()).unwrap(),
                      CString::new(b"echo nix!!! && echo foo=$foo && echo baz=$baz"
                                   .as_ref()).unwrap()],
                    &[CString::new(b"foo=bar".as_ref()).unwrap(),
                      CString::new(b"baz=quux".as_ref()).unwrap()]
                    $(, $flags)*).unwrap();
            },
            Parent { child } => {
                // Wait for the child to exit.
                waitpid(child, None).unwrap();
                // Read 1024 bytes.
                let mut buf = [0u8; 1024];
                read(reader, &mut buf).unwrap();
                // It should contain the things we printed using `/bin/sh`.
                let string = String::from_utf8_lossy(&buf);
                assert!(string.contains("nix!!!"));
                assert!(string.contains("foo=bar"));
                assert!(string.contains("baz=quux"));
            }
        }
    }
    )
);

cfg_if!{
    if #[cfg(target_os = "android")] {
        execve_test_factory!(test_execve, execve, &CString::new("/system/bin/sh").unwrap());
        execve_test_factory!(test_fexecve, fexecve, File::open("/system/bin/sh").unwrap().into_raw_fd());
    } else if #[cfg(any(target_os = "freebsd",
                        target_os = "linux",
                        target_os = "netbsd",
                        target_os = "openbsd"))] {
        execve_test_factory!(test_execve, execve, &CString::new("/bin/sh").unwrap());
        execve_test_factory!(test_fexecve, fexecve, File::open("/bin/sh").unwrap().into_raw_fd());
    } else if #[cfg(any(target_os = "dragonfly",
                        target_os = "ios",
                        target_os = "macos"))] {
        execve_test_factory!(test_execve, execve, &CString::new("/bin/sh").unwrap());
        // No fexecve() on macos/ios and DragonFly.
    }
}

cfg_if!{
    if #[cfg(target_os = "android")] {
        use nix::fcntl::AtFlags;
        execve_test_factory!(test_execveat_empty, execveat, File::open("/system/bin/sh").unwrap().into_raw_fd(),
                             "", AtFlags::AT_EMPTY_PATH);
        execve_test_factory!(test_execveat_relative, execveat, File::open("/system/bin/").unwrap().into_raw_fd(),
                             "./sh", AtFlags::empty());
        execve_test_factory!(test_execveat_absolute, execveat, File::open("/").unwrap().into_raw_fd(),
                             "/system/bin/sh", AtFlags::empty());
    } else if #[cfg(all(target_os = "linux"), any(target_arch ="x86_64", target_arch ="x86"))] {
        use nix::fcntl::AtFlags;
        execve_test_factory!(test_execveat_empty, execveat, File::open("/bin/sh").unwrap().into_raw_fd(),
                             "", AtFlags::AT_EMPTY_PATH);
        execve_test_factory!(test_execveat_relative, execveat, File::open("/bin/").unwrap().into_raw_fd(),
                             "./sh", AtFlags::empty());
        execve_test_factory!(test_execveat_absolute, execveat, File::open("/").unwrap().into_raw_fd(),
                             "/bin/sh", AtFlags::empty());
    }
}

#[test]
fn test_fchdir() {
    // fchdir changes the process's cwd
    #[allow(unused_variables)]
    let m = ::CWD_MTX.lock().expect("Mutex got poisoned by another test");

    let tmpdir = TempDir::new("test_fchdir").unwrap();
    let tmpdir_path = tmpdir.path().canonicalize().unwrap();
    let tmpdir_fd = File::open(&tmpdir_path).unwrap().into_raw_fd();

    assert!(fchdir(tmpdir_fd).is_ok());
    assert_eq!(getcwd().unwrap(), tmpdir_path);

    assert!(close(tmpdir_fd).is_ok());
}

#[test]
fn test_getcwd() {
    // chdir changes the process's cwd
    #[allow(unused_variables)]
    let m = ::CWD_MTX.lock().expect("Mutex got poisoned by another test");

    let tmpdir = TempDir::new("test_getcwd").unwrap();
    let tmpdir_path = tmpdir.path().canonicalize().unwrap();
    assert!(chdir(&tmpdir_path).is_ok());
    assert_eq!(getcwd().unwrap(), tmpdir_path);

    // make path 500 chars longer so that buffer doubling in getcwd
    // kicks in.  Note: One path cannot be longer than 255 bytes
    // (NAME_MAX) whole path cannot be longer than PATH_MAX (usually
    // 4096 on linux, 1024 on macos)
    let mut inner_tmp_dir = tmpdir_path.to_path_buf();
    for _ in 0..5 {
        let newdir = iter::repeat("a").take(100).collect::<String>();
        inner_tmp_dir.push(newdir);
        assert!(mkdir(inner_tmp_dir.as_path(), Mode::S_IRWXU).is_ok());
    }
    assert!(chdir(inner_tmp_dir.as_path()).is_ok());
    assert_eq!(getcwd().unwrap(), inner_tmp_dir.as_path());
}

#[test]
fn test_lseek() {
    const CONTENTS: &[u8] = b"abcdef123456";
    let mut tmp = tempfile().unwrap();
    tmp.write_all(CONTENTS).unwrap();
    let tmpfd = tmp.into_raw_fd();

    let offset: off_t = 5;
    lseek(tmpfd, offset, Whence::SeekSet).unwrap();

    let mut buf = [0u8; 7];
    ::read_exact(tmpfd, &mut buf);
    assert_eq!(b"f123456", &buf);

    close(tmpfd).unwrap();
}

#[test]
fn test_unlinkat() {
    let tempdir = TempDir::new("nix-test_unlinkat").unwrap();
    let dirfd = fcntl::open(tempdir.path(),
                            fcntl::OFlag::empty(),
                            stat::Mode::empty());
    let file = tempdir.path().join("foo");
    File::create(&file).unwrap();

    unlinkat(dirfd.unwrap(),
            &file.file_name(),
            fcntl::AtFlags::empty()).unwrap();
    assert!(!file.exists());
}

#[cfg(any(target_os = "linux", target_os = "android"))]
#[test]
fn test_lseek64() {
    const CONTENTS: &[u8] = b"abcdef123456";
    let mut tmp = tempfile().unwrap();
    tmp.write_all(CONTENTS).unwrap();
    let tmpfd = tmp.into_raw_fd();

    lseek64(tmpfd, 5, Whence::SeekSet).unwrap();

    let mut buf = [0u8; 7];
    ::read_exact(tmpfd, &mut buf);
    assert_eq!(b"f123456", &buf);

    close(tmpfd).unwrap();
}

#[test]
fn test_fpathconf_limited() {
    let f = tempfile().unwrap();
    // AFAIK, PATH_MAX is limited on all platforms, so it makes a good test
    let path_max = fpathconf(f.as_raw_fd(), PathconfVar::PATH_MAX);
    assert!(path_max.expect("fpathconf failed").expect("PATH_MAX is unlimited") > 0);
}

#[test]
fn test_linkat() {
    let tempdir = TempDir::new("nix-test_linkat").unwrap();
    let src = tempdir.path().join("foo");
    let dst = tempdir.path().join("bar");
    File::create(&src).unwrap();

    let dirfd = fcntl::open(tempdir.path(),
                            fcntl::OFlag::empty(),
                            stat::Mode::empty());
    linkat(dirfd.unwrap(),
           &src.file_name(),
           dirfd.unwrap(),
           &dst.file_name(),
           fcntl::AtFlags::empty()).unwrap();
    assert!(dst.exists());
}

#[test]
fn test_link() {
    let tempdir = TempDir::new("nix-test_link").unwrap();
    let src = tempdir.path().join("foo");
    let dst = tempdir.path().join("bar");
    File::create(&src).unwrap();

    link(&src, &dst).unwrap();
    assert!(dst.exists());
}


#[test]
fn test_pathconf_limited() {
    // AFAIK, PATH_MAX is limited on all platforms, so it makes a good test
    let path_max = pathconf("/", PathconfVar::PATH_MAX);
    assert!(path_max.expect("pathconf failed").expect("PATH_MAX is unlimited") > 0);
}

#[test]
fn test_sysconf_limited() {
    // AFAIK, OPEN_MAX is limited on all platforms, so it makes a good test
    let open_max = sysconf(SysconfVar::OPEN_MAX);
    assert!(open_max.expect("sysconf failed").expect("OPEN_MAX is unlimited") > 0);
}

#[cfg(target_os = "freebsd")]
#[test]
fn test_sysconf_unsupported() {
    // I know of no sysconf variables that are unsupported everywhere, but
    // _XOPEN_CRYPT is unsupported on FreeBSD 11.0, which is one of the platforms
    // we test.
    let open_max = sysconf(SysconfVar::_XOPEN_CRYPT);
    assert!(open_max.expect("sysconf failed").is_none())
}

// Test that we can create a pair of pipes.  No need to verify that they pass
// data; that's the domain of the OS, not nix.
#[test]
fn test_pipe() {
    let (fd0, fd1) = pipe().unwrap();
    let m0 = stat::SFlag::from_bits_truncate(stat::fstat(fd0).unwrap().st_mode);
    // S_IFIFO means it's a pipe
    assert_eq!(m0, SFlag::S_IFIFO);
    let m1 = stat::SFlag::from_bits_truncate(stat::fstat(fd1).unwrap().st_mode);
    assert_eq!(m1, SFlag::S_IFIFO);
}

// pipe2(2) is the same as pipe(2), except it allows setting some flags.  Check
// that we can set a flag.
#[test]
fn test_pipe2() {
    let (fd0, fd1) = pipe2(OFlag::O_CLOEXEC).unwrap();
    let f0 = FdFlag::from_bits_truncate(fcntl(fd0, FcntlArg::F_GETFD).unwrap());
    assert!(f0.contains(FdFlag::FD_CLOEXEC));
    let f1 = FdFlag::from_bits_truncate(fcntl(fd1, FcntlArg::F_GETFD).unwrap());
    assert!(f1.contains(FdFlag::FD_CLOEXEC));
}

// Used in `test_alarm`.
static mut ALARM_CALLED: bool = false;

// Used in `test_alarm`.
pub extern fn alarm_signal_handler(raw_signal: libc::c_int) {
    assert_eq!(raw_signal, libc::SIGALRM, "unexpected signal: {}", raw_signal);
    unsafe { ALARM_CALLED = true };
}

#[test]
fn test_alarm() {
    let _m = ::SIGNAL_MTX.lock().expect("Mutex got poisoned by another test");

    let handler = SigHandler::Handler(alarm_signal_handler);
    let signal_action = SigAction::new(handler, SaFlags::SA_RESTART, SigSet::empty());
    let old_handler = unsafe {
        sigaction(Signal::SIGALRM, &signal_action)
            .expect("unable to set signal handler for alarm")
    };

    // Set an alarm.
    assert_eq!(alarm::set(60), None);

    // Overwriting an alarm should return the old alarm.
    assert_eq!(alarm::set(1), Some(60));

    // We should be woken up after 1 second by the alarm, so we'll sleep for 2
    // seconds to be sure.
    sleep(2);
    assert_eq!(unsafe { ALARM_CALLED }, true, "expected our alarm signal handler to be called");

    // Reset the signal.
    unsafe {
        sigaction(Signal::SIGALRM, &old_handler)
            .expect("unable to set signal handler for alarm");
    }
}

#[test]
fn test_canceling_alarm() {
    let _m = ::SIGNAL_MTX.lock().expect("Mutex got poisoned by another test");

    assert_eq!(alarm::cancel(), None);

    assert_eq!(alarm::set(60), None);
    assert_eq!(alarm::cancel(), Some(60));
}
