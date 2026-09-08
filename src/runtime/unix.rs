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
) -> io::Result<()> {
    use std::process::{Command, Stdio};
    let mut child = Command::new(executable)
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
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
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
