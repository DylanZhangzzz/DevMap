//! Unix owns an armed process group until signal delivery, before leader reap.
//! Descendants that call setsid can escape; non-child zombies cannot be reaped here.
use std::{io, time::Duration};
pub(super) fn prepare(command: &mut tokio::process::Command) {
    command.process_group(0);
}
pub(super) struct Tree {
    pgid: u32,
    armed: bool,
}
impl Tree {
    pub(super) fn attach(child: &tokio::process::Child) -> io::Result<Self> {
        Ok(Self {
            pgid: child
                .id()
                .ok_or_else(|| io::Error::other("Git child has no PID"))?,
            armed: true,
        })
    }
    pub(super) async fn terminate_and_wait(&mut self) -> io::Result<()> {
        if !self.armed {
            return Ok(());
        }
        if unsafe { libc::kill(-(self.pgid as i32), libc::SIGKILL) } != 0 {
            let error = io::Error::last_os_error();
            if error.raw_os_error() != Some(libc::ESRCH) {
                return Err(error);
            }
        }
        self.armed = false; // Never signal this numeric group again after leader reap.
        Ok(())
    }
}
impl Drop for Tree {
    fn drop(&mut self) {
        if self.armed {
            unsafe {
                libc::kill(-(self.pgid as i32), libc::SIGKILL);
            }
        }
    }
}
pub(super) async fn observe_exit(pid: u32) -> io::Result<()> {
    loop {
        let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
        if unsafe {
            libc::waitid(
                libc::P_PID,
                pid as libc::id_t,
                &mut info,
                libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
            )
        } != 0
        {
            return Err(io::Error::last_os_error());
        }
        if unsafe { info.si_pid() } != 0 {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}
