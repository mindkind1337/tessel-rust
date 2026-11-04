//! Where the daemon lives: `~/.tessel-server/host/<version>/`, a new directory of its own (not
//! the JavaScript helper's `run/`), private to the user, with its lock, socket and log.
use std::fs::{self, DirBuilder, File, OpenOptions};
use std::io;
use std::os::unix::fs::{DirBuilderExt, FileTypeExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::os::unix::io::AsRawFd;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};

/// `sun_path` holds 108 bytes, with the final NUL.
const SOCKET_PATH_MAX: usize = 107;

pub struct Runtime {
    pub dir: PathBuf,
    pub socket: PathBuf,
    pub lock: PathBuf,
    pub log: PathBuf,
}

fn euid() -> u32 {
    // SAFETY: geteuid has no preconditions and cannot fail.
    unsafe { libc::geteuid() }
}

fn refused(what: &str, path: &Path) -> io::Error {
    io::Error::new(
        io::ErrorKind::PermissionDenied,
        format!("{what}: {}", path.display()),
    )
}

/// A directory that is really a directory (never a symbolic link), owned by this user. `shared`:
/// `~/.tessel-server`, which the JavaScript helper uses too, needs only to be writable by nobody
/// else; the daemon's own directories are exactly private. Nobody else can then replace anything
/// below them, which is what makes using these paths afterwards safe.
fn private_dir(path: &Path, shared: bool) -> io::Result<()> {
    if let Err(error) = DirBuilder::new().mode(0o700).create(path)
        && error.kind() != io::ErrorKind::AlreadyExists
    {
        return Err(error);
    }
    let meta = fs::symlink_metadata(path)?;
    let mode = meta.mode() & 0o7777;
    let open = if shared { mode & 0o022 } else { mode & 0o077 };
    if !meta.file_type().is_dir() || meta.uid() != euid() || open != 0 {
        return Err(refused("unsafe runtime directory", path));
    }
    Ok(())
}

pub fn runtime() -> io::Result<Runtime> {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .filter(|home| home.is_absolute())
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "HOME is not set"))?;
    let base = home.join(".tessel-server");
    private_dir(&base, true)?;
    let host = base.join("host");
    private_dir(&host, false)?;
    let dir = host.join(crate::VERSION);
    private_dir(&dir, false)?;
    let socket = dir.join("daemon.sock");
    if socket.as_os_str().len() > SOCKET_PATH_MAX {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "socket path too long",
        ));
    }
    Ok(Runtime {
        lock: dir.join("daemon.lock"),
        log: dir.join("daemon.log"),
        socket,
        dir,
    })
}

/// Held for the daemon's whole life: one daemon per user and version.
pub struct Lock(#[allow(dead_code)] File);

/// None: another daemon holds it.
pub fn try_lock(runtime: &Runtime) -> io::Result<Option<Lock>> {
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(&runtime.lock)?;
    // SAFETY: a valid descriptor owned by `file`.
    if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0 {
        return Ok(Some(Lock(file)));
    }
    let error = io::Error::last_os_error();
    if error.raw_os_error() == Some(libc::EWOULDBLOCK) {
        Ok(None)
    } else {
        Err(error)
    }
}

/// The socket, made by the lock's holder: a leftover socket of a daemon that died is removed;
/// anything else at that path is refused.
pub fn bind(runtime: &Runtime, _held: &Lock) -> io::Result<UnixListener> {
    match fs::symlink_metadata(&runtime.socket) {
        Ok(meta) if meta.file_type().is_socket() => fs::remove_file(&runtime.socket)?,
        Ok(_) => return Err(refused("not a socket", &runtime.socket)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    let listener = UnixListener::bind(&runtime.socket)?;
    fs::set_permissions(&runtime.socket, fs::Permissions::from_mode(0o600))?;
    Ok(listener)
}

/// The user at the other end of a connection, from the kernel.
pub fn peer_uid(stream: &UnixStream) -> io::Result<u32> {
    // SAFETY: ucred is plain data; getsockopt writes at most `len` bytes into it.
    let mut cred: libc::ucred = unsafe { std::mem::zeroed() };
    let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    let result = unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            (&mut cred as *mut libc::ucred).cast(),
            &mut len,
        )
    };
    if result == 0 {
        Ok(cred.uid)
    } else {
        Err(io::Error::last_os_error())
    }
}

pub fn is_this_user(uid: u32) -> bool {
    uid == euid()
}

pub fn random_bytes<const N: usize>() -> io::Result<[u8; N]> {
    use std::io::Read;
    let mut bytes = [0u8; N];
    File::open("/dev/urandom")?.read_exact(&mut bytes)?;
    Ok(bytes)
}
