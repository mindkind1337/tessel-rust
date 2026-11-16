//! Encrypted private account record. Plaintext buffers are zeroized on drop.
use super::{
    service::{Credentials, Store},
    *,
};
pub(super) struct DiskStore {
    pub path: PathBuf,
}
impl Store for DiskStore {
    fn encryption(&self) -> Result<(), String> {
        protect_key(b"Linear protected storage capability", true).map(|_| ())
    }
    fn load(&self) -> Result<Option<Credentials>, String> {
        let Some(plain) = load_plain_at(&self.path)? else {
            return Ok(None);
        };
        // Deserialize the key directly into a zeroizing owner; no JSON Value holds it.
        #[derive(Deserialize)]
        struct Record {
            version: u8,
            #[serde(deserialize_with = "secret")]
            key: Zeroizing<String>,
            identity: LinearIdentity,
        }
        let record: Record =
            serde_json::from_slice(&plain).map_err(|_| "Invalid protected Linear account")?;
        if record.version != 1 {
            return Err("Unsupported Linear account format".into());
        }
        valid_key(&record.key)?;
        let identity = validated_identity(&record.identity)?;
        Ok(Some(Credentials {
            key: record.key,
            identity,
        }))
    }
    fn save(&self, value: &Credentials) -> Result<(), String> {
        #[derive(Serialize)]
        struct Record<'a> {
            version: u8,
            key: &'a str,
            identity: &'a LinearIdentity,
        }
        valid_key(&value.key)?;
        let identity = validated_identity(&value.identity)?;
        let plain = Zeroizing::new(
            serde_json::to_vec(&Record {
                version: 1,
                key: &value.key,
                identity: &identity,
            })
            .map_err(|_| "Cannot encode Linear account")?,
        );
        save_plain_at(&self.path, &plain)
    }
    fn remove(&self) -> Result<(), String> {
        delete_key_at(&self.path)
    }
}
fn secret<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Zeroizing<String>, D::Error> {
    String::deserialize(deserializer).map(Zeroizing::new)
}

pub(super) fn validated_identity(identity: &LinearIdentity) -> Result<LinearIdentity, String> {
    parse::identity(&serde_json::json!({
        "viewer":{"id":identity.viewer.id,"name":identity.viewer.name,"displayName":identity.viewer.display_name,"email":identity.viewer.email},
        "organization":{"id":identity.organization.id,"name":identity.organization.name,"urlKey":identity.organization.url_key}
    }))
}
