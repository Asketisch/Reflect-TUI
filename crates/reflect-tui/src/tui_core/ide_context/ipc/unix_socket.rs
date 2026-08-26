//! IDE 上下文 Unix socket 底层连接/校验辅助。从 ipc.rs 抽出。

use super::*;

pub(super) struct UnixDeadlineStream {
    stream: std::os::unix::net::UnixStream,
    deadline: Instant,
}

#[cfg(unix)]
impl UnixDeadlineStream {
    pub(super) fn connect(socket_path: PathBuf, deadline: Instant) -> std::io::Result<Self> {
        let stream = connect_unix_stream_before_deadline(&socket_path, deadline)?;
        validate_unix_peer_owner(&stream)?;
        Ok(Self::new(stream, deadline))
    }

    pub(crate) fn new(stream: std::os::unix::net::UnixStream, deadline: Instant) -> Self {
        Self { stream, deadline }
    }

    pub(super) fn set_deadline(&mut self, deadline: Instant) {
        self.deadline = deadline;
    }

    fn wait_for_ready(&self, events: libc::c_short) -> std::io::Result<()> {
        use std::os::fd::AsRawFd;

        wait_for_fd_ready(self.stream.as_raw_fd(), events, self.deadline)
    }
}

#[cfg(unix)]
pub(super) fn connect_unix_stream_before_deadline(
    socket_path: &Path,
    deadline: Instant,
) -> std::io::Result<std::os::unix::net::UnixStream> {
    use std::os::fd::AsRawFd;
    use std::os::fd::FromRawFd;
    use std::os::fd::IntoRawFd;
    use std::os::fd::OwnedFd;

    validate_unix_socket_path(socket_path)?;
    let (addr, addr_len) = unix_socket_addr(socket_path)?;
    let fd = unsafe { libc::socket(libc::AF_UNIX, libc::SOCK_STREAM, 0) };
    if fd < 0 {
        return Err(std::io::Error::last_os_error());
    }
    let fd = unsafe { OwnedFd::from_raw_fd(fd) };
    set_fd_close_on_exec(fd.as_raw_fd())?;
    set_fd_nonblocking(fd.as_raw_fd())?;

    let result = unsafe {
        libc::connect(
            fd.as_raw_fd(),
            &addr as *const libc::sockaddr_un as *const libc::sockaddr,
            addr_len,
        )
    };
    if result != 0 {
        let error = std::io::Error::last_os_error();
        if !is_in_progress_connect_error(&error) {
            return Err(error);
        }

        wait_for_fd_ready(fd.as_raw_fd(), libc::POLLOUT, deadline)?;
        let socket_error = socket_error(fd.as_raw_fd())?;
        if socket_error != 0 {
            return Err(std::io::Error::from_raw_os_error(socket_error));
        }
    }

    Ok(unsafe { std::os::unix::net::UnixStream::from_raw_fd(fd.into_raw_fd()) })
}

#[cfg(unix)]
pub(super) fn unix_socket_addr(
    socket_path: &Path,
) -> std::io::Result<(libc::sockaddr_un, libc::socklen_t)> {
    use std::os::unix::ffi::OsStrExt;

    let path_bytes = socket_path.as_os_str().as_bytes();
    if path_bytes.contains(&0) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "IDE context Unix socket path contains a nul byte",
        ));
    }

    let mut addr = unsafe { std::mem::zeroed::<libc::sockaddr_un>() };
    if path_bytes.len() >= addr.sun_path.len() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "IDE context Unix socket path is too long",
        ));
    }

    addr.sun_family = libc::AF_UNIX as libc::sa_family_t;
    for (slot, byte) in addr.sun_path.iter_mut().zip(path_bytes) {
        *slot = *byte as libc::c_char;
    }

    let addr_len =
        std::mem::size_of::<libc::sockaddr_un>() - addr.sun_path.len() + path_bytes.len() + 1;
    #[cfg(any(
        target_os = "macos",
        target_os = "freebsd",
        target_os = "openbsd",
        target_os = "netbsd",
        target_os = "dragonfly"
    ))]
    {
        addr.sun_len = u8::try_from(addr_len).map_err(|_| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "IDE context Unix socket address is too long",
            )
        })?;
    }

    let addr_len = libc::socklen_t::try_from(addr_len).map_err(|_| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "IDE context Unix socket address is too long",
        )
    })?;
    Ok((addr, addr_len))
}

