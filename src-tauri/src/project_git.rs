//! Project-scoped file synchronization and embedded Git. No command execution.
use base64::{engine::general_purpose::STANDARD, Engine};
use git2::{DiffOptions, Repository, RepositoryOpenFlags, Signature, StatusOptions};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::Write,
    path::{Component, Path, PathBuf},
};

const MAX_FILE: usize = 64 * 1024 * 1024;
const MAX_PROJECT: usize = 256 * 1024 * 1024;
const MAX_FILES: usize = 10000;
type Result<T> = std::result::Result<T, String>;
fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct File {
    pub path: String,
    pub data: String,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Snapshot {
    pub files: Vec<File>,
    #[serde(default)]
    pub directories: Vec<String>,
}

pub struct Project {
    pub id: String,
    pub root: PathBuf,
    pub revision: u64,
    baseline: BTreeMap<String, Vec<u8>>,
}
fn reserved(part: &str) -> bool {
    matches!(
        part.to_ascii_lowercase().as_str(),
        ".git" | ".ssh" | "sys" | "node_modules" | "target" | ".ds_store"
    )
}
pub fn relative(path: &str) -> Result<PathBuf> {
    if !path.starts_with('/') || path.len() < 2 || path.contains(['\\', '\0', ':']) {
        return Err("Invalid project path".into());
    }
    let mut result = PathBuf::new();
    for part in path[1..].split('/') {
        if part.is_empty()
            || part == "."
            || part == ".."
            || reserved(part)
            || part.ends_with(['.', ' '])
        {
            return Err(format!("Reserved or invalid project path: {path}"));
        }
        result.push(part);
    }
    if result
        .components()
        .any(|c| !matches!(c, Component::Normal(_)))
    {
        return Err("Invalid project path".into());
    }
    Ok(result)
}
fn safe_path(root: &Path, name: &str) -> Result<PathBuf> {
    verify_root(root)?;
    let rel = relative(name)?;
    // Reject links at every component, including the final file. Never follow a
    // WebView-supplied path through a link into another project or ~/.ssh.
    let mut current = root.to_path_buf();
    for part in rel.components() {
        current.push(part.as_os_str());
        match fs::symlink_metadata(&current) {
            Ok(meta) if meta.file_type().is_symlink() => {
                return Err(format!("Symbolic links are not supported: {name}"))
            }
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(err(e)),
        }
        if current != root && current.join(".git").exists() {
            return Err("Nested repositories are outside this project's managed files".into());
        }
    }
    Ok(current)
}
fn verify_root(root: &Path) -> Result<()> {
    let meta = fs::symlink_metadata(root).map_err(err)?;
    if !meta.is_dir() || meta.file_type().is_symlink() || root.canonicalize().map_err(err)? != root
    {
        return Err("Project folder was moved or replaced; reopen it".into());
    }
    Ok(())
}
fn read_file(path: &Path) -> Result<Option<Vec<u8>>> {
    match fs::symlink_metadata(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(err(e)),
        Ok(meta) => {
            if !meta.is_file() || meta.file_type().is_symlink() {
                return Err("Expected a regular project file".into());
            }
            if meta.len() > MAX_FILE as u64 {
                return Err("Project file exceeds 64 MiB".into());
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::MetadataExt;
                if meta.nlink() != 1 {
                    return Err("Hard-linked project files are not supported".into());
                }
            }
            fs::read(path).map(Some).map_err(err)
        }
    }
}
fn verify_git_dir(root: &Path) -> Result<()> {
    verify_root(root)?;
    let path = root.join(".git");
    if let Ok(meta) = fs::symlink_metadata(path) {
        if meta.file_type().is_symlink() || !meta.is_dir() {
            return Err("Linked repositories and worktrees are not supported yet".into());
        }
    }
    Ok(())
}
pub fn repo(root: &Path) -> Result<Repository> {
    verify_git_dir(root)?;
    let repository = Repository::open_ext(
        root,
        RepositoryOpenFlags::NO_SEARCH,
        std::iter::empty::<&Path>(),
    )
    .map_err(err)?;
    if repository.is_bare() {
        return Err("Open a working-tree repository".into());
    }
    if repository
        .workdir()
        .ok_or("Missing working tree")?
        .canonicalize()
        .map_err(err)?
        != root
    {
        return Err("External Git working trees are not supported".into());
    }
    Ok(repository)
}
fn scan(root: &Path) -> Result<(BTreeMap<String, Vec<u8>>, Vec<String>)> {
    verify_git_dir(root)?;
    let repository = repo(root).ok();
    let index = repository.as_ref().and_then(|r| r.index().ok());
    fn walk(
        root: &Path,
        dir: &Path,
        repository: Option<&Repository>,
        index: Option<&git2::Index>,
        files: &mut BTreeMap<String, Vec<u8>>,
        dirs: &mut Vec<String>,
        size: &mut usize,
    ) -> Result<()> {
        let mut entries = fs::read_dir(dir)
            .map_err(err)?
            .collect::<std::io::Result<Vec<_>>>()
            .map_err(err)?;
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            let name = entry
                .file_name()
                .into_string()
                .map_err(|_| "Non-UTF-8 filenames are not supported")?;
            if reserved(&name) {
                continue;
            }
            let path = entry.path();
            let rel = path.strip_prefix(root).map_err(err)?;
            let virtual_path = format!(
                "/{}",
                rel.to_str().ok_or("Non-UTF-8 path")?.replace('\\', "/")
            );
            relative(&virtual_path)?;
            let meta = fs::symlink_metadata(&path).map_err(err)?;
            if meta.file_type().is_symlink() {
                return Err(format!("Symbolic links are not supported: {virtual_path}"));
            }
            // Do not import an embedded repository into the surrounding project.
            if meta.is_dir() && path.join(".git").exists() {
                continue;
            }
            let tracked = index.is_some_and(|i| i.get_path(rel, 0).is_some());
            if !meta.is_dir()
                && !tracked
                && repository.is_some_and(|r| r.status_should_ignore(rel).unwrap_or(false))
            {
                continue;
            }
            if meta.is_dir() {
                if dirs.len() >= MAX_FILES {
                    return Err("Too many project directories".into());
                }
                dirs.push(virtual_path);
                walk(root, &path, repository, index, files, dirs, size)?;
            } else {
                let bytes = read_file(&path)?.ok_or("File disappeared while opening project")?;
                *size += bytes.len();
                if *size > MAX_PROJECT || files.len() >= MAX_FILES {
                    return Err("Project exceeds 256 MiB or 10000 files".into());
                }
                files.insert(virtual_path, bytes);
            }
        }
        Ok(())
    }
    let mut files = BTreeMap::new();
    let mut dirs = vec!["/".to_owned()];
    let mut size = 0;
    walk(
        root,
        root,
        repository.as_ref(),
        index.as_ref(),
        &mut files,
        &mut dirs,
        &mut size,
    )?;
    Ok((files, dirs))
}
fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path.parent().ok_or("Missing parent directory")?;
    fs::create_dir_all(parent).map_err(err)?;
    let temporary = parent.join(format!(".tetorica-save-{}", uuid::Uuid::new_v4()));
    let result = (|| {
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&temporary).map_err(err)?;
        file.write_all(bytes).map_err(err)?;
        if let Ok(meta) = fs::metadata(path) {
            fs::set_permissions(&temporary, meta.permissions()).map_err(err)?;
        }
        file.sync_all().map_err(err)?;
        fs::rename(&temporary, path).map_err(err)
    })();
    if result.is_err() {
        let _ = fs::remove_file(temporary);
    }
    result
}
impl Project {
    pub fn create(root: PathBuf, snapshot: Snapshot) -> Result<(Self, Value)> {
        if fs::read_dir(&root).map_err(err)?.next().is_some() {
            return Err("Choose an empty folder for the current project".into());
        }
        let (mut project, _) = Self::open(root)?;
        let mut value = project.save(snapshot)?;
        let status = project.init()?;
        value["folder"] = json!(project.root.display().to_string());
        value["status"] = status;
        Ok((project, value))
    }
    pub fn open(root: PathBuf) -> Result<(Self, Value)> {
        let root = root.canonicalize().map_err(err)?;
        if !root.is_dir() {
            return Err("Choose a project folder".into());
        }
        let (baseline, dirs) = scan(&root)?;
        let project = Self {
            id: uuid::Uuid::new_v4().to_string(),
            root,
            revision: 1,
            baseline,
        };
        let value = project.snapshot(&dirs);
        Ok((project, value))
    }
    fn snapshot(&self, dirs: &[String]) -> Value {
        json!({"projectId":self.id,"revision":self.revision,"folder":self.root.display().to_string(),
            "snapshot":Snapshot{files:self.baseline.iter().map(|(path,bytes)|File{path:path.clone(),data:STANDARD.encode(bytes)}).collect(),directories:dirs.to_vec()}})
    }
    pub fn check(&self, id: &str, revision: u64) -> Result<()> {
        if self.id != id || self.revision != revision {
            return Err("Project changed; reopen or reload before saving".into());
        }
        Ok(())
    }
    pub fn reload(&mut self) -> Result<Value> {
        let (baseline, dirs) = scan(&self.root)?;
        self.baseline = baseline;
        self.revision += 1;
        Ok(self.snapshot(&dirs))
    }
    pub fn save(&mut self, snapshot: Snapshot) -> Result<Value> {
        verify_git_dir(&self.root)?;
        if snapshot.files.len() > MAX_FILES || snapshot.directories.len() > MAX_FILES {
            return Err("Too many project entries".into());
        }
        let mut next = BTreeMap::new();
        let mut size = 0;
        let mut folded = BTreeSet::new();
        for file in snapshot.files {
            safe_path(&self.root, &file.path)?;
            if !folded.insert(file.path.to_lowercase()) {
                return Err("Duplicate or case-conflicting path".into());
            }
            if file.data.len() > (MAX_FILE * 4 / 3 + 8) {
                return Err("Project file exceeds 64 MiB".into());
            }
            let bytes = STANDARD.decode(file.data).map_err(err)?;
            size += bytes.len();
            if bytes.len() > MAX_FILE || size > MAX_PROJECT {
                return Err("Project exceeds size limit".into());
            }
            next.insert(file.path, bytes);
        }
        for name in next.keys() {
            let mut parent = Path::new(name).parent();
            while let Some(path) = parent {
                if next.contains_key(path.to_str().ok_or("Invalid path")?) {
                    return Err("File blocks directory".into());
                }
                parent = path.parent();
            }
        }
        for dir in &snapshot.directories {
            if dir == "/" {
                continue;
            }
            let path = safe_path(&self.root, dir)?;
            if next.contains_key(dir) || path.exists() && !path.is_dir() {
                return Err("File blocks project directory".into());
            }
        }
        let paths: BTreeSet<_> = self.baseline.keys().chain(next.keys()).cloned().collect();
        // Validate all files before writing any. Even unchanged editor files
        // must not silently replace the baseline of an external modification.
        for name in &paths {
            let path = safe_path(&self.root, name)?;
            if read_file(&path)?.as_ref() != self.baseline.get(name) {
                return Err(format!(
                    "External change: {name}. Reload or resolve it before saving."
                ));
            }
        }
        let changed: Vec<_> = paths
            .into_iter()
            .filter(|p| self.baseline.get(p) != next.get(p))
            .collect();
        let mut applied = Vec::new();
        let result: Result<()> = (|| {
            for name in &changed {
                let path = safe_path(&self.root, name)?;
                if read_file(&path)?.as_ref() != self.baseline.get(name) {
                    return Err(format!("External change while saving: {name}"));
                }
                if let Some(bytes) = next.get(name) {
                    atomic_write(&path, bytes)?;
                } else {
                    fs::remove_file(&path).map_err(err)?;
                }
                applied.push(name.clone());
            }
            for name in &snapshot.directories {
                if name != "/" {
                    fs::create_dir_all(safe_path(&self.root, name)?).map_err(err)?;
                }
            }
            Ok(())
        })();
        if let Err(error) = result {
            // Best-effort rollback. Never overwrite a further external edit.
            for name in applied.iter().rev() {
                if let Ok(path) = safe_path(&self.root, name) {
                    if read_file(&path).ok().flatten().as_ref() == next.get(name) {
                        if let Some(bytes) = self.baseline.get(name) {
                            let _ = atomic_write(&path, bytes);
                        } else {
                            let _ = fs::remove_file(path);
                        }
                    }
                }
            }
            return Err(format!(
                "Save failed; check project files before retrying: {error}"
            ));
        }
        self.baseline = next;
        self.revision += 1;
        Ok(json!({"projectId":self.id,"revision":self.revision,"saved":changed.len()}))
    }
    pub fn init(&self) -> Result<Value> {
        verify_git_dir(&self.root)?;
        Repository::init(&self.root).map_err(err)?;
        self.status()
    }
    pub fn status(&self) -> Result<Value> {
        let repository = match repo(&self.root) {
            Ok(r) => r,
            Err(e) if !self.root.join(".git").exists() => {
                return Ok(json!({"initialized":false,"files":[],"message":e}))
            }
            Err(e) => return Err(e),
        };
        let mut options = StatusOptions::new();
        options.include_untracked(true).recurse_untracked_dirs(true);
        let files: Vec<_> = repository.statuses(Some(&mut options)).map_err(err)?.iter().map(|s|json!({"path":s.path().ok(),"status":s.status().bits(),"staged":s.status().intersects(git2::Status::INDEX_NEW|git2::Status::INDEX_MODIFIED|git2::Status::INDEX_DELETED|git2::Status::INDEX_RENAMED|git2::Status::INDEX_TYPECHANGE),"untracked":s.status().contains(git2::Status::WT_NEW),"worktree":s.status().intersects(git2::Status::WT_MODIFIED|git2::Status::WT_DELETED|git2::Status::WT_RENAMED|git2::Status::WT_TYPECHANGE),"conflict":s.status().contains(git2::Status::CONFLICTED)})).collect();
        Ok(
            json!({"initialized":true,"branch":repository.head().ok().and_then(|h|h.shorthand().ok().map(str::to_owned)),"files":files}),
        )
    }
    pub fn diff(&self, staged: bool) -> Result<Value> {
        let repository = repo(&self.root)?;
        let mut options = DiffOptions::new();
        options
            .include_untracked(true)
            .recurse_untracked_dirs(true)
            .show_untracked_content(true);
        let diff = if staged {
            let tree = repository.head().ok().and_then(|h| h.peel_to_tree().ok());
            repository.diff_tree_to_index(tree.as_ref(), None, Some(&mut options))
        } else {
            repository.diff_index_to_workdir(None, Some(&mut options))
        }
        .map_err(err)?;
        let mut output = Vec::new();
        let mut truncated = false;
        diff.print(git2::DiffFormat::Patch, |_, _, line| {
            if output.len() + line.content().len() > 1024 * 1024 {
                truncated = true;
                return true;
            }
            if matches!(line.origin(), '+' | '-' | ' ') {
                output.push(line.origin() as u8);
            }
            output.extend_from_slice(line.content());
            true
        })
        .map_err(err)?;
        Ok(json!({"text":String::from_utf8_lossy(&output),"truncated":truncated}))
    }
    pub fn stage(&self, paths: &[String]) -> Result<Value> {
        let repository = repo(&self.root)?;
        let mut index = repository.index().map_err(err)?;
        // Only explicitly named, project-managed paths can enter the index.
        for name in paths {
            let path = safe_path(&self.root, name)?;
            let rel = relative(name)?;
            if !self.baseline.contains_key(name) && index.get_path(&rel, 0).is_none() {
                return Err("Save this project file before staging it".into());
            }
            if path.exists() {
                index.add_path(&rel).map_err(err)?;
            } else {
                index.remove_path(&rel).map_err(err)?;
            }
        }
        index.write().map_err(err)?;
        self.status()
    }
    pub fn commit(&self, message: &str, name: &str, email: &str) -> Result<Value> {
        if message.trim().is_empty() || message.len() > 16384 {
            return Err("Enter a commit message (at most 16 KiB)".into());
        }
        let repository = repo(&self.root)?;
        let mut index = repository.index().map_err(err)?;
        if index.has_conflicts() || repository.state() != git2::RepositoryState::Clean {
            return Err("Resolve the repository operation and conflicts before committing".into());
        }
        let tree = repository
            .find_tree(index.write_tree().map_err(err)?)
            .map_err(err)?;
        let parent = repository.head().ok().and_then(|h| h.peel_to_commit().ok());
        if parent.as_ref().is_some_and(|p| p.tree_id() == tree.id()) {
            return Err("No staged changes to commit".into());
        }
        let signature = if name.trim().is_empty() && email.trim().is_empty() {
            repository
                .signature()
                .map_err(|_| "Enter an author name and email for the commit".to_owned())?
        } else {
            Signature::now(name, email).map_err(err)?
        };
        let parents: Vec<_> = parent.iter().collect();
        let oid = repository
            .commit(
                Some("HEAD"),
                &signature,
                &signature,
                message,
                &tree,
                &parents,
            )
            .map_err(err)?;
        Ok(json!({"commit":oid.to_string()}))
    }
    pub fn log(&self) -> Result<Value> {
        let repository = repo(&self.root)?;
        let mut walk = repository.revwalk().map_err(err)?;
        if repository.head().is_err() {
            return Ok(json!([]));
        }
        walk.push_head().map_err(err)?;
        let mut commits = Vec::new();
        for oid in walk.take(50) {
            let commit = repository.find_commit(oid.map_err(err)?).map_err(err)?;
            commits.push(json!({"id":commit.id().to_string(),"message":commit.message().ok(),"author":commit.author().name().ok(),"time":commit.time().seconds()}));
        }
        Ok(json!(commits))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Temp(PathBuf);
    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    fn temp() -> Temp {
        let p = std::env::temp_dir().join(format!("tetorica-git-test-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&p).unwrap();
        Temp(p)
    }
    fn snapshot(files: &[(&str, &[u8])]) -> Snapshot {
        Snapshot {
            files: files
                .iter()
                .map(|(p, b)| File {
                    path: p.to_string(),
                    data: STANDARD.encode(b),
                })
                .collect(),
            directories: vec![],
        }
    }
    #[test]
    fn create_current_project_in_empty_folder() {
        let t = temp();
        let (p, value) = Project::create(
            t.0.clone(),
            snapshot(&[
                ("/index.js", b"current code"),
                ("/sample.bin", &[0, 128, 255]),
            ]),
        )
        .unwrap();
        assert_eq!(fs::read(t.0.join("index.js")).unwrap(), b"current code");
        assert_eq!(fs::read(t.0.join("sample.bin")).unwrap(), [0, 128, 255]);
        assert_eq!(value["status"]["initialized"], true);
        p.check(
            value["projectId"].as_str().unwrap(),
            value["revision"].as_u64().unwrap(),
        )
        .unwrap();
        assert!(Project::create(t.0.clone(), snapshot(&[("/index.js", b"replacement")])).is_err());
        assert_eq!(fs::read(t.0.join("index.js")).unwrap(), b"current code");
    }
    #[test]
    fn status_and_diff_distinguish_staged_and_editor_changes() {
        let t = temp();
        let (mut p, _) = Project::create(t.0.clone(), snapshot(&[("/index.js", b"first\n")])).unwrap();
        assert_eq!(p.status().unwrap()["files"][0]["untracked"], true);
        assert!(p.diff(false).unwrap()["text"].as_str().unwrap().contains("first"));
        p.stage(&["/index.js".into()]).unwrap();
        assert_eq!(p.status().unwrap()["files"][0]["staged"], true);
        assert_eq!(p.diff(false).unwrap()["text"], "");
        assert!(p.diff(true).unwrap()["text"].as_str().unwrap().contains("first"));
        p.commit("first", "Test", "test@example.invalid").unwrap();
        p.save(snapshot(&[("/index.js", b"edited\n")])).unwrap();
        assert_eq!(p.status().unwrap()["files"][0]["worktree"], true);
        assert!(p.diff(false).unwrap()["text"].as_str().unwrap().contains("edited"));
        assert_eq!(p.diff(true).unwrap()["text"], "");
    }
    #[test]
    fn sync_and_local_git_without_cli() {
        let t = temp();
        fs::write(t.0.join("index.js"), "old").unwrap();
        let (mut p, _) = Project::open(t.0.clone()).unwrap();
        p.save(snapshot(&[
            ("/index.js", b"new"),
            ("/sample.bin", &[0, 128, 255]),
        ]))
        .unwrap();
        assert_eq!(fs::read(t.0.join("sample.bin")).unwrap(), [0, 128, 255]);
        p.init().unwrap();
        p.stage(&["/index.js".into(), "/sample.bin".into()])
            .unwrap();
        assert!(p.diff(true).unwrap()["text"]
            .as_str()
            .unwrap()
            .contains("new"));
        p.commit("first", "Test", "test@example.invalid").unwrap();
        assert_eq!(p.log().unwrap().as_array().unwrap().len(), 1);
        assert_eq!(p.status().unwrap()["files"].as_array().unwrap().len(), 0);
        fs::write(t.0.join("unmanaged.txt"), "keep").unwrap();
        p.save(snapshot(&[("/index.js", b"new")])).unwrap();
        assert!(!t.0.join("sample.bin").exists());
        assert!(t.0.join("unmanaged.txt").exists());
    }
    #[test]
    fn external_changes_and_stale_revisions_are_rejected_before_writes() {
        let t = temp();
        fs::write(t.0.join("a"), "old").unwrap();
        let (mut p, _) = Project::open(t.0.clone()).unwrap();
        fs::write(t.0.join("a"), "human").unwrap();
        assert!(p
            .save(snapshot(&[("/a", b"app"), ("/b", b"new")]))
            .unwrap_err()
            .contains("External change"));
        assert!(!t.0.join("b").exists());
        assert!(p.check("stale", 1).is_err());
        assert!(p.check(&p.id, 2).is_err());
    }
    #[test]
    fn reserved_paths_and_case_collisions_are_rejected() {
        let t = temp();
        let (mut p, _) = Project::open(t.0.clone()).unwrap();
        for path in [
            "/.git/config",
            "/x/../a",
            "/x/.GIT/key",
            "/.ssh/id",
            "/C:/x",
            "/sys/x",
        ] {
            assert!(p.save(snapshot(&[(path, b"bad")])).is_err(), "{path}");
        }
        assert!(p.save(snapshot(&[("/A", b"a"), ("/a", b"b")])).is_err());
    }
    #[cfg(unix)]
    #[test]
    fn links_and_linked_git_dirs_are_rejected() {
        use std::os::unix::fs::symlink;
        let t = temp();
        let out = temp();
        symlink(&out.0, t.0.join("link")).unwrap();
        assert!(Project::open(t.0.clone()).is_err());
        fs::remove_file(t.0.join("link")).unwrap();
        let (mut p, _) = Project::open(t.0.clone()).unwrap();
        symlink(&out.0, t.0.join("link")).unwrap();
        assert!(p.save(snapshot(&[("/link/key", b"bad")])).is_err());
        assert!(!out.0.join("key").exists());
        symlink(&out.0, t.0.join(".git")).unwrap();
        assert!(p.init().is_err());
    }
}
