//! One app-generated, encrypted Ed25519 identity. External key paths are never accepted.
use serde_json::{json, Value};
use ssh_key::{Algorithm, HashAlg, LineEnding, PrivateKey};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};
use zeroize::Zeroizing;
type Result<T> = std::result::Result<T, String>;
pub struct Vault {
    pub directory: PathBuf,
}
fn regular(path: &Path) -> Result<()> {
    let meta = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if !meta.is_file() || meta.file_type().is_symlink() {
        return Err("Invalid application key file".into());
    }
    Ok(())
}
pub fn private_write(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path).map_err(|e| e.to_string())?;
    file.write_all(bytes).map_err(|e| e.to_string())?;
    file.sync_all().map_err(|e| e.to_string())
}
impl Vault {
    pub fn new(directory: PathBuf) -> Result<Self> {
        fs::create_dir_all(&directory).map_err(|e| e.to_string())?;
        if fs::symlink_metadata(&directory)
            .map_err(|e| e.to_string())?
            .file_type()
            .is_symlink()
        {
            return Err("Invalid application key directory".into());
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&directory, fs::Permissions::from_mode(0o700))
                .map_err(|e| e.to_string())?;
        }
        Ok(Self { directory })
    }
    fn key_path(&self) -> PathBuf {
        self.directory.join("playground-ed25519")
    }
    pub fn generate(&self, passphrase: &str) -> Result<Value> {
        if passphrase.is_empty() {
            return Err("A nonempty passphrase is required".into());
        }
        if self.key_path().exists() {
            return Err("A Playground key already exists. It will not be overwritten.".into());
        }
        let key = PrivateKey::random(&mut rand_core::OsRng, Algorithm::Ed25519)
            .map_err(|e| e.to_string())?;
        let encrypted = key
            .encrypt(&mut rand_core::OsRng, passphrase)
            .map_err(|e| e.to_string())?;
        let pem = encrypted
            .to_openssh(LineEnding::LF)
            .map_err(|e| e.to_string())?;
        private_write(&self.key_path(), pem.as_bytes())?;
        self.info()
    }
    fn encrypted(&self) -> Result<PrivateKey> {
        regular(&self.key_path())?;
        let pem = Zeroizing::new(fs::read_to_string(self.key_path()).map_err(|e| e.to_string())?);
        let key = PrivateKey::from_openssh(pem.as_bytes())
            .map_err(|_| "Invalid Playground key".to_owned())?;
        if !key.is_encrypted() {
            return Err("Unencrypted private keys are not permitted".into());
        }
        Ok(key)
    }
    pub fn info(&self) -> Result<Value> {
        if !self.key_path().exists() {
            return Ok(json!({"generated":false}));
        }
        let key = self.encrypted()?;
        Ok(
            json!({"generated":true,"keyId":"playground-ed25519","publicKey":key.public_key().to_openssh().map_err(|e|e.to_string())?,"fingerprint":key.public_key().fingerprint(HashAlg::Sha256).to_string()}),
        )
    }
    pub fn unlock(&self, passphrase: &str) -> Result<(Zeroizing<String>, String)> {
        if passphrase.is_empty() {
            return Err("A nonempty passphrase is required".into());
        }
        let key = self
            .encrypted()?
            .decrypt(passphrase)
            .map_err(|_| "Incorrect passphrase or invalid Playground key".to_owned())?;
        let pem = key.to_openssh(LineEnding::LF).map_err(|e| e.to_string())?;
        let public = key.public_key().to_openssh().map_err(|e| e.to_string())?;
        Ok((pem, public))
    }
    pub fn trust_host(
        &self,
        url: &str,
        fingerprint: &str,
        confirm: impl FnOnce() -> Result<()>,
    ) -> Result<()> {
        let path = self.directory.join("known-hosts.json");
        let mut hosts: std::collections::BTreeMap<String, String> = if path.exists() {
            regular(&path)?;
            serde_json::from_slice(&fs::read(&path).map_err(|e| e.to_string())?)
                .map_err(|e| e.to_string())?
        } else {
            Default::default()
        };
        match hosts.get(url) {
            Some(expected) if expected == fingerprint => return Ok(()),
            Some(_) => return Err("SSH host key changed. Connection refused; verify the server before resetting its entry.".into()),
            None => {},
        }
        confirm()?;
        hosts.insert(url.to_owned(), fingerprint.to_owned());
        let temporary = self
            .directory
            .join(format!("hosts-{}", uuid::Uuid::new_v4()));
        private_write(
            &temporary,
            &serde_json::to_vec(&hosts).map_err(|e| e.to_string())?,
        )?;
        fs::rename(temporary, path).map_err(|e| e.to_string())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn host_keys_require_first_confirmation_and_never_accept_changes() {
        let dir = std::env::temp_dir().join(format!("tetorica-host-test-{}", uuid::Uuid::new_v4()));
        let vault = Vault::new(dir.clone()).unwrap();
        assert!(vault
            .trust_host(
                "ssh://test/repo",
                "SHA256:first",
                || Err("Cancelled".into())
            )
            .is_err());
        assert!(!dir.join("known-hosts.json").exists());
        vault
            .trust_host("ssh://test/repo", "SHA256:first", || Ok(()))
            .unwrap();
        vault
            .trust_host("ssh://test/repo", "SHA256:first", || {
                panic!("Known key must not prompt again")
            })
            .unwrap();
        assert!(vault
            .trust_host("ssh://test/repo", "SHA256:changed", || panic!(
                "Changed key must never be accepted"
            ))
            .is_err());
        fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn keys_are_app_generated_encrypted_and_return_only_public_metadata() {
        let dir = std::env::temp_dir().join(format!("tetorica-key-test-{}", uuid::Uuid::new_v4()));
        let vault = Vault::new(dir.clone()).unwrap();
        assert!(vault.generate("").is_err());
        let info = vault.generate("test passphrase").unwrap();
        assert!(info["publicKey"]
            .as_str()
            .unwrap()
            .starts_with("ssh-ed25519 "));
        assert!(!info.to_string().contains("PRIVATE"));
        assert!(vault.generate("another").is_err());
        assert!(vault.unlock("wrong").is_err());
        let (private, _) = vault.unlock("test passphrase").unwrap();
        assert!(!PrivateKey::from_openssh(private.as_bytes())
            .unwrap()
            .is_encrypted());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(vault.key_path()).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn external_unencrypted_key_is_rejected() {
        let dir = std::env::temp_dir().join(format!("tetorica-key-test-{}", uuid::Uuid::new_v4()));
        let vault = Vault::new(dir.clone()).unwrap();
        let key = PrivateKey::random(&mut rand_core::OsRng, Algorithm::Ed25519).unwrap();
        private_write(
            &vault.key_path(),
            key.to_openssh(LineEnding::LF).unwrap().as_bytes(),
        )
        .unwrap();
        assert!(vault.info().is_err());
        fs::remove_dir_all(dir).unwrap();
    }
}

// Run against the ephemeral test-only SSH server. It never uses a user key,
// SSH agent, system git, or system ssh executable.
#[cfg(test)]
mod transport_test {
    use super::*;
    #[test]
    #[ignore = "requires the ephemeral SSH server"]
    fn encrypted_app_key_authenticates_through_embedded_libssh2() {
        let port = std::env::var("TETORICA_TEST_SSH_PORT").expect("test server port");
        let expected =
            std::env::var("TETORICA_TEST_SSH_FINGERPRINT").expect("test host fingerprint");
        let dir = std::env::temp_dir().join(format!("tetorica-ssh-test-{}", uuid::Uuid::new_v4()));
        let vault = Vault::new(dir.clone()).unwrap();
        vault.generate("temporary test passphrase").unwrap();
        let (private, public) = vault.unlock("temporary test passphrase").unwrap();
        let repository = git2::Repository::init(dir.join("repo")).unwrap();
        let url = format!("ssh://git@127.0.0.1:{port}/test.git");
        let mut remote = repository.remote_anonymous(&url).unwrap();
        let mut cb = git2::RemoteCallbacks::new();
        cb.credentials(|_, user, _| {
            git2::Cred::ssh_key_from_memory(user.unwrap_or("git"), Some(&public), &private, None)
        });
        cb.certificate_check(|cert, _| {
            use base64::Engine;
            let hash = cert.as_hostkey().unwrap().hash_sha256().unwrap();
            assert_eq!(
                base64::engine::general_purpose::STANDARD_NO_PAD.encode(hash),
                expected
            );
            Ok(git2::CertificateCheckStatus::CertificateOk)
        });
        {
            let mut connection = remote
                .connect_auth(git2::Direction::Fetch, Some(cb), None)
                .unwrap();
            assert!(connection.remote().connected());
        }
        drop(remote);
        drop(repository);
        fs::remove_dir_all(dir).unwrap();
    }
}
