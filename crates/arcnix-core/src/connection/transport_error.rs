//! Classify a lost SSH transport separately from ordinary file/permission errors.
fn ssh_connection_error(error: &ssh2::Error) -> bool {
    matches!(
        error.code(),
        ssh2::ErrorCode::Session(-1 | -2 | -3 | -7 | -9 | -13 | -30 | -43 | -45)
    )
}
pub fn is_connection_error(error: &anyhow::Error) -> bool {
    error.chain().any(|cause| {
        if let Some(error) = cause.downcast_ref::<ssh2::Error>() {
            return ssh_connection_error(error);
        }
        cause.downcast_ref::<std::io::Error>().is_some_and(|error| {
            matches!(
                error.kind(),
                std::io::ErrorKind::ConnectionAborted
                    | std::io::ErrorKind::ConnectionReset
                    | std::io::ErrorKind::BrokenPipe
                    | std::io::ErrorKind::NotConnected
                    | std::io::ErrorKind::UnexpectedEof
                    | std::io::ErrorKind::TimedOut
            ) || error
                .get_ref()
                .and_then(|inner| inner.downcast_ref::<ssh2::Error>())
                .is_some_and(ssh_connection_error)
        })
    })
}

/// Recover the libssh2 code immediately after a stream read/write fails. ssh2's
/// std::io conversion retains only the message for ErrorKind::Other.
pub(crate) fn stream_error(session: &ssh2::Session, error: std::io::Error) -> anyhow::Error {
    let last_error = (error.kind() == std::io::ErrorKind::Other)
        .then(|| ssh2::Error::last_session_error(session))
        .flatten();
    recover_stream_error(error, last_error)
}

fn recover_stream_error(error: std::io::Error, last_error: Option<ssh2::Error>) -> anyhow::Error {
    if error.kind() == std::io::ErrorKind::Other
        && let Some(last_error) = last_error
        && ssh_connection_error(&last_error)
    {
        return last_error.into();
    }
    error.into()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn distinguishes_disconnect_from_permission_and_missing_files() {
        let broken = anyhow::Error::new(std::io::Error::from(std::io::ErrorKind::BrokenPipe))
            .context("SFTP upload");
        assert!(is_connection_error(&broken));
        assert!(is_connection_error(&anyhow::Error::new(
            ssh2::Error::from_errno(ssh2::ErrorCode::Session(-43))
        )));
        assert!(!is_connection_error(&anyhow::Error::new(
            ssh2::Error::from_errno(ssh2::ErrorCode::SFTP(2))
        )));
        assert!(!is_connection_error(&anyhow::Error::new(
            std::io::Error::from(std::io::ErrorKind::PermissionDenied)
        )));
        assert!(!is_connection_error(&anyhow::anyhow!(
            "Destination already exists"
        )));
    }
    #[test]
    fn recovers_transport_codes_erased_by_ssh2_stream_io_conversion() {
        let lost = || ssh2::Error::new(ssh2::ErrorCode::Session(-43), "transport read");
        let erased: std::io::Error = lost().into();
        assert_eq!(erased.kind(), std::io::ErrorKind::Other);
        let erased_again: std::io::Error = lost().into();
        assert!(!is_connection_error(&anyhow::Error::new(erased_again)));
        assert!(is_connection_error(&recover_stream_error(
            erased,
            Some(lost())
        )));
        assert!(is_connection_error(&anyhow::Error::new(
            std::io::Error::other(lost())
        )));
        let missing = ssh2::Error::from_errno(ssh2::ErrorCode::SFTP(2));
        assert!(!is_connection_error(&recover_stream_error(
            std::io::Error::other("not found"),
            Some(missing)
        )));
        assert!(!is_connection_error(&recover_stream_error(
            std::io::Error::from(std::io::ErrorKind::PermissionDenied),
            Some(lost())
        )));
    }
}
