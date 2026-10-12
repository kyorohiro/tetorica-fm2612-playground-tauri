use crate::{
    git_dialog as dialog,
    git_keys::Vault,
    project_git::{repo, Project, Snapshot},
};
use base64::{engine::general_purpose::STANDARD_NO_PAD, Engine};
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Manager, WebviewWindow};
pub struct GitState {
    pub project: Option<Project>,
    pub vault: Vault,
}
type Shared = Arc<Mutex<GitState>>;
#[derive(Default)]
pub struct Operations {
    busy: AtomicBool,
    cancelled: AtomicBool,
}
struct OperationGuard(Arc<Operations>);
impl Drop for OperationGuard {
    fn drop(&mut self) {
        self.0.busy.store(false, Ordering::Release);
    }
}
type Result<T> = std::result::Result<T, String>;
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Request {
    action: String,
    project_id: Option<String>,
    revision: Option<u64>,
    snapshot: Option<Snapshot>,
    #[serde(default)]
    paths: Vec<String>,
    #[serde(default)]
    staged: bool,
    message: Option<String>,
    name: Option<String>,
    email: Option<String>,
    url: Option<String>,
}
fn need<'a>(value: &'a Option<String>, field: &str) -> Result<&'a str> {
    value.as_deref().ok_or(format!("Missing {field}"))
}
fn ssh_url(url: &str) -> Result<()> {
    if url.len() > 2048
        || url.chars().any(|c| c.is_whitespace() || c.is_control())
        || !(url.starts_with("ssh://")
            || (url.contains('@') && url.contains(':') && !url.contains("://")))
    {
        return Err("Use an SSH repository URL such as git@github.com:owner/repository.git".into());
    }
    if url.starts_with("ssh://") {
        let parsed = tauri::Url::parse(url).map_err(|_| "Invalid SSH URL")?;
        if parsed.host_str().is_none()
            || parsed.password().is_some()
            || parsed.path() == "/"
            || parsed.path().is_empty()
        {
            return Err("SSH URLs must name a repository and must not contain a password".into());
        }
    } else {
        let (user_host, path) = url.split_once(':').ok_or("Invalid SSH URL")?;
        let (user, host) = user_host.split_once('@').ok_or("Invalid SSH URL")?;
        if user.is_empty()
            || host.is_empty()
            || path.is_empty()
            || !host
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-'))
        {
            return Err("Invalid SSH repository URL".into());
        }
    }
    Ok(())
}
fn callbacks<'a>(
    app: &'a AppHandle,
    vault: &'a Vault,
    private: &'a str,
    public: &'a str,
    url: &'a str,
) -> git2::RemoteCallbacks<'a> {
    let mut callbacks = git2::RemoteCallbacks::new();
    let operations = app.state::<Arc<Operations>>().inner().clone();
    let credentials_cancel = operations.clone();
    let certificate_cancel = operations.clone();
    let transfer_cancel = operations.clone();
    let sideband_cancel = operations.clone();
    callbacks.transfer_progress(move |_| !transfer_cancel.cancelled.load(Ordering::Acquire));
    callbacks.sideband_progress(move |_| !sideband_cancel.cancelled.load(Ordering::Acquire));
    callbacks.push_negotiation(move |_| {
        if operations.cancelled.load(Ordering::Acquire) {
            Err(git2::Error::from_str("Git operation cancelled"))
        } else {
            Ok(())
        }
    });
    let mut attempts = 0;
    callbacks.credentials(move |requested, username, allowed| {
        if credentials_cancel.cancelled.load(Ordering::Acquire) {
            return Err(git2::Error::from_str("Git operation cancelled"));
        }
        attempts += 1;
        if attempts > 4 || requested != url {
            return Err(git2::Error::from_str("Unexpected authentication request"));
        }
        let username = username.unwrap_or("git");
        if allowed.contains(git2::CredentialType::USERNAME) {
            return git2::Cred::username(username);
        }
        if !allowed.contains(git2::CredentialType::SSH_MEMORY)
            && !allowed.contains(git2::CredentialType::SSH_KEY)
        {
            return Err(git2::Error::from_str(
                "Only the Playground SSH key is permitted",
            ));
        }
        git2::Cred::ssh_key_from_memory(username, Some(public), private, None)
    });
    callbacks.certificate_check(move |certificate, _hostname| {
        let check = || -> Result<()> {
            if certificate_cancel.cancelled.load(Ordering::Acquire) {
                return Err("Git operation cancelled".into());
            }
            let host = certificate.as_hostkey().ok_or("Expected an SSH host key")?;
            let hash = host.hash_sha256().ok_or("The server did not provide a SHA-256 host fingerprint")?;
            let fingerprint = format!("SHA256:{}", STANDARD_NO_PAD.encode(hash));
            vault.trust_host(url, &fingerprint, || dialog::confirm(app, "Trust SSH host key?", &format!(
                "Repository: {url}\nHost key: {fingerprint}\nVerify this fingerprint with your Git host before continuing."
            )))
        };
        check().map(|_| git2::CertificateCheckStatus::CertificateOk).map_err(|e| git2::Error::from_str(&e))
    });
    callbacks
}
fn clean(repository: &git2::Repository) -> Result<()> {
    let mut options = git2::StatusOptions::new();
    options.include_untracked(true).recurse_untracked_dirs(true);
    if repository.state() != git2::RepositoryState::Clean
        || !repository
            .statuses(Some(&mut options))
            .map_err(|e| e.to_string())?
            .is_empty()
    {
        return Err("Commit or resolve local changes before pulling".into());
    }
    Ok(())
}
fn fast_forward(repository: &git2::Repository) -> Result<()> {
    clean(repository)?;
    let head = repository.head().map_err(|e| e.to_string())?;
    let name = head.shorthand().map_err(|e| e.to_string())?;
    let local = repository
        .find_branch(name, git2::BranchType::Local)
        .map_err(|e| e.to_string())?;
    let upstream = local
        .upstream()
        .map_err(|_| "Set a tracking branch before pulling".to_owned())?;
    let target = repository
        .reference_to_annotated_commit(upstream.get())
        .map_err(|e| e.to_string())?;
    let (analysis, _) = repository
        .merge_analysis(&[&target])
        .map_err(|e| e.to_string())?;
    if analysis.is_up_to_date() {
        return Ok(());
    }
    if !analysis.is_fast_forward() {
        return Err("Branches have diverged; automatic merge/rebase is not supported yet".into());
    }
    let object = repository
        .find_object(target.id(), None)
        .map_err(|e| e.to_string())?;
    let mut checkout = git2::build::CheckoutBuilder::new();
    checkout.safe();
    repository
        .checkout_tree(&object, Some(&mut checkout))
        .map_err(|e| e.to_string())?;
    let mut head = repository.head().map_err(|e| e.to_string())?;
    head.set_target(target.id(), "Playground fast-forward pull")
        .map_err(|e| e.to_string())?;
    Ok(())
}
fn remote_action(
    app: &AppHandle,
    project: &mut Project,
    vault: &Vault,
    action: &str,
) -> Result<Value> {
    let repository = repo(&project.root)?;
    let mut remote = repository
        .find_remote("origin")
        .map_err(|_| "Configure origin first".to_owned())?;
    let url = remote.url().map_err(|e| e.to_string())?.to_owned();
    ssh_url(&url)?;
    if let Some(push) = remote.pushurl().map_err(|e| e.to_string())? {
        if push != url {
            return Err("Separate push URLs are not supported".into());
        }
    }
    if action == "pull" {
        clean(&repository)?;
    }
    let push_target = if action == "push" {
        let head = repository.head().map_err(|e| e.to_string())?;
        if !head.is_branch() {
            return Err("Select a branch before pushing".into());
        }
        Some((
            head.name().map_err(|e| e.to_string())?.to_owned(),
            head.target().ok_or("Branch has no commit")?,
        ))
    } else {
        None
    };
    let description = if let Some((branch, commit)) = &push_target {
        format!(
            "Push {branch} at {commit} to {url}\nOnly a normal (non-force) push will be performed."
        )
    } else {
        format!("{action} from {url}")
    };
    let password = dialog::passphrase(app, "Playground SSH authentication", &description, false)?;
    let (private, public) = vault.unlock(&password)?;
    let mut fetched = git2::FetchOptions::new();
    fetched.remote_callbacks(callbacks(app, vault, &private, &public, &url));
    if action == "push" {
        let (branch, commit) = push_target.ok_or("Missing push target")?;
        // Pin the reviewed commit across native authentication and host dialogs.
        let refspec = format!("{commit}:{branch}");
        let mut callback = callbacks(app, vault, &private, &public, &url);
        let failure = Arc::new(Mutex::new(None));
        let record = failure.clone();
        callback.push_update_reference(move |_, status| {
            if let Some(status) = status {
                *record.lock().unwrap() = Some(status.to_owned());
            }
            Ok(())
        });
        let mut options = git2::PushOptions::new();
        options.remote_callbacks(callback);
        remote
            .push(&[refspec], Some(&mut options))
            .map_err(|e| e.to_string())?;
        if let Some(error) = failure.lock().map_err(|e| e.to_string())?.take() {
            return Err(error);
        };
    } else {
        remote
            .fetch(&[] as &[&str], Some(&mut fetched), None)
            .map_err(|e| e.to_string())?;
        if action == "pull" {
            fast_forward(&repository)?;
        }
    }
    if action == "pull" {
        project.reload()
    } else {
        Ok(json!({"operation":action,"completed":true}))
    }
}
fn execute(app: &AppHandle, state: &Shared, request: Request) -> Result<Value> {
    let mut state = state.lock().map_err(|e| e.to_string())?;
    if request.action == "keys" {
        return state.vault.info();
    }
    if request.action == "generate-key" {
        if state.vault.info()?["generated"] == true {
            return Err("A Playground key already exists".into());
        }
        let password=dialog::passphrase(app,"Generate Playground SSH key","Enter and repeat a passphrase. This key is used only by Playground. Keep the passphrase; it cannot be recovered.",true)?;
        return state.vault.generate(&password);
    }
    if matches!(request.action.as_str(), "open" | "clone" | "create") {
        let root = dialog::folder(
            app,
            if request.action == "clone" {
                "Choose an empty folder for the clone"
            } else if request.action == "create" {
                "Save current project: choose or create an empty folder"
            } else {
                "Open Playground project folder"
            },
        )?;
        let root = root.canonicalize().map_err(|e| e.to_string())?;
        let key_directory = state
            .vault
            .directory
            .canonicalize()
            .map_err(|e| e.to_string())?;
        if root.starts_with(&key_directory) || key_directory.starts_with(&root) {
            return Err("The application's key storage cannot be opened as a project, including its parent folders".into());
        }
        if request.action == "clone" {
            if std::fs::read_dir(&root)
                .map_err(|e| e.to_string())?
                .next()
                .is_some()
            {
                return Err("Clone destination must be empty".into());
            }
            let url = need(&request.url, "URL")?;
            ssh_url(url)?;
            let password = dialog::passphrase(
                app,
                "Clone repository",
                &format!("Clone {url} into {}", root.display()),
                false,
            )?;
            let (private, public) = state.vault.unlock(&password)?;
            let mut fetch = git2::FetchOptions::new();
            fetch.remote_callbacks(callbacks(app, &state.vault, &private, &public, url));
            git2::build::RepoBuilder::new()
                .fetch_options(fetch)
                .clone(url, &root)
                .map_err(|e| e.to_string())?;
        }
        let (project, mut value) = if request.action == "create" {
            Project::create(root, request.snapshot.ok_or("Missing snapshot")?)?
        } else {
            Project::open(root)?
        };
        value["gitSnapshot"] = project.git_snapshot()?;
        state.project = Some(project);
        return Ok(value);
    }
    let GitState { project, vault } = &mut *state;
    let project = project.as_mut().ok_or("Open a project folder first")?;
    project.check(
        need(&request.project_id, "project ID")?,
        request.revision.ok_or("Missing project revision")?,
    )?;
    match request.action.as_str() {
        "save" => project.save(request.snapshot.ok_or("Missing snapshot")?),
        "reload" => project.reload(),
        "init" => project.init(),
        "status" => project.status(),
        "diff" => project.diff(request.staged),
        "stage" => project.stage(&request.paths),
        "commit" => project.commit(
            need(&request.message, "commit message")?,
            need(&request.name, "author name")?,
            need(&request.email, "author email")?,
        ),
        "log" => project.log(),
        "remote" => {
            let url = need(&request.url, "URL")?;
            ssh_url(url)?;
            dialog::confirm(
                app,
                "Set Git origin?",
                &format!(
                    "Project: {}\nRemote: {url}\nThis remote will use the Playground SSH key.",
                    project.root.display()
                ),
            )?;
            let repository = repo(&project.root)?;
            if repository.find_remote("origin").is_ok() {
                repository
                    .remote_set_url("origin", url)
                    .map_err(|e| e.to_string())?;
            } else {
                repository
                    .remote("origin", url)
                    .map_err(|e| e.to_string())?;
            }
            Ok(json!({"remote":url}))
        }
        "fetch" | "push" | "pull" => remote_action(app, project, vault, &request.action),
        _ => Err("Unsupported Git operation".into()),
    }
}
#[tauri::command]
pub async fn git_request(
    window: WebviewWindow,
    app: AppHandle,
    state: tauri::State<'_, Shared>,
    request: Request,
) -> Result<Value> {
    super::allowed(&window)?;
    let operations = app.state::<Arc<Operations>>().inner().clone();
    if operations
        .busy
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return Err("A Git operation is already running".into());
    }
    operations.cancelled.store(false, Ordering::Release);
    let guard = OperationGuard(operations);
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = guard;
        execute(&app, &state, request)
    })
    .await
    .map_err(|e| e.to_string())?
}
#[tauri::command]
pub fn git_cancel(window: WebviewWindow, state: tauri::State<'_, Arc<Operations>>) -> Result<()> {
    super::allowed(&window)?;
    state.cancelled.store(true, Ordering::Release);
    Ok(())
}
pub fn setup(app: &AppHandle) -> Result<Shared> {
    // These libgit2 globals are configured before any Git operation is started.
    unsafe {
        git2::opts::set_server_connect_timeout_in_milliseconds(15000).map_err(|e| e.to_string())?;
        git2::opts::set_server_timeout_in_milliseconds(60000).map_err(|e| e.to_string())?;
    }
    app.manage(Arc::new(Operations::default()));
    let directory = app
        .path()
        .app_data_dir()
        .map_err(|e| e.to_string())?
        .join("git-keys");
    Ok(Arc::new(Mutex::new(GitState {
        project: None,
        vault: Vault::new(directory)?,
    })))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project_git::{File, Snapshot};
    use base64::{engine::general_purpose::STANDARD, Engine};
    #[test]
    fn embedded_fetch_push_and_fast_forward_preserve_local_changes() {
        let root =
            std::env::temp_dir().join(format!("tetorica-remote-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        let origin = git2::Repository::init_bare(root.join("origin.git")).unwrap();
        let seed = root.join("seed");
        std::fs::create_dir(&seed).unwrap();
        let seed = seed.canonicalize().unwrap();
        let (mut project, _) = Project::open(seed.clone()).unwrap();
        project.init().unwrap();
        let save = |project: &mut Project, text: &str| {
            project
                .save(Snapshot {
                    files: vec![File {
                        path: "/index.js".into(),
                        data: STANDARD.encode(text),
                    }],
                    directories: vec![],
                })
                .unwrap();
            project.stage(&["/index.js".into()]).unwrap();
            project
                .commit(text, "Test", "test@example.invalid")
                .unwrap();
        };
        save(&mut project, "first");
        let seed_repo = repo(&seed).unwrap();
        let branch = seed_repo.head().unwrap().name().unwrap().to_owned();
        let mut remote = seed_repo
            .remote("origin", origin.path().to_str().unwrap())
            .unwrap();
        remote
            .push(
                &[format!(
                    "{}:{branch}",
                    seed_repo.head().unwrap().target().unwrap()
                )],
                None,
            )
            .unwrap();
        origin.set_head(&branch).unwrap();
        let clone =
            git2::Repository::clone(origin.path().to_str().unwrap(), root.join("clone")).unwrap();
        save(&mut project, "second");
        remote.push(&[format!("{branch}:{branch}")], None).unwrap();
        clone
            .find_remote("origin")
            .unwrap()
            .fetch(&[] as &[&str], None, None)
            .unwrap();
        fast_forward(&clone).unwrap();
        assert_eq!(
            std::fs::read_to_string(root.join("clone/index.js")).unwrap(),
            "second"
        );
        std::fs::write(root.join("clone/index.js"), "human edit").unwrap();
        assert!(fast_forward(&clone).is_err());
        assert_eq!(
            std::fs::read_to_string(root.join("clone/index.js")).unwrap(),
            "human edit"
        );
        drop(remote);
        drop(seed_repo);
        drop(origin);
        drop(clone);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn requests_do_not_accept_passwords_or_arbitrary_native_paths() {
        assert!(
            serde_json::from_value::<Request>(json!({"action":"keys","passphrase":"secret"}))
                .is_err()
        );
        assert!(serde_json::from_value::<Request>(
            json!({"action":"open","path":"/arbitrary/native/path"})
        )
        .is_err());
        for url in [
            "https://example.com/repo",
            "file:///tmp/repo",
            "ext::sh -c command",
            "git@host:repo\ncommand",
            "ssh://git:password@host/repo",
        ] {
            assert!(ssh_url(url).is_err());
        }
    }
}