#[cfg(unix)]
pub(super) fn set_fd_close_on_exec(fd: libc::c_int) -> std::io::Result<()> {
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFD) };
    if flags < 0 {
        return Err(std::io::Error::last_os_error());
    }
    let result = unsafe { libc::fcntl(fd, libc::F_SETFD, flags | libc::FD_CLOEXEC) };
    if result < 0 {
        return Err(std::io::Error::last_os_error());
    }

    Ok(())
}

#[cfg(unix)]
pub(super) fn set_fd_nonblocking(fd: libc::c_int) -> std::io::Result<()> {
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags < 0 {
        return Err(std::io::Error::last_os_error());
    }
    let result = unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) };
    if result < 0 {
        return Err(std::io::Error::last_os_error());
    }

    Ok(())
}

#[cfg(unix)]
pub(super) fn is_in_progress_connect_error(error: &std::io::Error) -> bool {
    matches!(
        error.raw_os_error(),
        Some(code)
            if code == libc::EINPROGRESS
                || code == libc::EALREADY
                || code == libc::EWOULDBLOCK
                || code == libc::EINTR
    )
}

#[cfg(unix)]
pub(super) fn socket_error(fd: libc::c_int) -> std::io::Result<libc::c_int> {
    let mut socket_error = 0;
    let mut socket_error_len = libc::socklen_t::try_from(std::mem::size_of::<libc::c_int>())
        .map_err(|_| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "invalid socket error length",
            )
        })?;
    let result = unsafe {
        libc::getsockopt(
            fd,
            libc::SOL_SOCKET,
            libc::SO_ERROR,
            &mut socket_error as *mut _ as *mut libc::c_void,
            &mut socket_error_len,
        )
    };
    if result != 0 {
        return Err(std::io::Error::last_os_error());
    }

    Ok(socket_error)
}

#[cfg(unix)]
pub(super) fn remaining_timeout(deadline: Instant) -> std::io::Result<Duration> {
    deadline
        .checked_duration_since(Instant::now())
        .filter(|duration| !duration.is_zero())
        .ok_or_else(deadline_timeout_io_error)
}

#[cfg(unix)]
pub(super) fn remaining_timeout_ms(deadline: Instant) -> std::io::Result<libc::c_int> {
    let millis = remaining_timeout(deadline)?.as_millis().max(1);
    Ok(libc::c_int::try_from(millis).unwrap_or(libc::c_int::MAX))
}

#[cfg(unix)]
pub(super) fn wait_for_fd_ready(
    fd: libc::c_int,
    events: libc::c_short,
    deadline: Instant,
) -> std::io::Result<()> {
    loop {
        // 把截止时间处理保留在用户空间。某些 macOS Unix socket 环境会拒绝
        // SO_RCVTIMEO/SO_SNDTIMEO，而 poll 对我们的请求级超时始终有效。
        let mut poll_fd = libc::pollfd {
            fd,
            events,
            revents: 0,
        };
        let result = unsafe { libc::poll(&mut poll_fd, 1, remaining_timeout_ms(deadline)?) };
        if result == 0 {
            return Err(deadline_timeout_io_error());
        }
        if result < 0 {
            let error = std::io::Error::last_os_error();
            if error.kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            return Err(error);
        }
        if poll_fd.revents & libc::POLLNVAL != 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "invalid IDE context Unix socket",
            ));
        }
        if poll_fd.revents & (events | libc::POLLERR | libc::POLLHUP) != 0 {
            return Ok(());
        }
    }
}

