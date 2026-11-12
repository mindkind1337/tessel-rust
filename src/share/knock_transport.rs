//! Non-consuming liveness probe during approval, on the connection's exclusive host worker.
use std::{io::ErrorKind, net::TcpStream};
// This caller owns a blocking socket and performs no concurrent socket I/O.
// Restore blocking mode before rustls can read or write another frame.
pub(super) fn disconnected(stream: &TcpStream) -> bool {
    if stream.set_nonblocking(true).is_err() {
        return true;
    }
    let probe = stream.peek(&mut [0u8; 1]);
    if stream.set_nonblocking(false).is_err() {
        return true;
    }
    match probe {
        Ok(0) => true,
        Ok(_) => false,
        Err(error) => !matches!(error.kind(), ErrorKind::WouldBlock | ErrorKind::Interrupted),
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::{Read, Write},
        net::{Shutdown, TcpListener},
        time::Duration,
    };
    #[test]
    fn knock_probe_keeps_idle_and_buffered_connections_without_consuming_bytes() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let mut client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (mut host, _) = listener.accept().unwrap();
        assert!(!disconnected(&host));
        client.write_all(b"probe").unwrap();
        assert!(!disconnected(&host));
        host.set_read_timeout(Some(Duration::from_secs(1))).unwrap();
        let mut bytes = [0u8; 5];
        host.read_exact(&mut bytes).unwrap();
        assert_eq!(&bytes, b"probe");
        assert!(!disconnected(&host));
        client.shutdown(Shutdown::Both).unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(1);
        while !disconnected(&host) {
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}
