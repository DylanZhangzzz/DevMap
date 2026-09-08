use super::transport::invalid;
use std::{
    fs, io,
    os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt},
    path::Path,
};
use tokio::net::{UnixListener, UnixStream};
pub type Client = UnixStream;
pub struct Listener(UnixListener);
pub fn user_id() -> io::Result<String> {
    Ok(unsafe { libc::geteuid() }.to_string())
}
pub fn private_dir(path: &Path) -> io::Result<()> {
    match fs::DirBuilder::new().mode(0o700).create(path) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
        Err(e) => return Err(e),
    }
    validate(path, true)
}
pub fn validate(path: &Path, directory: bool) -> io::Result<()> {
    let m = fs::symlink_metadata(path)?;
    if m.file_type().is_symlink()
        || m.is_dir() != directory
        || m.uid() != unsafe { libc::geteuid() }
        || m.mode() & 0o077 != 0
    {
        return Err(invalid("runtime path is not private to current user"));
    }
    Ok(())
}
pub fn lock_file(path: &Path) -> io::Result<fs::File> {
    let f = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)?;
    validate(path, false)?;
    if !f.metadata()?.is_file() {
        return Err(invalid("runtime lock is not a regular file"));
    }
    Ok(f)
}
fn authenticate(s: &UnixStream) -> io::Result<()> {
    if s.peer_cred()?.uid() != unsafe { libc::geteuid() } {
        return Err(invalid("runtime peer UID differs"));
    }
    Ok(())
}
pub async fn connect(endpoint: &str) -> io::Result<Client> {
    let s = UnixStream::connect(endpoint).await?;
    authenticate(&s)?;
    Ok(s)
}
impl Listener {
    pub fn bind(endpoint: &str) -> io::Result<Self> {
        let path = Path::new(endpoint);
        // Called only while holding the lifetime owner lock; never unlink an unowned file.
        if path.symlink_metadata().is_ok() {
            use std::os::unix::fs::FileTypeExt;
            validate(path, false)?;
            if !path.symlink_metadata()?.file_type().is_socket() {
                return Err(invalid("runtime endpoint is not a socket"));
            }
            fs::remove_file(path)?;
        }
        let listener = UnixListener::bind(path)?;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
        Ok(Self(listener))
    }
    pub async fn accept(&mut self) -> io::Result<UnixStream> {
        let (s, _) = self.0.accept().await?;
        authenticate(&s)?;
        Ok(s)
    }
}

pub fn spawn_owner(
    executable: &Path,
    source: &Path,
    instance: &str,
    idle_seconds: u64,
) -> io::Result<SpawnedOwner> {
    use std::process::{Command, Stdio};
    let child = Command::new(executable)
        .arg("runtime")
        .arg("--source")
        .arg(source)
        .arg("--owner")
        .arg("--instance")
        .arg(instance)
        .arg("--idle-seconds")
        .arg(idle_seconds.to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    Ok(SpawnedOwner(Some(child)))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn private_directory_rejects_world_permissions_and_symlinks() {
        let temp = tempfile::tempdir().unwrap();
        let private = temp.path().join("private");
        private_dir(&private).unwrap();
        fs::set_permissions(&private, fs::Permissions::from_mode(0o755)).unwrap();
        assert!(private_dir(&private).is_err());
        let alias = temp.path().join("alias");
        std::os::unix::fs::symlink(&private, &alias).unwrap();
        assert!(private_dir(&alias).is_err());
    }
}

pub fn prepare_identity(command: &mut tokio::process::Command) {
    command.process_group(0);
}
pub struct IdentityTree(u32);
impl IdentityTree {
    pub fn attach(child: &tokio::process::Child) -> io::Result<Self> {
        Ok(Self(
            child
                .id()
                .ok_or_else(|| invalid("identity child PID missing"))?,
        ))
    }
}
impl Drop for IdentityTree {
    fn drop(&mut self) {
        unsafe {
            libc::kill(-(self.0 as i32), libc::SIGKILL);
        }
    }
}

pub fn validate_parent(path: &Path) -> io::Result<()> {
    let m = fs::symlink_metadata(path)?;
    let current = unsafe { libc::geteuid() };
    if !m.is_dir() || m.file_type().is_symlink() || (m.uid() != current && m.uid() != 0) {
        return Err(invalid("runtime parent ownership is untrusted"));
    }
    // Root-owned sticky /tmp permits creation without permitting foreign users
    // to rename/remove this user's private child. Ordinary writable parents do not.
    if m.mode() & 0o022 != 0 && !(m.uid() == 0 && m.mode() & 0o1000 != 0) {
        return Err(invalid("runtime parent permits untrusted mutation"));
    }
    Ok(())
}

impl IdentityTree {
    pub async fn terminate_and_wait(&self) -> io::Result<()> {
        // Called before reaping the group leader, retaining the owned PGID.
        if unsafe { libc::kill(-(self.0 as i32), libc::SIGKILL) } != 0 {
            let error = io::Error::last_os_error();
            if error.raw_os_error() != Some(libc::ESRCH) {
                return Err(error);
            }
        }
        Ok(())
    }
}

pub struct SpawnedOwner(Option<std::process::Child>);
impl SpawnedOwner {
    pub fn detach(mut self) {
        if let Some(mut child) = self.0.take() {
            std::thread::spawn(move || {
                let _ = child.wait();
            });
        }
    }
}
impl Drop for SpawnedOwner {
    fn drop(&mut self) {
        if let Some(child) = self.0.as_mut() {
            let _ = child.kill();
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(1);
            while matches!(child.try_wait(), Ok(None)) && std::time::Instant::now() < deadline {
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
        }
    }
}

pub fn validate_chain(path: &Path) -> io::Result<()> {
    for ancestor in path.ancestors() {
        validate_parent(ancestor)?;
    }
    Ok(())
}

#[cfg(test)]
mod ancestor_tests {
    use super::*;
    #[test]
    fn private_leaf_does_not_hide_a_writable_ancestor() {
        let temp = tempfile::tempdir().unwrap();
        let grandparent = temp.path().join("grandparent");
        private_dir(&grandparent).unwrap();
        let parent = grandparent.join("parent");
        private_dir(&parent).unwrap();
        let leaf = parent.join("leaf");
        private_dir(&leaf).unwrap();
        fs::set_permissions(&grandparent, fs::Permissions::from_mode(0o777)).unwrap();
        validate_parent(&leaf).unwrap();
        assert!(validate_chain(&leaf).is_err());
    }
    #[test]
    fn system_sticky_temporary_chain_is_accepted() {
        validate_chain(&fs::canonicalize("/tmp").unwrap()).unwrap();
    }
}
