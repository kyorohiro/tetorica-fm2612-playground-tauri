//! Named PATs in the OS credential store; app-wide selection, independent of Cassettes.
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{collections::BTreeMap, fs, io::Write, path::PathBuf, sync::Mutex};
use tauri::{AppHandle, Manager, WebviewWindow};
use zeroize::Zeroizing;
type Result<T> = std::result::Result<T, String>;
#[path = "git_secret_store.rs"]
mod backend;
use backend::{Keychain, Secrets, SUPPORTED, storage_name};
#[cfg(test)]
use backend::SERVICE;

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    pub id: String,
    pub name: String,
    pub host: String,
}
#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Metadata {
    tokens: Vec<Entry>,
    #[serde(default, rename = "selectedId")]
    selected_id: Option<String>,
    // Read the initial repository-based format without losing existing tokens.
    #[serde(default, skip_serializing)]
    bindings: BTreeMap<String, String>,
}
pub struct Store {
    directory: PathBuf,
    metadata: Metadata,
    secrets: Box<dyn Secrets>,
}
pub struct Tokens(pub Mutex<Store>);
pub fn host(value: &str) -> Result<String> {
    let url =
        tauri::Url::parse(value).map_err(|_| "Use an HTTPS host such as https://github.com")?;
    if url.scheme() != "https"
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.path() != "/"
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err("Use an HTTPS host without credentials or a repository path".into());
    }
    Ok(url.origin().ascii_serialization())
}
fn token(secret: &str) -> Result<()> {
    if secret.is_empty()
        || secret.len() > 8192
        || secret.chars().any(|c| c.is_control() || c.is_whitespace())
    {
        return Err("Enter a nonempty personal access token without whitespace".into());
    }
    Ok(())
}
impl Store {
    fn open(directory: PathBuf, secrets: Box<dyn Secrets>) -> Result<Self> {
        fs::create_dir_all(&directory).map_err(|e| e.to_string())?;
        if fs::symlink_metadata(&directory)
            .map_err(|e| e.to_string())?
            .file_type()
            .is_symlink()
        {
            return Err("Token metadata directory cannot be a link".into());
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&directory, fs::Permissions::from_mode(0o700))
                .map_err(|e| e.to_string())?;
        }
        let path = directory.join("tokens.json");
        let mut legacy = false;
        let mut metadata = if path.exists() {
            let info = fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
            if !info.is_file() || info.file_type().is_symlink() || info.len() > 1024 * 1024 {
                return Err("Invalid token metadata file".into());
            }
            let value: Value = serde_json::from_slice(&fs::read(&path).map_err(|e| e.to_string())?)
                .map_err(|_| "Invalid token metadata")?;
            legacy = value.get("selectedId").is_none();
            serde_json::from_value::<Metadata>(value).map_err(|_| "Invalid token metadata")?
        } else {
            Metadata::default()
        };
        let mut ids = std::collections::BTreeSet::new();
        for entry in &metadata.tokens {
            if uuid::Uuid::parse_str(&entry.id).is_err()
                || entry.name.trim().is_empty()
                || entry.name.len() > 160
                || entry.name.chars().any(char::is_control)
                || host(&entry.host)? != entry.host
                || !ids.insert(&entry.id)
            {
                return Err("Invalid saved token entry".into());
            }
        }
        for (url, id) in &metadata.bindings {
            let repository = crate::git_http::repository_url(url)?;
            if !metadata.tokens.iter().any(|entry| {
                entry.id == *id && entry.host == repository.origin().ascii_serialization()
            }) {
                return Err("Invalid saved token repository binding".into());
            }
        }
        if legacy && metadata.selected_id.is_none() {
            let previous: std::collections::BTreeSet<_> =
                metadata.bindings.values().cloned().collect();
            metadata.selected_id = if previous.len() == 1 {
                previous.into_iter().next()
            } else if metadata.tokens.len() == 1 {
                Some(metadata.tokens[0].id.clone())
            } else {
                None
            };
        }
        if metadata
            .selected_id
            .as_ref()
            .is_some_and(|id| !metadata.tokens.iter().any(|entry| entry.id == *id))
        {
            return Err("Invalid app token selection".into());
        }
        metadata.bindings.clear();
        let directory = directory.canonicalize().map_err(|e| e.to_string())?;
        Ok(Self {
            directory,
            metadata,
            secrets,
        })
    }
    fn persist(&mut self, next: Metadata) -> Result<()> {
        let directory = fs::symlink_metadata(&self.directory).map_err(|e| e.to_string())?;
        if !directory.is_dir()
            || directory.file_type().is_symlink()
            || self.directory.canonicalize().map_err(|e| e.to_string())? != self.directory
        {
            return Err("Token metadata directory was moved or replaced".into());
        }
        let temporary = self
            .directory
            .join(format!(".tokens-{}", uuid::Uuid::new_v4()));
        let result = (|| {
            let mut options = fs::OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            let mut file = options.open(&temporary).map_err(|e| e.to_string())?;
            file.write_all(&serde_json::to_vec(&next).map_err(|e| e.to_string())?)
                .map_err(|e| e.to_string())?;
            file.sync_all().map_err(|e| e.to_string())?;
            fs::rename(&temporary, self.directory.join("tokens.json")).map_err(|e| e.to_string())
        })();
        if result.is_err() {
            let _ = fs::remove_file(temporary);
        } else {
            self.metadata = next;
        }
        result
    }
    fn entry(&self, id: &str) -> Result<Entry> {
        self.metadata
            .tokens
            .iter()
            .find(|entry| entry.id == id)
            .cloned()
            .ok_or("No such saved token".into())
    }
    fn add(&mut self, name: &str, origin: &str, secret: &str) -> Result<Entry> {
        let name = name.trim();
        if name.is_empty() || name.len() > 160 || name.chars().any(char::is_control) {
            return Err("Enter a token name (at most 160 bytes)".into());
        }
        if self
            .metadata
            .tokens
            .iter()
            .any(|entry| entry.name.to_lowercase() == name.to_lowercase())
        {
            return Err("A saved token already has this name".into());
        }
        if self.metadata.tokens.len() >= 128 {
            return Err("The app supports at most 128 saved tokens".into());
        }
        token(secret)?;
        let host = host(origin)?;
        let entry = Entry {
            id: uuid::Uuid::new_v4().to_string(),
            name: name.into(),
            host,
        };
        self.secrets.put(&entry.id, secret.as_bytes())?;
        let mut next = self.metadata.clone();
        next.tokens.push(entry.clone());
        next.selected_id = Some(entry.id.clone());
        if let Err(error) = self.persist(next) {
            let _ = self.secrets.delete(&entry.id);
            return Err(error);
        }
        Ok(entry)
    }
    fn replace(&mut self, id: &str, secret: &str) -> Result<()> {
        self.entry(id)?;
        token(secret)?;
        self.secrets.put(id, secret.as_bytes())
    }
    fn select(&mut self, id: &str) -> Result<()> {
        self.entry(id)?;
        let mut next = self.metadata.clone();
        next.selected_id = Some(id.into());
        self.persist(next)
    }
    fn remove(&mut self, id: &str) -> Result<()> {
        self.entry(id)?;
        let old = self.metadata.clone();
        let mut next = old.clone();
        next.tokens.retain(|entry| entry.id != id);
        if next.selected_id.as_deref() == Some(id) {
            next.selected_id = None;
        }
        self.persist(next)?;
        if let Err(error) = self.secrets.delete(id) {
            let _ = self.persist(old);
            return Err(error);
        }
        Ok(())
    }
    #[cfg(test)]
    pub fn selected(&self, url: &str) -> Result<Entry> {
        let repository = crate::git_http::repository_url(url)?;
        let id = self
            .metadata
            .selected_id
            .as_ref()
            .ok_or("No app token is selected. Choose a saved token in Menu → Git settings")?;
        let entry = self.entry(id)?;
        if entry.host != repository.origin().ascii_serialization() {
            return Err("Token host does not match the repository".into());
        }
        Ok(entry)
    }
    pub fn choices(&self, url: &str) -> Result<(Vec<Entry>, Option<String>)> {
        let origin = crate::git_http::repository_url(url)?.origin().ascii_serialization();
        let entries: Vec<_> = self.metadata.tokens.iter().filter(|entry| entry.host == origin).cloned().collect();
        if entries.is_empty() {
            return Err("No saved token for this host. Register one in Menu → Git settings".into());
        }
        Ok((entries, self.metadata.selected_id.clone()))
    }
    pub fn choose(&mut self, url: &str, id: &str) -> Result<Entry> {
        let (entries, _) = self.choices(url)?;
        let entry = entries.into_iter().find(|entry| entry.id == id).ok_or("Token host does not match the repository")?;
        self.select(id)?;
        Ok(entry)
    }
    pub fn directory(&self) -> &std::path::Path {
        &self.directory
    }
    pub fn read(&self, entry: &Entry) -> Result<Zeroizing<String>> {
        let expected = self.entry(&entry.id)?;
        if expected.host != entry.host {
            return Err("Saved token changed".into());
        }
        let bytes = self.secrets.get(&entry.id)?;
        let value = std::str::from_utf8(&bytes).map_err(|_| "Invalid saved token")?;
        token(value)?;
        Ok(Zeroizing::new(value.into()))
    }
    fn public(&self) -> Result<Value> {
        let selected = &self.metadata.selected_id;
        Ok(
            json!({"tokens":self.metadata.tokens,"selectedId":selected,"supported":SUPPORTED,"storage":storage_name()}),
        )
    }
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Request {
    action: String,
    name: Option<String>,
    host: Option<String>,
    id: Option<String>,
}
#[tauri::command]
pub async fn git_tokens(window: WebviewWindow, app: AppHandle, request: Request) -> Result<Value> {
    super::allowed(&window)?;
    tauri::async_runtime::spawn_blocking(move||{
        let state=app.state::<Tokens>();let mut store=state.0.lock().map_err(|e|e.to_string())?;
        match request.action.as_str(){
            "list"=>store.public(),
            "add"=>{
                let name=request.name.as_deref().ok_or("Enter a token name")?;
                if name.trim().is_empty()||name.len()>160||name.chars().any(char::is_control){return Err("Enter a token name (at most 160 bytes)".into());}
                let origin=host(request.host.as_deref().ok_or("Enter an HTTPS host")?)?;
                let secret=crate::git_dialog::secret(&app,"Save personal access token",&format!("Save token ‘{name}’ for {origin} in {}. The secret is never returned to JavaScript or saved in the Cassette.",storage_name()),"Personal access token (required)",false)?;
                let entry=store.add(name,&origin,&secret)?;Ok(json!({"saved":true,"entry":entry}))
            }
            "replace"=>{
                let id=request.id.as_deref().ok_or("Choose a token")?;let entry=store.entry(id)?;
                let secret=crate::git_dialog::secret(&app,"Update personal access token",&format!("Replace saved token ‘{}’ for {} in {}.",entry.name,entry.host,storage_name()),"New personal access token (required)",false)?;
                store.replace(id,&secret)?;app.state::<crate::git_http::Transport>().revoke_token(id)?;Ok(json!({"updated":true}))
            }
            "select"=>{
                let id=request.id.as_deref().ok_or("Choose a token")?;let entry=store.entry(id)?;
                crate::git_dialog::confirm(&app,"Select app token",&format!("Use ‘{}’ ({}) in this app? This selection applies to all Cassettes.",entry.name,entry.host))?;
                let previous=store.metadata.selected_id.clone();store.select(id)?;
                if let Some(previous)=previous{app.state::<crate::git_http::Transport>().revoke_token(&previous)?;}Ok(json!({"selectedId":id}))
            }
            "delete"=>{
                let id=request.id.as_deref().ok_or("Choose a token")?;let entry=store.entry(id)?;
                crate::git_dialog::confirm(&app,"Delete saved token",&format!("Delete ‘{}’ from {}? The token itself is not revoked on GitHub.",entry.name,storage_name()))?;
                store.remove(id)?;app.state::<crate::git_http::Transport>().revoke_token(id)?;Ok(json!({"deleted":true}))
            }
            _=>Err("Unsupported saved token operation".into())
        }
    }).await.map_err(|e|e.to_string())?
}
pub fn setup(app: &AppHandle) -> Result<Tokens> {
    Ok(Tokens(Mutex::new(Store::open(
        app.path()
            .app_data_dir()
            .map_err(|e| e.to_string())?
            .join("git-auth"),
        Box::new(Keychain),
    )?)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    #[derive(Clone, Default)]
    struct Memory(Arc<Mutex<BTreeMap<String, Vec<u8>>>>);
    impl Secrets for Memory {
        fn put(&mut self, id: &str, value: &[u8]) -> Result<()> {
            self.0.lock().unwrap().insert(id.into(), value.into());
            Ok(())
        }
        fn get(&self, id: &str) -> Result<Zeroizing<Vec<u8>>> {
            self.0
                .lock()
                .unwrap()
                .get(id)
                .cloned()
                .map(Zeroizing::new)
                .ok_or("Missing token".into())
        }
        fn delete(&mut self, id: &str) -> Result<()> {
            self.0.lock().unwrap().remove(id);
            Ok(())
        }
    }
    struct Temp(PathBuf);
    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    fn temp() -> Temp {
        Temp(std::env::temp_dir().join(format!("tetorica-token-test-{}", uuid::Uuid::new_v4())))
    }
    #[test]
    fn failed_credential_save_does_not_register_a_token() {
        struct Unavailable;
        impl Secrets for Unavailable {
            fn put(&mut self, _: &str, _: &[u8]) -> Result<()> { Err("Credential store unavailable".into()) }
            fn get(&self, _: &str) -> Result<Zeroizing<Vec<u8>>> { Err("Credential store unavailable".into()) }
            fn delete(&mut self, _: &str) -> Result<()> { Err("Credential store unavailable".into()) }
        }
        let t = temp();
        let mut store = Store::open(t.0.clone(), Box::new(Unavailable)).unwrap();
        let error = store.add("GitHub", "https://github.com", "dummy-never-saved").err().unwrap();
        assert!(!error.contains("dummy-never-saved"));
        assert!(store.metadata.tokens.is_empty());
        assert!(store.metadata.selected_id.is_none());
        assert!(!t.0.join("tokens.json").exists());
    }
    #[test]
    fn connection_choices_filter_host_and_revalidate_deleted_tokens() {
        let t = temp();
        let mut store = Store::open(t.0.clone(), Box::new(Memory::default())).unwrap();
        let github = store.add("GitHub", "https://github.com", "dummy-one").unwrap();
        let other = store.add("Other", "https://example.com", "dummy-two").unwrap();
        let url = "https://github.com/owner/repo.git";
        let (choices, preferred) = store.choices(url).unwrap();
        assert_eq!(choices.len(), 1);
        assert_eq!(choices[0].id, github.id);
        assert_eq!(preferred.unwrap(), other.id);
        assert!(store.choose(url, &other.id).is_err());
        store.choose(url, &github.id).unwrap();
        assert_eq!(store.selected(url).unwrap().id, github.id);
        store.remove(&github.id).unwrap();
        assert!(store.choose(url, &github.id).is_err());
        assert!(store.choices(url).is_err());
    }
    #[test]
    fn named_tokens_and_repository_choices_persist_without_secret_serialization() {
        let t = temp();
        let memory = Memory::default();
        let mut store = Store::open(t.0.clone(), Box::new(memory.clone())).unwrap();
        let first = store
            .add("Personal", "https://github.com", "dummy-secret-one")
            .unwrap();
        let second = store
            .add("Work", "https://github.com", "dummy-secret-two")
            .unwrap();
        let repo = "https://github.com/owner/repository.git";
        assert_eq!(store.selected(repo).unwrap().id, second.id);
        store.select(&second.id).unwrap();
        assert_eq!(
            store.read(&store.selected(repo).unwrap()).unwrap().as_str(),
            "dummy-secret-two"
        );
        let public = store.public().unwrap();
        assert_eq!(public["selectedId"], second.id);
        assert_eq!(public["tokens"].as_array().unwrap().len(), 2);
        let json = fs::read_to_string(t.0.join("tokens.json")).unwrap();
        assert!(!json.contains("dummy-secret"));
        assert!(!public.to_string().contains("dummy-secret"));
        let mut restored = Store::open(t.0.clone(), Box::new(memory.clone())).unwrap();
        assert_eq!(restored.selected(repo).unwrap().id, second.id);
        restored.replace(&second.id, "updated-secret").unwrap();
        assert_eq!(restored.read(&second).unwrap().as_str(), "updated-secret");
        restored.select(&first.id).unwrap();
        restored.remove(&first.id).unwrap();
        assert!(restored.selected(repo).is_err());
        assert!(restored.read(&first).is_err());
        let reopened = Store::open(t.0.clone(), Box::new(memory)).unwrap();
        assert!(reopened.selected(repo).is_err());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(t.0.join("tokens.json"))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
    }
    #[test]
    fn host_scoping_and_ipc_do_not_accept_secret_values_or_unapproved_repositories() {
        let t = temp();
        let mut store = Store::open(t.0.clone(), Box::new(Memory::default())).unwrap();
        let entry = store
            .add("GitHub", "https://github.com", "dummy-secret")
            .unwrap();
        assert!(store
            .selected("https://evil.example/owner/repository.git")
            .is_err());
        assert!(store.select("unknown").is_err());
        assert_eq!(
            store
                .selected("https://github.com/owner/one.git")
                .unwrap()
                .id,
            entry.id
        );
        assert_eq!(
            store
                .selected("https://github.com/owner/two.git")
                .unwrap()
                .id,
            entry.id
        );
        assert!(store
            .add("github", "https://github.com", "another")
            .is_err());
        assert!(store.replace(&entry.id, "").is_err());
        for value in [
            "http://github.com",
            "https://user:secret@github.com",
            "https://github.com/owner/repo",
        ] {
            assert!(host(value).is_err());
        }
        assert!(serde_json::from_value::<Request>(
            json!({"action":"add","name":"test","host":"https://github.com","token":"secret"})
        )
        .is_err());
    }
    #[cfg(target_os = "macos")]
    #[test]
    #[ignore = "writes a temporary dummy item to macOS Keychain"]
    fn macos_keychain_round_trip() {
        let service = format!("{SERVICE}.self-test");
        let account = uuid::Uuid::new_v4().to_string();
        struct Cleanup(String, String);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = security_framework::passwords::delete_generic_password(&self.0, &self.1);
            }
        }
        let _cleanup = Cleanup(service.clone(), account.clone());
        security_framework::passwords::set_generic_password(
            &service,
            &account,
            b"dummy-test-token",
        )
        .unwrap();
        let bytes = Zeroizing::new(
            security_framework::passwords::generic_password(
                security_framework::passwords::PasswordOptions::new_generic_password(
                    &service, &account,
                ),
            )
            .unwrap(),
        );
        assert_eq!(bytes.as_slice(), b"dummy-test-token");
        security_framework::passwords::set_generic_password(
            &service,
            &account,
            b"updated-test-token",
        )
        .unwrap();
        let bytes = Zeroizing::new(
            security_framework::passwords::generic_password(
                security_framework::passwords::PasswordOptions::new_generic_password(
                    &service, &account,
                ),
            )
            .unwrap(),
        );
        assert_eq!(bytes.as_slice(), b"updated-test-token");
        security_framework::passwords::delete_generic_password(&service, &account).unwrap();
        assert!(security_framework::passwords::generic_password(
            security_framework::passwords::PasswordOptions::new_generic_password(
                &service, &account
            )
        )
        .is_err());
    }
    #[test]
    fn repository_format_migrates_to_app_selection_without_recreating_keychain_items() {
        let t = temp();
        let memory = Memory::default();
        let mut store = Store::open(t.0.clone(), Box::new(memory.clone())).unwrap();
        let a = store
            .add("Personal", "https://github.com", "dummy-one")
            .unwrap();
        let b = store
            .add("Work", "https://github.com", "dummy-two")
            .unwrap();
        fs::write(t.0.join("tokens.json"),serde_json::to_vec(&json!({"tokens":[a,b.clone()],"bindings":{"https://github.com/owner/old.git":b.id}})).unwrap()).unwrap();
        let mut migrated = Store::open(t.0.clone(), Box::new(memory)).unwrap();
        assert_eq!(
            migrated
                .selected("https://github.com/other/new.git")
                .unwrap()
                .id,
            b.id
        );
        assert_eq!(migrated.read(&b).unwrap().as_str(), "dummy-two");
        migrated.select(&b.id).unwrap();
        let json = fs::read_to_string(t.0.join("tokens.json")).unwrap();
        assert!(!json.contains("bindings"));
        assert!(!json.contains("old.git"));
        assert!(json.contains("selectedId"));
    }
}
