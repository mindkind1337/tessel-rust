//! Owner-only Windows vault. Never part of workspace state or sharing protocols.
use serde::{Deserialize, Serialize};
use std::{
    fs::{File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
};
use zeroize::{Zeroize, Zeroizing};

const LIMIT: u64 = 5 * 1024 * 1024;
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Account {
    pub origin: String,
    pub username: String,
    pub password: String,
}
impl Drop for Account {
    fn drop(&mut self) {
        self.username.zeroize();
        self.password.zeroize();
    }
}
pub fn origin(raw: &str) -> Result<String, String> {
    let url = url::Url::parse(raw).map_err(|_| "Invalid site URL")?;
    if url.scheme() != "https"
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err("Passwords require an HTTPS site without embedded credentials".into());
    }
    Ok(url.origin().ascii_serialization())
}
fn valid(account: &Account) -> bool {
    origin(&account.origin).is_ok_and(|o| o == account.origin)
        && account.username.len() <= 1024
        && !account.password.is_empty()
        && account.password.len() <= 8192
}
fn protect(bytes: &[u8], encrypt: bool) -> Result<Zeroizing<Vec<u8>>, String> {
    use windows::{
        Win32::{
            Foundation::{HLOCAL, LocalFree},
            Security::Cryptography::{
                CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN, CryptProtectData, CryptUnprotectData,
            },
        },
        core::PCWSTR,
    };
    if bytes.len() > LIMIT as usize {
        return Err("Vault exceeds size limit".into());
    }
    let input = CRYPT_INTEGER_BLOB {
        cbData: bytes.len() as u32,
        pbData: bytes.as_ptr() as *mut u8,
    };
    let mut output = CRYPT_INTEGER_BLOB::default();
    // DPAPI allocates output with LocalAlloc; copy then wipe and free it on every success.
    unsafe {
        let result = if encrypt {
            CryptProtectData(
                &input,
                PCWSTR::null(),
                None,
                None,
                None,
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
        } else {
            CryptUnprotectData(
                &input,
                None,
                None,
                None,
                None,
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
        };
        result.map_err(|_| "Windows could not unlock or protect this vault".to_string())?;
        let data = std::slice::from_raw_parts_mut(output.pbData, output.cbData as usize);
        let owned = Zeroizing::new(data.to_vec());
        data.zeroize();
        let _ = LocalFree(Some(HLOCAL(output.pbData.cast())));
        Ok(owned)
    }
}
pub struct Vault {
    path: PathBuf,
    _lock: File,
    pub accounts: Vec<Account>,
}
impl Vault {
    pub fn open_default() -> Result<Self, String> {
        let dirs = directories::ProjectDirs::from("com", "Tessel", "TesselRust")
            .ok_or("No private data directory")?;
        Self::open(dirs.data_local_dir())
    }
    fn open(dir: &Path) -> Result<Self, String> {
        std::fs::create_dir_all(dir).map_err(|_| "Cannot create private vault directory")?;
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(dir.join("vault.lock"))
            .map_err(|_| "Cannot open vault lock")?;
        fs2::FileExt::try_lock_exclusive(&lock).map_err(|_| "Vault is open in another instance")?;
        let path = dir.join("vault.dat");
        let accounts: Vec<Account> = match File::open(&path) {
            Ok(file) => {
                let mut bytes = Zeroizing::new(Vec::new());
                file.take(LIMIT + 1)
                    .read_to_end(&mut bytes)
                    .map_err(|_| "Cannot read vault")?;
                let plain = protect(&bytes, false)?;
                serde_json::from_slice(&plain)
                    .map_err(|_| "Vault format is invalid; original retained")?
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => vec![],
            Err(_) => return Err("Cannot read vault".into()),
        };
        if accounts.len() > 10000 || accounts.iter().any(|a| !valid(a)) {
            return Err("Vault contains invalid records; original retained".into());
        }
        Ok(Self {
            path,
            _lock: lock,
            accounts,
        })
    }
    fn save(&self) -> Result<(), String> {
        let plain =
            Zeroizing::new(serde_json::to_vec(&self.accounts).map_err(|_| "Cannot encode vault")?);
        let encrypted = protect(&plain, true)?;
        let mut temp =
            tempfile::NamedTempFile::new_in(self.path.parent().ok_or("Invalid vault path")?)
                .map_err(|_| "Cannot create protected vault file")?;
        temp.write_all(&encrypted)
            .and_then(|_| temp.as_file().sync_all())
            .map_err(|_| "Cannot write vault")?;
        temp.persist(&self.path)
            .map_err(|_| "Cannot replace vault")?;
        Ok(())
    }
    pub fn import_csv(&mut self, path: &Path) -> Result<usize, String> {
        let mut bytes = Zeroizing::new(Vec::new());
        File::open(path)
            .map_err(|_| "Cannot open selected CSV")?
            .take(LIMIT + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| "Cannot read CSV")?;
        if bytes.len() > LIMIT as usize {
            return Err("CSV exceeds 5 MiB".into());
        }
        let imported = parse_csv(&bytes)?;
        let count = imported.len();
        // Keep previous state available if the atomic save fails.
        let previous = Zeroizing::new(
            serde_json::to_vec(&self.accounts).map_err(|_| "Cannot prepare import")?,
        );
        for account in imported {
            if let Some(index) = self
                .accounts
                .iter()
                .position(|a| a.origin == account.origin && a.username == account.username)
            {
                self.accounts[index] = account;
            } else {
                self.accounts.push(account);
            }
        }
        let result = if self.accounts.len() > 10000 {
            Err("Vault exceeds 10000 accounts".into())
        } else {
            self.save()
        };
        if let Err(e) = result {
            self.accounts =
                serde_json::from_slice(&previous).map_err(|_| "Cannot restore vault state")?;
            return Err(e);
        }
        Ok(count)
    }
    pub fn remove(&mut self, index: usize) -> Result<(), String> {
        if index >= self.accounts.len() {
            return Err("Unknown account".into());
        }
        let account = self.accounts.remove(index);
        if let Err(e) = self.save() {
            self.accounts.insert(index, account);
            return Err(e);
        }
        Ok(())
    }
}
fn parse_csv(bytes: &[u8]) -> Result<Vec<Account>, String> {
    let bytes = bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(bytes);
    let mut reader = csv::ReaderBuilder::new().from_reader(bytes);
    let headers = reader.headers().map_err(|_| "Invalid CSV header")?.clone();
    let column = |name: &str| {
        headers
            .iter()
            .position(|h| h.eq_ignore_ascii_case(name))
            .ok_or_else(|| format!("CSV requires a {name} column"))
    };
    let (url, user, password) = (column("url")?, column("username")?, column("password")?);
    let mut accounts = Vec::new();
    for row in reader.records() {
        let row = row.map_err(|_| "Invalid CSV row")?;
        let account = Account {
            origin: origin(row.get(url).ok_or("Missing site")?)?,
            username: row.get(user).unwrap_or("").into(),
            password: row.get(password).unwrap_or("").into(),
        };
        if !valid(&account) {
            return Err("Invalid account fields".into());
        }
        accounts.push(account);
        if accounts.len() > 10000 {
            return Err("CSV exceeds 10000 accounts".into());
        }
    }
    Ok(accounts)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn encrypted_roundtrip_tamper_and_lock() {
        let dir = tempfile::tempdir().unwrap();
        let csv = dir.path().join("fixture.csv");
        std::fs::write(
            &csv,
            "url,username,password\nhttps://example.com/login,fixture-user,synthetic-secret-123\n",
        )
        .unwrap();
        let mut vault = Vault::open(dir.path()).unwrap();
        assert!(Vault::open(dir.path()).is_err());
        assert_eq!(vault.import_csv(&csv).unwrap(), 1);
        assert_eq!(vault.import_csv(&csv).unwrap(), 1);
        assert_eq!(vault.accounts.len(), 1);
        drop(vault);
        let encrypted = std::fs::read(dir.path().join("vault.dat")).unwrap();
        assert!(
            !encrypted
                .windows(b"synthetic-secret-123".len())
                .any(|w| w == b"synthetic-secret-123")
        );
        let mut vault = Vault::open(dir.path()).unwrap();
        assert_eq!(vault.accounts[0].password, "synthetic-secret-123");
        vault.remove(0).unwrap();
        drop(vault);
        let mut corrupt = encrypted;
        let end = corrupt.len() - 1;
        corrupt[end] ^= 1;
        assert!(protect(&corrupt, false).is_err());
    }
    #[test]
    fn csv_and_origin_fail_closed() {
        assert_eq!(
            origin("https://example.com:443/login").unwrap(),
            "https://example.com"
        );
        for value in [
            "http://example.com",
            "file:///C:/secret",
            "https://user:pass@example.com",
        ] {
            assert!(origin(value).is_err());
        }
        assert!(parse_csv(b"url,username,password\nhttps://example.com,u,\n").is_err());
        assert!(parse_csv(b"url,username,password\nhttps://example.com,u,p,extra\n").is_err());
    }
}
