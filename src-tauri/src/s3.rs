use crate::settings::S3Config;
use s3::{creds::Credentials, Bucket, Region};

const KEYRING_SERVICE: &str = "com.cvprogramer.app";
const KEYRING_USER: &str = "s3-secret-key";

fn entry() -> Result<keyring::Entry, String> {
    keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER).map_err(|e| e.to_string())
}

pub fn load_secret() -> Option<String> {
    entry().ok()?.get_password().ok()
}

pub fn store_secret(secret: &str) -> Result<(), String> {
    let e = entry()?;
    if secret.is_empty() {
        let _ = e.delete_credential();
        return Ok(());
    }
    e.set_password(secret).map_err(|e| format!("Cannot save secret to system keychain: {e}"))
}

pub fn bucket(cfg: &S3Config, secret: &str) -> Result<Box<Bucket>, String> {
    if cfg.bucket.trim().is_empty() {
        return Err("Bucket is required.".into());
    }
    if cfg.access_key.trim().is_empty() || secret.is_empty() {
        return Err("Access key and secret key are required.".into());
    }
    let endpoint = cfg.endpoint.trim().trim_end_matches('/');
    let region_name = if cfg.region.trim().is_empty() { "us-east-1" } else { cfg.region.trim() };
    let region = if endpoint.is_empty() {
        region_name.parse::<Region>().map_err(|e| e.to_string())?
    } else {
        Region::Custom { region: region_name.into(), endpoint: endpoint.into() }
    };
    let creds = Credentials::new(Some(cfg.access_key.trim()), Some(secret), None, None, None).map_err(|e| e.to_string())?;
    let b = Bucket::new(cfg.bucket.trim(), region, creds).map_err(|e| e.to_string())?;
    Ok(if cfg.path_style { b.with_path_style() } else { b })
}

fn key(cfg: &S3Config, name: &str) -> String {
    let p = cfg.prefix.trim().trim_matches('/');
    if p.is_empty() { name.into() } else { format!("{p}/{name}") }
}

/// Verifies credentials, bucket access and write permission by writing and deleting a tiny object.
pub fn test(cfg: &S3Config, secret: &str) -> Result<(), String> {
    let b = bucket(cfg, secret)?;
    let k = key(cfg, ".cv-programer-test");
    let put = b.put_object(&k, b"ok").map_err(|e| format!("Connection failed: {e}"))?;
    if !(200..300).contains(&put.status_code()) {
        return Err(format!("Write failed (HTTP {}): {}", put.status_code(), String::from_utf8_lossy(put.as_slice())));
    }
    let _ = b.delete_object(&k);
    Ok(())
}
