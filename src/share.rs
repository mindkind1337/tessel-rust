//! Read-only native LAN sharing. TLS certificate pin and bearer capability travel
//! in an explicit invitation. No terminal, browser, credential or settings API exists.
use crate::workspace::Result;
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD as B64};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    io::{Read, Write},
    net::{SocketAddr, TcpListener, TcpStream},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    thread,
    time::{Duration, Instant},
};
use subtle::ConstantTimeEq;

const MAX_REQUEST: usize = 64 * 1024;
const MAX_FILE: usize = 8 * 1024 * 1024;
const MAX_RESPONSE: usize = 12 * 1024 * 1024;

struct DeadlineStream {
    stream: TcpStream,
    deadline: Instant,
}
impl DeadlineStream {
    fn new(stream: TcpStream) -> Self {
        Self {
            stream,
            deadline: Instant::now() + Duration::from_secs(15),
        }
    }
    fn remaining(&self) -> std::io::Result<Duration> {
        let remaining = self.deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "Share request deadline reached",
            ))
        } else {
            Ok(remaining)
        }
    }
}
impl Read for DeadlineStream {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.stream.set_read_timeout(Some(self.remaining()?))?;
        self.stream.read(buf)
    }
}
impl Write for DeadlineStream {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.stream.set_write_timeout(Some(self.remaining()?))?;
        self.stream.write(buf)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.stream.flush()
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Invitation {
    version: u32,
    address: SocketAddr,
    certificate: String,
    token: String,
}
impl Invitation {
    pub fn encode(&self) -> Result<String> {
        Ok(format!(
            "tessel-rust-share:{}",
            B64.encode(serde_json::to_vec(self).map_err(|e| e.to_string())?)
        ))
    }
    pub fn decode(text: &str) -> Result<Self> {
        if text.len() > 8192 {
            return Err("Invitation is too long.".into());
        }
        let bytes = B64
            .decode(
                text.trim()
                    .strip_prefix("tessel-rust-share:")
                    .ok_or("Not a Tessel Rust invitation")?,
            )
            .map_err(|_| "Invalid invitation encoding")?;
        let invitation: Self = serde_json::from_slice(&bytes).map_err(|_| "Invalid invitation")?;
        if invitation.version != 1
            || invitation.address.ip().is_unspecified()
            || invitation.address.port() == 0
            || B64
                .decode(&invitation.token)
                .map_err(|_| "Invalid invitation token")?
                .len()
                != 32
            || B64
                .decode(&invitation.certificate)
                .map_err(|_| "Invalid certificate")?
                .len()
                > 4096
        {
            return Err("Invalid invitation fields.".into());
        }
        Ok(invitation)
    }
    pub fn endpoint(&self) -> String {
        self.address.to_string()
    }
}

fn read_frame(reader: &mut impl Read, limit: usize) -> Result<Value> {
    let mut header = [0; 4];
    reader
        .read_exact(&mut header)
        .map_err(|_| "Connection ended or timed out")?;
    let size = u32::from_be_bytes(header) as usize;
    if size > limit {
        return Err("Message exceeds size limit.".into());
    }
    let mut bytes = vec![0; size];
    reader
        .read_exact(&mut bytes)
        .map_err(|_| "Incomplete message")?;
    serde_json::from_slice(&bytes).map_err(|_| "Invalid message".into())
}
fn write_frame(writer: &mut impl Write, value: &Value) -> Result<()> {
    let bytes = serde_json::to_vec(value).map_err(|e| e.to_string())?;
    if bytes.len() > MAX_RESPONSE {
        return Err("Response exceeds size limit.".into());
    }
    writer
        .write_all(&(bytes.len() as u32).to_be_bytes())
        .and_then(|_| writer.write_all(&bytes))
        .and_then(|_| writer.flush())
        .map_err(|error| format!("Connection write failed: {error}"))
}
fn private_path(path: &Path) -> bool {
    path.components().any(|part| {
        let name = part.as_os_str().to_string_lossy().to_lowercase();
        name.starts_with('.')
            || matches!(
                name.as_str(),
                "tesselrust"
                    | "appdata"
                    | "application support"
                    | "browser-owner"
                    | "login data"
                    | "cookies"
                    | "web data"
                    | "vault.dat"
                    | "workspace.json"
                    | "id_rsa"
                    | "id_ed25519"
            )
            || [".pem", ".key", ".pfx", ".p12", ".kdbx"]
                .iter()
                .any(|suffix| name.ends_with(suffix))
    })
}
fn share_path(root: &Path, relative: &str) -> Result<PathBuf> {
    let relative = Path::new(relative);
    if relative.is_absolute()
        || relative
            .components()
            .any(|c| !matches!(c, std::path::Component::Normal(_)))
    {
        return Err("Only relative project paths are accepted.".into());
    }
    let path = crate::files::confined(root, &root.join(relative))?;
    // Apply checks to both requested and resolved paths, including junction targets.
    if private_path(relative) || private_path(&path) {
        return Err("Private path is not shared.".into());
    }
    Ok(path)
}
fn handle(root: &Path, request: &Value) -> Result<Value> {
    let operation = request
        .get("operation")
        .and_then(Value::as_str)
        .ok_or("Missing operation")?;
    let relative = request.get("path").and_then(Value::as_str).unwrap_or("");
    let path = if relative.is_empty() {
        root.to_owned()
    } else {
        share_path(root, relative)?
    };
    match operation {
        "list" => {
            let entries = crate::files::list(root, &path)?;
            Ok(
                json!({"entries":entries.into_iter().filter(|e|!private_path(&e.path)).map(|e|json!({"name":e.name,"path":e.path.strip_prefix(root).unwrap_or(&e.path).to_string_lossy(),"directory":e.directory})).collect::<Vec<_>>(),"readOnly":true}),
            )
        }
        "read" => {
            let mut bytes = Vec::new();
            std::fs::File::open(&path)
                .map_err(|_| "File unavailable")?
                .take((MAX_FILE + 1) as u64)
                .read_to_end(&mut bytes)
                .map_err(|_| "File read failed")?;
            if bytes.len() > MAX_FILE {
                return Err("Guest download limit is 8 MiB.".into());
            }
            Ok(
                json!({"name":path.file_name().unwrap_or_default().to_string_lossy(),"data":B64.encode(bytes)}),
            )
        }
        _ => Err("Read-only share: operation is not permitted.".into()),
    }
}

pub struct Host {
    pub invitation: Invitation,
    stop: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}
impl Host {
    pub fn start(root: &Path, bind: SocketAddr, advertise: std::net::IpAddr) -> Result<Self> {
        let root = root.canonicalize().map_err(|e| e.to_string())?;
        if !root.is_dir() || private_path(&root) {
            return Err("This private directory cannot be shared.".into());
        }
        let _ = rustls::crypto::ring::default_provider().install_default();
        let rcgen::CertifiedKey { cert, signing_key } =
            rcgen::generate_simple_self_signed(vec!["tessel-rust".into()])
                .map_err(|e| e.to_string())?;
        let certificate = cert.der().clone();
        let key = rustls::pki_types::PrivatePkcs8KeyDer::from(signing_key.serialize_der());
        let config = rustls::ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(vec![certificate.clone()], key.into())
            .map_err(|e| e.to_string())?;
        let config = Arc::new(config);
        let listener = TcpListener::bind(bind).map_err(|e| e.to_string())?;
        listener.set_nonblocking(true).map_err(|e| e.to_string())?;
        let address = SocketAddr::new(
            advertise,
            listener.local_addr().map_err(|e| e.to_string())?.port(),
        );
        let mut token = [0u8; 32];
        getrandom::fill(&mut token).map_err(|e| e.to_string())?;
        let invitation = Invitation {
            version: 1,
            address,
            certificate: B64.encode(certificate.as_ref()),
            token: B64.encode(token),
        };
        let stop = Arc::new(AtomicBool::new(false));
        let stopping = stop.clone();
        let clients = Arc::new(AtomicUsize::new(0));
        let worker = thread::spawn(move || {
            let mut last_window = Instant::now();
            let mut connections = 0;
            while !stopping.load(Ordering::Relaxed) {
                let (stream, _) = match listener.accept() {
                    Ok(v) => v,
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(20));
                        continue;
                    }
                    Err(_) => break,
                };
                if last_window.elapsed() > Duration::from_secs(60) {
                    last_window = Instant::now();
                    connections = 0;
                }
                connections += 1;
                if connections > 120 || clients.load(Ordering::Relaxed) >= 4 {
                    continue;
                }
                clients.fetch_add(1, Ordering::Relaxed);
                let clients = clients.clone();
                let root = root.clone();
                let config = config.clone();
                let stop = stopping.clone();
                thread::spawn(move || {
                    let _ = stream.set_nonblocking(false);
                    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
                    let _ = stream.set_write_timeout(Some(Duration::from_secs(5)));
                    if let Ok(connection) = rustls::ServerConnection::new(config) {
                        let mut tls =
                            rustls::StreamOwned::new(connection, DeadlineStream::new(stream));
                        if let Ok(request) = read_frame(&mut tls, MAX_REQUEST) {
                            let supplied = request
                                .get("token")
                                .and_then(Value::as_str)
                                .and_then(|v| B64.decode(v).ok())
                                .unwrap_or_default();
                            let allowed = bool::from(supplied.as_slice().ct_eq(&token))
                                && !stop.load(Ordering::Relaxed);
                            let response = if allowed {
                                match handle(&root, &request) {
                                    Ok(result) => json!({"result":result}),
                                    Err(_) => {
                                        json!({"error":"Request unavailable or not allowed by this share."})
                                    }
                                }
                            } else {
                                json!({"error":"Invitation expired or invalid."})
                            };
                            if !stop.load(Ordering::Relaxed) {
                                let _ = write_frame(&mut tls, &response);
                            }
                        }
                    }
                    clients.fetch_sub(1, Ordering::Relaxed);
                });
            }
        });
        Ok(Self {
            invitation,
            stop,
            thread: Some(worker),
        })
    }
}
impl Drop for Host {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(worker) = self.thread.take() {
            let _ = worker.join();
        }
    }
}

