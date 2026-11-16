//! Private bounded packet codec for the local team broker, not an HTTP endpoint.
use serde::{Serialize, de::DeserializeOwned};
use std::io::{self, Read, Write};
use std::net::TcpStream;
use std::time::Instant;

pub(crate) const MAX_BYTES: usize = 1024 * 1024;

struct Buffer(Vec<u8>);
impl Write for Buffer {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() > MAX_BYTES.saturating_sub(self.0.len()) {
            return Err(io::Error::other("Team packet exceeds limit"));
        }
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

pub(crate) fn encode<T: Serialize>(value: &T) -> Result<Vec<u8>, String> {
    let mut buffer = Buffer(Vec::new());
    serde_json::to_writer(&mut buffer, value)
        .map_err(|_| "Team packet cannot be encoded within limit".to_owned())?;
    Ok(buffer.0)
}

pub(crate) fn decode<T: DeserializeOwned>(bytes: &[u8]) -> Result<T, String> {
    if bytes.is_empty() || bytes.len() > MAX_BYTES {
        return Err("Invalid team packet length".into());
    }
    serde_json::from_slice(bytes).map_err(|_| "Invalid team packet".into())
}

fn remaining(deadline: Instant) -> io::Result<std::time::Duration> {
    deadline
        .checked_duration_since(Instant::now())
        .filter(|duration| !duration.is_zero())
        .ok_or_else(|| io::Error::new(io::ErrorKind::TimedOut, "Team packet deadline expired"))
}

fn running(deadline: Instant, keep_running: &impl Fn() -> bool) -> io::Result<()> {
    if !keep_running() {
        return Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "Team packet cancelled",
        ));
    }
    remaining(deadline).map(|_| ())
}

fn wait_ready(deadline: Instant) -> io::Result<()> {
    std::thread::sleep(remaining(deadline)?.min(std::time::Duration::from_millis(5)));
    Ok(())
}

fn read_exact(
    stream: &mut TcpStream,
    mut bytes: &mut [u8],
    deadline: Instant,
    keep_running: &impl Fn() -> bool,
) -> io::Result<()> {
    while !bytes.is_empty() {
        running(deadline, keep_running)?;
        match stream.read(bytes) {
            Ok(0) => {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "Incomplete team packet",
                ));
            }
            Ok(count) => bytes = &mut bytes[count..],
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => wait_ready(deadline)?,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        }
    }
    running(deadline, keep_running)
}

fn write_all(
    stream: &mut TcpStream,
    mut bytes: &[u8],
    deadline: Instant,
    keep_running: &impl Fn() -> bool,
) -> io::Result<()> {
    while !bytes.is_empty() {
        running(deadline, keep_running)?;
        match stream.write(bytes) {
            Ok(0) => {
                return Err(io::Error::new(
                    io::ErrorKind::WriteZero,
                    "Incomplete team packet",
                ));
            }
            Ok(count) => bytes = &bytes[count..],
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => wait_ready(deadline)?,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        }
    }
    running(deadline, keep_running)
}

#[cfg(test)]
pub(crate) fn read_packet(stream: &mut TcpStream, deadline: Instant) -> Result<Vec<u8>, String> {
    read_packet_while(stream, deadline, || true)
}

pub(crate) fn read_packet_while(
    stream: &mut TcpStream,
    deadline: Instant,
    keep_running: impl Fn() -> bool,
) -> Result<Vec<u8>, String> {
    stream
        .set_nonblocking(true)
        .map_err(|_| "Cannot configure team packet input")?;
    let mut header = [0u8; 4];
    read_exact(stream, &mut header, deadline, &keep_running)
        .map_err(|_| "Team packet header unavailable".to_owned())?;
    let len = u32::from_be_bytes(header) as usize;
    if len == 0 || len > MAX_BYTES {
        return Err("Invalid team packet length".into());
    }
    let mut bytes = vec![0; len];
    read_exact(stream, &mut bytes, deadline, &keep_running)
        .map_err(|_| "Team packet incomplete or timed out".to_owned())?;
    Ok(bytes)
}

#[cfg(test)]
pub(crate) fn write_packet(
    stream: &mut TcpStream,
    bytes: &[u8],
    deadline: Instant,
) -> Result<(), String> {
    write_packet_while(stream, bytes, deadline, || true)
}

pub(crate) fn write_packet_while(
    stream: &mut TcpStream,
    bytes: &[u8],
    deadline: Instant,
    keep_running: impl Fn() -> bool,
) -> Result<(), String> {
    if bytes.is_empty() || bytes.len() > MAX_BYTES {
        return Err("Invalid team packet length".into());
    }
    stream
        .set_nonblocking(true)
        .map_err(|_| "Cannot configure team packet output")?;
    write_all(
        stream,
        &(bytes.len() as u32).to_be_bytes(),
        deadline,
        &keep_running,
    )
    .and_then(|()| write_all(stream, bytes, deadline, &keep_running))
    .map_err(|_| "Team packet write failed or timed out; outcome may be unknown".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{Ipv4Addr, TcpListener};
    use std::time::Duration;

    fn pair() -> (TcpStream, TcpStream) {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let sender = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (receiver, _) = listener.accept().unwrap();
        (sender, receiver)
    }

    #[test]
    fn packet_limits_reject_bad_lengths_encoding_and_partial_data() {
        assert!(encode(&"x".repeat(MAX_BYTES)).is_err());
        assert!(decode::<serde_json::Value>(&[0xff]).is_err());
        assert!(decode::<serde_json::Value>(&vec![b'x'; MAX_BYTES + 1]).is_err());
        for length in [0, MAX_BYTES as u32 + 1] {
            let (mut sender, mut receiver) = pair();
            sender.write_all(&length.to_be_bytes()).unwrap();
            assert!(read_packet(&mut receiver, Instant::now() + Duration::from_secs(1)).is_err());
        }
        let (mut sender, mut receiver) = pair();
        sender.write_all(&[0, 0]).unwrap();
        let start = Instant::now();
        assert!(read_packet(&mut receiver, start + Duration::from_millis(60)).is_err());
        assert!(start.elapsed() < Duration::from_secs(1));
        let (mut sender, mut receiver) = pair();
        sender.write_all(&5u32.to_be_bytes()).unwrap();
        sender.write_all(b"ab").unwrap();
        assert!(read_packet(&mut receiver, Instant::now() + Duration::from_millis(60)).is_err());
    }

    #[test]
    fn trickled_input_cannot_extend_the_absolute_packet_deadline() {
        let (mut sender, mut receiver) = pair();
        sender.write_all(&100u32.to_be_bytes()).unwrap();
        let worker = std::thread::spawn(move || {
            for _ in 0..12 {
                if sender.write_all(b"x").is_err() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
        });
        let start = Instant::now();
        assert!(read_packet(&mut receiver, start + Duration::from_millis(70)).is_err());
        assert!(start.elapsed() < Duration::from_millis(500));
        drop(receiver);
        worker.join().unwrap();
    }

    #[test]
    fn a_nonreading_peer_cannot_extend_writes_past_the_absolute_deadline() {
        let (mut sender, _receiver) = pair();
        let bytes = vec![b'x'; MAX_BYTES];
        let start = Instant::now();
        let deadline = start + Duration::from_millis(100);
        let mut refused = false;
        for _ in 0..100 {
            if write_packet(&mut sender, &bytes, deadline).is_err() {
                refused = true;
                break;
            }
        }
        assert!(refused);
        assert!(start.elapsed() < Duration::from_secs(1));
    }
}
