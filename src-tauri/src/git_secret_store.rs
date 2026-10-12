//! Platform credential backends. Secret bytes never cross the WebView boundary.
use zeroize::Zeroizing;
type Result<T> = std::result::Result<T, String>;
pub const SERVICE: &str = "net.tetorica.ym2612-playground.git-tokens";
pub trait Secrets: Send {
    fn put(&mut self, id: &str, secret: &[u8]) -> Result<()>;
    fn get(&self, id: &str) -> Result<Zeroizing<Vec<u8>>>;
    fn delete(&mut self, id: &str) -> Result<()>;
}
pub struct Keychain;
pub const SUPPORTED: bool = cfg!(any(
    target_os = "macos",
    target_os = "windows",
    target_os = "linux"
));
pub fn storage_name() -> &'static str {
    if cfg!(target_os = "macos") {
        "macOS Keychain"
    } else if cfg!(target_os = "windows") {
        "Windows Credential Manager"
    } else if cfg!(target_os = "linux") {
        "Secret Service"
    } else {
        "OS credential store"
    }
}
#[cfg(target_os = "macos")]
impl Secrets for Keychain {
    fn put(&mut self, id: &str, secret: &[u8]) -> Result<()> {
        security_framework::passwords::set_generic_password(SERVICE, id, secret)
            .map_err(|_| "Could not save the token in macOS Keychain".into())
    }
    fn get(&self, id: &str) -> Result<Zeroizing<Vec<u8>>> {
        security_framework::passwords::generic_password(security_framework::passwords::PasswordOptions::new_generic_password(SERVICE,id))
            .map(Zeroizing::new).map_err(|_|"Could not read the saved token from macOS Keychain. Unlock Keychain or delete and register this token again in Menu → Git".into())
    }
    fn delete(&mut self, id: &str) -> Result<()> {
        match security_framework::passwords::delete_generic_password(SERVICE, id) {
            Ok(()) => Ok(()),
            Err(error) if error.code() == -25300 => Ok(()),
            Err(_) => Err("Could not delete the token from macOS Keychain".into()),
        }
    }
}
#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
impl Secrets for Keychain {
    fn put(&mut self, _: &str, _: &[u8]) -> Result<()> {
        Err("Saved tokens are unavailable on this platform".into())
    }
    fn get(&self, _: &str) -> Result<Zeroizing<Vec<u8>>> {
        Err("Saved tokens are unavailable on this platform".into())
    }
    fn delete(&mut self, _: &str) -> Result<()> {
        Err("Saved tokens are unavailable on this platform".into())
    }
}
#[cfg(any(target_os = "windows", target_os = "linux"))]
impl Secrets for Keychain {
    fn put(&mut self, id: &str, secret: &[u8]) -> Result<()> {
        let entry = keyring::Entry::new(SERVICE, id).map_err(|_| storage_error("open"))?;
        entry.set_secret(secret).map_err(|_| storage_error("save"))
    }
    fn get(&self, id: &str) -> Result<Zeroizing<Vec<u8>>> {
        let entry = keyring::Entry::new(SERVICE, id).map_err(|_| storage_error("open"))?;
        entry
            .get_secret()
            .map(Zeroizing::new)
            .map_err(|_| storage_error("read"))
    }
    fn delete(&mut self, id: &str) -> Result<()> {
        let entry = keyring::Entry::new(SERVICE, id).map_err(|_| storage_error("open"))?;
        match entry.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(_) => Err(storage_error("delete")),
        }
    }
}
#[cfg(any(target_os = "windows", target_os = "linux"))]
fn storage_error(action: &str) -> String {
    if cfg!(target_os = "linux") {
        format!("Could not {action} the token in Secret Service. Start and unlock a Secret Service provider such as GNOME Keyring or KDE Wallet. The token is not saved in a file.")
    } else {
        format!("Could not {action} the token in Windows Credential Manager. Check that the credential service is available.")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "writes and removes a unique dummy item in the OS credential store"]
    fn os_credential_round_trip() {
        let id = format!(
            "dummy-platform-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        struct Cleanup(String);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = Keychain.delete(&self.0);
            }
        }
        let _cleanup = Cleanup(id.clone());
        let mut store = Keychain;
        store.put(&id, b"dummy-one").unwrap();
        assert_eq!(&*store.get(&id).unwrap(), b"dummy-one");
        store.put(&id, b"dummy-two").unwrap();
        assert_eq!(&*store.get(&id).unwrap(), b"dummy-two");
        store.delete(&id).unwrap();
        assert!(store.get(&id).is_err());
        store.delete(&id).unwrap();
    }
}