pub fn request(invitation: &Invitation, operation: &str, path: &str) -> Result<Value> {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let cert = B64
        .decode(&invitation.certificate)
        .map_err(|_| "Invalid certificate")?;
    let mut roots = rustls::RootCertStore::empty();
    roots
        .add(rustls::pki_types::CertificateDer::from(cert))
        .map_err(|_| "Invalid certificate")?;
    let config = rustls::ClientConfig::builder()
        .with_root_certificates(roots)
        .with_no_client_auth();
    let stream = TcpStream::connect_timeout(&invitation.address, Duration::from_secs(5))
        .map_err(|_| "Host is unreachable")?;
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .map_err(|e| e.to_string())?;
    stream
        .set_write_timeout(Some(Duration::from_secs(10)))
        .map_err(|e| e.to_string())?;
    let connection = rustls::ClientConnection::new(
        Arc::new(config),
        rustls::pki_types::ServerName::try_from("tessel-rust").map_err(|_| "Invalid TLS name")?,
    )
    .map_err(|e| e.to_string())?;
    let mut tls = rustls::StreamOwned::new(connection, DeadlineStream::new(stream));
    write_frame(
        &mut tls,
        &json!({"token":invitation.token,"operation":operation,"path":path}),
    )?;
    let reply = read_frame(&mut tls, MAX_RESPONSE)?;
    if let Some(error) = reply.get("error").and_then(Value::as_str) {
        return Err(error.into());
    }
    reply
        .get("result")
        .cloned()
        .ok_or("Missing host response".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (tempfile::TempDir, Host) {
        let dir = tempfile::Builder::new()
            .prefix("tessel-share-test-")
            .tempdir_in(std::env::current_dir().unwrap())
            .unwrap();
        std::fs::write(dir.path().join("hello.txt"), "shared").unwrap();
        std::fs::write(dir.path().join(".env"), "SECRET").unwrap();
        let host = Host::start(
            dir.path(),
            "127.0.0.1:0".parse().unwrap(),
            "127.0.0.1".parse().unwrap(),
        )
        .unwrap();
        (dir, host)
    }
    #[test]
    fn real_tls_guest_reads_project_but_not_private_paths() {
        let (_dir, host) = fixture();
        let listed = request(&host.invitation, "list", "").unwrap();
        assert_eq!(listed["entries"].as_array().unwrap().len(), 1);
        let read = request(&host.invitation, "read", "hello.txt").unwrap();
        assert_eq!(
            B64.decode(read["data"].as_str().unwrap()).unwrap(),
            b"shared"
        );
        for path in [".env", "../outside", "C:\\Windows\\win.ini"] {
            assert!(request(&host.invitation, "read", path).is_err());
        }
        assert!(request(&host.invitation, "execute", "").is_err());
    }
    #[test]
    fn invitation_is_revocable_and_wrong_token_fails() {
        let (_dir, host) = fixture();
        let invitation = Invitation::decode(&host.invitation.encode().unwrap()).unwrap();
        let mut wrong = invitation.clone();
        wrong.token = B64.encode([0; 32]);
        assert!(request(&wrong, "list", "").is_err());
        drop(host);
        assert!(request(&invitation, "list", "").is_err());
    }
    #[test]
    fn oversized_frame_rejected_before_allocation() {
        let mut data = std::io::Cursor::new(u32::MAX.to_be_bytes());
        assert!(read_frame(&mut data, MAX_REQUEST).is_err());
    }
    #[test]
    fn wrong_certificate_is_not_trusted() {
        let (_one, host) = fixture();
        let (_two, other) = fixture();
        let mut forged = host.invitation.clone();
        forged.certificate = other.invitation.certificate.clone();
        assert!(request(&forged, "list", "").is_err());
    }
}