#[cfg(unix)]
impl std::io::Read for UnixDeadlineStream {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }

        loop {
            self.wait_for_ready(libc::POLLIN)?;
            match self.stream.read(buf) {
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
                result => return result,
            }
        }
    }
}

#[cfg(unix)]
impl std::io::Write for UnixDeadlineStream {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }

        loop {
            self.wait_for_ready(libc::POLLOUT)?;
            match self.stream.write(buf) {
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
                result => return result,
            }
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.wait_for_ready(libc::POLLOUT)?;
        self.stream.flush()
    }
}

#[cfg(unix)]
pub(super) fn validate_unix_socket_path(socket_path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::FileTypeExt;
    use std::os::unix::fs::MetadataExt;
    use std::os::unix::fs::PermissionsExt;

    let uid = unsafe { libc::getuid() };
    let parent = socket_path.parent().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "IDE context socket has no parent directory",
        )
    })?;
    let parent_metadata = std::fs::symlink_metadata(parent)?;
    if !parent_metadata.is_dir() || parent_metadata.uid() != uid {
        return Err(permission_denied_io_error(
            "IDE context socket directory is not owned by the current user",
        ));
    }
    if parent_metadata.permissions().mode() & 0o022 != 0 {
        return Err(permission_denied_io_error(
            "IDE context socket directory is writable by other users",
        ));
    }

    let socket_metadata = std::fs::symlink_metadata(socket_path)?;
    if !socket_metadata.file_type().is_socket() || socket_metadata.uid() != uid {
        return Err(permission_denied_io_error(
            "IDE context socket is not owned by the current user",
        ));
    }

    Ok(())
}

#[cfg(any(target_os = "linux", target_os = "android"))]
pub(super) fn validate_unix_peer_owner(
    stream: &std::os::unix::net::UnixStream,
) -> std::io::Result<()> {
    use std::os::fd::AsRawFd;

    let mut credentials = unsafe { std::mem::zeroed::<libc::ucred>() };
    let mut credentials_len: libc::socklen_t =
        std::mem::size_of::<libc::ucred>().try_into().map_err(|_| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "invalid peer credential length",
            )
        })?;
    let result = unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            &mut credentials as *mut _ as *mut libc::c_void,
            &mut credentials_len,
        )
    };
    if result != 0 {
        return Err(std::io::Error::last_os_error());
    }

    ensure_peer_uid_matches_current_user(credentials.uid)
}

#[cfg(any(
    target_os = "macos",
    target_os = "freebsd",
    target_os = "openbsd",
    target_os = "netbsd",
    target_os = "dragonfly"
))]
pub(super) fn validate_unix_peer_owner(
    stream: &std::os::unix::net::UnixStream,
) -> std::io::Result<()> {
    use std::os::fd::AsRawFd;

    let mut peer_uid: libc::uid_t = 0;
    let mut peer_gid: libc::gid_t = 0;
    let result = unsafe { libc::getpeereid(stream.as_raw_fd(), &mut peer_uid, &mut peer_gid) };
    if result != 0 {
        return Err(std::io::Error::last_os_error());
    }

    ensure_peer_uid_matches_current_user(peer_uid)
}

#[cfg(all(
    unix,
    not(any(
        target_os = "linux",
        target_os = "android",
        target_os = "macos",
        target_os = "freebsd",
        target_os = "openbsd",
        target_os = "netbsd",
        target_os = "dragonfly"
    ))
))]
pub(super) fn validate_unix_peer_owner(
    _stream: &std::os::unix::net::UnixStream,
) -> std::io::Result<()> {
    Ok(())
}

#[cfg(unix)]
pub(super) fn ensure_peer_uid_matches_current_user(peer_uid: libc::uid_t) -> std::io::Result<()> {
    if peer_uid != unsafe { libc::getuid() } {
        return Err(permission_denied_io_error(
            "IDE context provider is not owned by the current user",
        ));
    }

    Ok(())
}
