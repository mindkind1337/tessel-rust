//! Owner-only account transport and Windows-protected session persistence.
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use subtle::ConstantTimeEq;
use zeroize::{Zeroize, Zeroizing};
const LIMIT: u64 = 65536;
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
fn opaque(s: &str) -> bool {
    (16..=128).contains(&s.len())
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}
fn random() -> Result<String, String> {
    let mut bytes = [0; 32];
    getrandom::fill(&mut bytes).map_err(|_| "Random generator unavailable")?;
    Ok(URL_SAFE_NO_PAD.encode(bytes))
}
fn challenge(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}
#[derive(Serialize, Deserialize)]
pub struct User {
    pub id: String,
    pub email: String,
    pub name: Option<String>,
}
impl User {
    fn valid(&self) -> bool {
        !self.id.is_empty()
            && self.id.len() <= 128
            && self.email.contains('@')
            && self.email.len() <= 320
            && self.name.as_ref().is_none_or(|n| n.len() <= 1024)
            && [&self.id, &self.email]
                .iter()
                .all(|s| !s.chars().any(char::is_control))
    }
}
#[derive(Serialize, Deserialize)]
pub struct Session {
    token: String,
    pub expires_at: u64,
    pub user: User,
}
impl Drop for Session {
    fn drop(&mut self) {
        self.token.zeroize();
    }
}
impl Session {
    fn valid(&self) -> bool {
        opaque(&self.token)
            && self.expires_at > now()
            && self.expires_at <= now() + 100 * 86400000
            && self.user.valid()
    }
    pub fn bearer(&self) -> Zeroizing<String> {
        Zeroizing::new(self.token.clone())
    }
}
#[derive(Serialize, Deserialize)]
struct Marker {
    state: String,
    expires: u64,
}
pub struct Flow {
    marker: Marker,
    verifier: Zeroizing<String>,
}
#[derive(Serialize, Deserialize)]
struct Callback {
    state: String,
    code: Option<String>,
    error: Option<String>,
}
impl Drop for Callback {
    fn drop(&mut self) {
        if let Some(c) = &mut self.code {
            c.zeroize();
        }
    }
}
fn parse_callback(raw: &str) -> Result<Callback, String> {
    if raw.len() > 8192 {
        return Err("Invalid account callback".into());
    }
    let u = url::Url::parse(raw).map_err(|_| "Invalid account callback")?;
    if u.scheme() != "tessel-rust"
        || u.host_str() != Some("callback")
        || !matches!(u.path(), "" | "/")
        || !u.username().is_empty()
        || u.password().is_some()
        || u.port().is_some()
        || u.fragment().is_some()
    {
        return Err("Invalid account callback".into());
    }
    let mut fields = std::collections::HashMap::new();
    for (k, v) in u.query_pairs() {
        if !matches!(k.as_ref(), "state" | "code" | "error")
            || fields.insert(k.into_owned(), v.into_owned()).is_some()
        {
            return Err("Invalid account callback".into());
        }
    }
    let state = fields.remove("state").ok_or("Missing account state")?;
    let code = fields.remove("code");
    let error = fields.remove("error");
    if state.len() != 43
        || !opaque(&state)
        || code.is_some() == error.is_some()
        || code.as_ref().is_some_and(|c| !opaque(c))
        || error.as_ref().is_some_and(|e| {
            e.is_empty()
                || e.len() > 128
                || !e.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
        })
    {
        return Err("Invalid account callback".into());
    }
    Ok(Callback { state, code, error })
}
fn read<T: DeserializeOwned>(path: &Path) -> Result<T, String> {
    let f = File::open(path).map_err(|_| "Cannot read private account data")?;
    let mut encrypted = Vec::new();
    f.take(LIMIT + 1)
        .read_to_end(&mut encrypted)
        .map_err(|_| "Cannot read private account data")?;
    if encrypted.len() as u64 > LIMIT {
        return Err("Account data exceeds limit".into());
    }
    let plain = crate::vault::protect(&encrypted, false)?;
    serde_json::from_slice(&plain).map_err(|_| "Invalid private account data".into())
}
fn save<T: Serialize>(path: &Path, value: &T, new: bool) -> Result<(), String> {
    let plain =
        Zeroizing::new(serde_json::to_vec(value).map_err(|_| "Cannot encode account data")?);
    let encrypted = crate::vault::protect(&plain, true)?;
    let mut file =
        tempfile::NamedTempFile::new_in(path.parent().ok_or("Invalid account directory")?)
            .map_err(|_| "Cannot write private account data")?;
    file.write_all(&encrypted)
        .and_then(|_| file.as_file().sync_all())
        .map_err(|_| "Cannot write private account data")?;
    if new {
        file.persist_noclobber(path)
    } else {
        file.persist(path)
    }
    .map_err(|_| "Cannot save private account data")?;
    Ok(())
}
fn default_dir() -> Result<PathBuf, String> {
    Ok(
        directories::ProjectDirs::from("com", "Tessel", "TesselRust")
            .ok_or("No private data directory")?
            .data_local_dir()
            .join("account"),
    )
}
fn callback_path(dir: &Path, state: &str) -> PathBuf {
    dir.join(format!("callback-{state}.dat"))
}
fn accept_callback(dir: &Path, raw: &str) -> Result<(), String> {
    let callback = parse_callback(raw)?;
    let marker: Marker = read(&dir.join("pending.dat"))?;
    if marker.expires <= now()
        || !bool::from(marker.state.as_bytes().ct_eq(callback.state.as_bytes()))
    {
        return Err("No matching active account sign-in".into());
    }
    save(&callback_path(dir, &callback.state), &callback, true)
}
pub fn receive_callback(raw: &str) -> Result<(), String> {
    accept_callback(&default_dir()?, raw)
}
pub struct Store {
    dir: PathBuf,
    _lock: File,
    pub session: Option<Session>,
}
impl Store {
    pub fn directory(&self) -> &Path {
        &self.dir
    }
    pub fn open_default() -> Result<Self, String> {
        Self::open(&default_dir()?)
    }
    fn open(dir: &Path) -> Result<Self, String> {
        fs::create_dir_all(dir).map_err(|_| "Cannot create private account directory")?;
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(dir.join("account.lock"))
            .map_err(|_| "Cannot open account lock")?;
        fs2::FileExt::try_lock_exclusive(&lock)
            .map_err(|_| "Account is already open in another Rust window")?;
        let session = if dir.join("session.dat").exists() {
            let s: Session = read(&dir.join("session.dat"))?;
            if s.valid() { Some(s) } else { None }
        } else {
            None
        };
        Ok(Self {
            dir: dir.into(),
            _lock: lock,
            session,
        })
    }
    pub fn begin(&self) -> Result<Flow, String> {
        let flow = Flow {
            marker: Marker {
                state: random()?,
                expires: now() + 900000,
            },
            verifier: Zeroizing::new(random()?),
        };
        save(&self.dir.join("pending.dat"), &flow.marker, false)?;
        Ok(flow)
    }
    pub fn cancel(&self, flow: &Flow) {
        let _ = fs::remove_file(self.dir.join("pending.dat"));
        let _ = fs::remove_file(callback_path(&self.dir, &flow.marker.state));
    }
    pub fn complete(&self, flow: &Flow) -> Result<Option<Zeroizing<String>>, String> {
        if flow.marker.expires <= now() {
            self.cancel(flow);
            return Err("Sign-in expired; try again".into());
        }
        let path = callback_path(&self.dir, &flow.marker.state);
        if !path.exists() {
            return Ok(None);
        }
        let result = read::<Callback>(&path);
        self.cancel(flow);
        let mut c = result?;
        if !bool::from(c.state.as_bytes().ct_eq(flow.marker.state.as_bytes())) {
            return Err("Invalid account state".into());
        }
        if c.error.is_some() {
            return Err("Sign-in was declined or failed".into());
        }
        Ok(Some(Zeroizing::new(
            c.code.take().ok_or("Missing sign-in code")?,
        )))
    }
    pub fn set(&mut self, s: Session) -> Result<(), String> {
        if !s.valid() {
            return Err("Invalid or expired account session".into());
        }
        save(&self.dir.join("session.dat"), &s, false)?;
        self.session = Some(s);
        Ok(())
    }
    pub fn update_user(&mut self, user: User) -> Result<(), String> {
        let s = self.session.as_mut().ok_or("No active account session")?;
        if user.id != s.user.id || !user.valid() {
            return Err("Account identity changed; sign out and sign in again".into());
        }
        let old = std::mem::replace(&mut s.user, user);
        if let Err(e) = save(&self.dir.join("session.dat"), s, false) {
            s.user = old;
            return Err(e);
        }
        Ok(())
    }
    pub fn clear(&mut self) -> Result<Option<Session>, String> {
        match fs::remove_file(self.dir.join("session.dat")) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err("Cannot remove saved account session".into()),
        }
        Ok(self.session.take())
    }
}
impl Flow {
    pub fn login_url(&self) -> String {
        let mut u = url::Url::parse("https://tesselcode.com/login").expect("constant URL");
        u.query_pairs_mut()
            .append_pair("challenge", &challenge(&self.verifier))
            .append_pair("state", &self.marker.state)
            .append_pair("app", "tessel-rust");
        u.into()
    }
    pub fn verifier(&self) -> Zeroizing<String> {
        Zeroizing::new(self.verifier.to_string())
    }
}
pub struct Api {
    client: reqwest::blocking::Client,
    base: String,
}
impl Api {
    pub fn production() -> Result<Self, String> {
        Self::new("https://api.tesselcode.com")
    }
    fn new(base: &str) -> Result<Self, String> {
        Ok(Self {
            client: reqwest::blocking::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .connect_timeout(Duration::from_secs(10))
                .timeout(Duration::from_secs(20))
                .build()
                .map_err(|_| "Cannot initialize account connection")?,
            base: base.into(),
        })
    }
    fn json<T: DeserializeOwned>(
        &self,
        req: reqwest::blocking::RequestBuilder,
    ) -> Result<T, String> {
        let r = req.send().map_err(|_| "Account connection failed")?;
        if !r.status().is_success() {
            return Err(format!(
                "Account request failed (HTTP {})",
                r.status().as_u16()
            ));
        }
        let mut bytes = Zeroizing::new(Vec::new());
        r.take(LIMIT + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| "Cannot read account response")?;
        if bytes.len() as u64 > LIMIT {
            return Err("Account response exceeds limit".into());
        }
        serde_json::from_slice(&bytes).map_err(|_| "Invalid account response".into())
    }
    pub fn exchange(&self, code: &str, verifier: &str) -> Result<Session, String> {
        #[derive(Serialize)]
        struct Exchange<'a> {
            code: &'a str,
            verifier: &'a str,
        }
        let s: Session = self.json(
            self.client
                .post(format!("{}/auth/token", self.base))
                .json(&Exchange { code, verifier }),
        )?;
        if !s.valid() {
            return Err("Invalid or expired account session".into());
        }
        Ok(s)
    }
    pub fn refresh(&self, token: &str) -> Result<User, String> {
        #[derive(Deserialize)]
        struct Me {
            user: User,
        }
        let me: Me = self.json(
            self.client
                .get(format!("{}/api/me", self.base))
                .bearer_auth(token),
        )?;
        if !me.user.valid() {
            return Err("Invalid account response".into());
        }
        Ok(me.user)
    }
    pub fn logout(&self, token: &str) -> Result<(), String> {
        let r = self
            .client
            .post(format!("{}/auth/logout", self.base))
            .bearer_auth(token)
            .send()
            .map_err(|_| "Local sign-out complete; server revocation failed")?;
        if !r.status().is_success() {
            return Err("Local sign-out complete; server revocation failed".into());
        }
        Ok(())
    }
}
pub fn register_protocol() -> Result<(), String> {
    use winreg::{
        RegKey,
        enums::{HKEY_CURRENT_USER, KEY_READ},
    };
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let path = "Software\\Classes\\tessel-rust";
    if let Ok(key) = hkcu.open_subkey_with_flags(path, KEY_READ)
        && key.get_value::<u32, _>("TesselRust").ok() != Some(1)
    {
        return Err("The tessel-rust protocol belongs to another application".into());
    }
    let exe = std::env::current_exe().map_err(|_| "Cannot locate this application")?;
    let exe = exe.to_str().ok_or("Unsupported executable path")?;
    if exe.contains(['"', '\r', '\n']) {
        return Err("Unsupported executable path".into());
    }
    let (key, _) = hkcu
        .create_subkey(path)
        .map_err(|_| "Cannot register account callback")?;
    key.set_value("TesselRust", &1u32)
        .and_then(|_| key.set_value("", &"URL:Tessel Rust account"))
        .and_then(|_| key.set_value("URL Protocol", &""))
        .map_err(|_| "Cannot register account callback")?;
    let (command, _) = key
        .create_subkey("shell\\open\\command")
        .map_err(|_| "Cannot register account callback")?;
    command
        .set_value("", &format!("\"{exe}\" --account-callback \"%1\""))
        .map_err(|_| "Cannot register account callback".into())
}
#[cfg(test)]
mod tests;
