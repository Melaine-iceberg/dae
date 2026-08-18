use super::error::FileSystemError;
use super::types::path_to_string;
use git2::{Repository, Status, StatusOptions};
use serde::{Deserialize, Serialize};
use specta::Type;
use std::collections::HashMap;

/// Git 工作区徽标类型，对应文件管理器中的 M/A/U 状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "lowercase")]
pub enum GitEntryStatusKind {
    Modified,
    Added,
    Untracked,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct GitEntryStatus {
    /// 当前目录下直接子项的名称（目录为其本身名称）。
    pub name: String,
    pub kind: GitEntryStatusKind,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct GitDirectoryStatus {
    /// 仓库工作区根路径。
    pub root: String,
    /// 当前分支名；分离 HEAD 时为短提交号。
    pub branch: String,
    /// 当前目录直接子项的状态徽标（目录为其后代状态的聚合）。
    pub entries: Vec<GitEntryStatus>,
    /// 当前目录本身整体未跟踪（git 将其折叠为一条未跟踪记录），
    /// 此时目录内所有子项都应显示 U 徽标。
    pub directory_untracked: bool,
}

/// 读取一个目录的 Git 装饰信息：当前分支与直接子项的 M/A/U 徽标。
/// 目录不在 Git 工作区内时返回 `None`。git2 的状态扫描在阻塞线程池
/// 执行，不会阻塞异步运行时或 UI。
#[tauri::command]
#[specta::specta]
pub async fn get_git_status(path: String) -> Result<Option<GitDirectoryStatus>, FileSystemError> {
    tauri::async_runtime::spawn_blocking(move || git_status(&path))
        .await
        .map_err(|error| FileSystemError::Internal(error.to_string()))?
}

fn git_status(dir: &str) -> Result<Option<GitDirectoryStatus>, FileSystemError> {
    let repo = match Repository::discover(dir) {
        Ok(repo) => repo,
        Err(_) => return Ok(None),
    };

    // 裸仓库没有工作区可供装饰。
    let workdir = match repo.workdir() {
        Some(workdir) => path_to_string(workdir),
        None => return Ok(None),
    };

    let branch = branch_name(&repo);

    let mut options = StatusOptions::new();
    options
        .include_untracked(true)
        .recurse_untracked_dirs(false)
        .include_ignored(false);
    let statuses = repo
        .statuses(Some(&mut options))
        .map_err(|error| FileSystemError::Internal(error.to_string()))?;

    let Some(relative_segments) = relative_segments(&workdir, dir) else {
        return Ok(None);
    };
    let depth = relative_segments.len();

    // 聚合当前目录直接子项的状态：文件取自身状态，目录取后代状态中
    // 优先级最高者（M > A > U）。
    let mut directory_untracked = false;
    let mut badges: HashMap<String, GitEntryStatusKind> = HashMap::new();
    for entry in statuses.iter() {
        let Ok(status_path) = entry.path() else { continue };
        let Some(kind) = classify(entry.status()) else { continue };

        // git2 返回以 `/` 分隔、相对工作区根的路径；未跟踪目录带尾部 `/`。
        let parts: Vec<&str> = status_path
            .split('/')
            .filter(|segment| !segment.is_empty())
            .collect();

        // 恰好与当前目录等深的未跟踪折叠条目：当前目录整体未跟踪。
        if depth > 0
            && parts.len() == depth
            && kind == GitEntryStatusKind::Untracked
            && parts
                .iter()
                .zip(&relative_segments)
                .all(|(segment, expected)| segment.eq_ignore_ascii_case(expected))
        {
            directory_untracked = true;
            continue;
        }

        if parts.len() <= depth {
            continue;
        }
        // Windows 路径大小写不敏感，逐段忽略大小写比较。
        if parts[..depth]
            .iter()
            .zip(&relative_segments)
            .any(|(segment, expected)| !segment.eq_ignore_ascii_case(expected))
        {
            continue;
        }

        let name = parts[depth].to_string();
        badges
            .entry(name)
            .and_modify(|current| *current = merge(*current, kind))
            .or_insert(kind);
    }

    let mut entries: Vec<GitEntryStatus> = badges
        .into_iter()
        .map(|(name, kind)| GitEntryStatus { name, kind })
        .collect();
    entries.sort_by(|left, right| left.name.cmp(&right.name));

    Ok(Some(GitDirectoryStatus {
        root: workdir,
        branch,
        entries,
        directory_untracked,
    }))
}

/// 将 git2 状态位映射为单个徽标：已暂存新增 (A) > 未跟踪 (U) > 已修改 (M)。
/// 仅删除/忽略等磁盘上不可见的变更被跳过（它们不会出现在文件列表中）。
fn classify(status: Status) -> Option<GitEntryStatusKind> {
    if status.contains(Status::INDEX_NEW) {
        Some(GitEntryStatusKind::Added)
    } else if status.contains(Status::WT_NEW) {
        Some(GitEntryStatusKind::Untracked)
    } else if status.intersects(
        Status::INDEX_MODIFIED
            | Status::WT_MODIFIED
            | Status::INDEX_RENAMED
            | Status::WT_RENAMED
            | Status::INDEX_TYPECHANGE
            | Status::WT_TYPECHANGE
            | Status::CONFLICTED,
    ) {
        Some(GitEntryStatusKind::Modified)
    } else {
        None
    }
}

/// 目录聚合优先级：修改 (M) > 新增 (A) > 未跟踪 (U)。
fn merge(left: GitEntryStatusKind, right: GitEntryStatusKind) -> GitEntryStatusKind {
    let rank = |kind: GitEntryStatusKind| match kind {
        GitEntryStatusKind::Modified => 2,
        GitEntryStatusKind::Added => 1,
        GitEntryStatusKind::Untracked => 0,
    };
    if rank(left) >= rank(right) {
        left
    } else {
        right
    }
}

fn branch_name(repo: &Repository) -> String {
    if let Ok(head) = repo.head() {
        if repo.head_detached().unwrap_or(false) {
            if let Some(target) = head.target() {
                let commit = target.to_string();
                return commit[..7.min(commit.len())].to_string();
            }
        }
        if let Ok(shorthand) = head.shorthand() {
            if !shorthand.is_empty() {
                return shorthand.to_string();
            }
        }
    } else if let Ok(head) = repo.find_reference("HEAD") {
        // 尚无提交的仓库：HEAD 符号指向未诞生的分支，如 refs/heads/main。
        if let Ok(Some(target)) = head.symbolic_target() {
            let branch = target.rsplit('/').next().unwrap_or(target);
            if !branch.is_empty() {
                return branch.to_string();
            }
        }
    }

    "HEAD".to_string()
}

/// 计算 `dir` 相对工作区根的路径段；不在工作区内时返回 `None`。
/// git2 与前端可能使用不同的分隔符/大小写形式，因此按段归一化比较。
fn relative_segments(workdir: &str, dir: &str) -> Option<Vec<String>> {
    let split_segments = |path: &str| -> Vec<String> {
        path.split(['/', '\\'])
            .filter(|segment| !segment.is_empty())
            .map(String::from)
            .collect()
    };

    let workdir_segments = split_segments(workdir.trim_end_matches(['/', '\\']));
    let dir_segments = split_segments(dir.trim_end_matches(['/', '\\']));
    if dir_segments.len() < workdir_segments.len() {
        return None;
    }

    for (segment, expected) in dir_segments.iter().zip(&workdir_segments) {
        if !segment.eq_ignore_ascii_case(expected) {
            return None;
        }
    }

    Some(dir_segments[workdir_segments.len()..].to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;
    use git2::IndexAddOption;
    use std::fs;
    use std::path::PathBuf;

    struct TempRepo {
        path: PathBuf,
    }

    impl TempRepo {
        fn create(name: &str) -> Self {
            let path = std::env::temp_dir().join(format!("dae-git-test-{}-{name}", std::process::id()));
            let _ = fs::remove_dir_all(&path);
            fs::create_dir_all(&path).expect("create temp dir");

            let repo = Repository::init(&path).expect("init repo");
            repo.config()
                .expect("repo config")
                .set_str("user.name", "Test")
                .expect("set user.name");
            repo.config()
                .expect("repo config")
                .set_str("user.email", "test@example.com")
                .expect("set user.email");

            Self { path }
        }

        fn repo(&self) -> Repository {
            Repository::open(&self.path).expect("open repo")
        }

        fn path_string(&self) -> String {
            path_to_string(&self.path)
        }
    }

    impl Drop for TempRepo {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    fn commit_all(repo: &Repository, message: &str) {
        let mut index = repo.index().expect("index");
        index
            .add_all(["*"].iter(), IndexAddOption::DEFAULT, None)
            .expect("add all");
        index.write().expect("write index");

        let tree_id = index.write_tree().expect("write tree");
        let tree = repo.find_tree(tree_id).expect("find tree");
        let signature = repo.signature().expect("signature");

        let head_commit = repo
            .head()
            .ok()
            .and_then(|head| head.target())
            .and_then(|target| repo.find_commit(target).ok());
        let parents: Vec<&git2::Commit> = head_commit.iter().collect();

        repo.commit(Some("HEAD"), &signature, &signature, message, &tree, &parents)
            .expect("commit");
    }

    #[test]
    fn returns_none_outside_a_repository() {
        // 不初始化仓库；同时确保目录不在任何上层 Git 工作区内。
        let dir = std::env::temp_dir().join(format!("dae-git-test-{}-outside", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("create dir");

        let result = git_status(&path_to_string(&dir)).expect("status call");
        let _ = fs::remove_dir_all(&dir);
        assert!(result.is_none());
    }

    #[test]
    fn reports_untracked_and_branch() {
        let temp = TempRepo::create("untracked");
        fs::write(temp.path.join("a.txt"), "hello").expect("write file");

        let status = git_status(&temp.path_string())
            .expect("status call")
            .expect("inside repo");
        assert!(!status.branch.is_empty());
        assert_eq!(status.entries.len(), 1);
        assert_eq!(status.entries[0].name, "a.txt");
        assert_eq!(status.entries[0].kind, GitEntryStatusKind::Untracked);
    }

    #[test]
    fn staged_new_file_is_added() {
        let temp = TempRepo::create("staged");
        fs::write(temp.path.join("a.txt"), "hello").expect("write file");

        let repo = temp.repo();
        let mut index = repo.index().expect("index");
        index.add_path(std::path::Path::new("a.txt")).expect("stage");
        index.write().expect("write index");

        let status = git_status(&temp.path_string())
            .expect("status call")
            .expect("inside repo");
        assert_eq!(status.entries[0].kind, GitEntryStatusKind::Added);
    }

    #[test]
    fn committed_then_edited_file_is_modified() {
        let temp = TempRepo::create("modified");
        fs::write(temp.path.join("a.txt"), "v1").expect("write file");
        commit_all(&temp.repo(), "initial");

        fs::write(temp.path.join("a.txt"), "v2").expect("edit file");

        let status = git_status(&temp.path_string())
            .expect("status call")
            .expect("inside repo");
        assert_eq!(status.entries[0].kind, GitEntryStatusKind::Modified);
    }

    #[test]
    fn untracked_subdirectory_aggregates_on_parent_and_resolves_inside() {
        let temp = TempRepo::create("subdir");
        let sub = temp.path.join("sub");
        fs::create_dir_all(&sub).expect("create sub");
        fs::write(sub.join("b.txt"), "hello").expect("write file");

        // 在仓库根目录查看：整个未跟踪目录折叠为一条 U 徽标。
        let root_status = git_status(&temp.path_string())
            .expect("status call")
            .expect("inside repo");
        assert!(!root_status.directory_untracked);
        assert_eq!(root_status.entries.len(), 1);
        assert_eq!(root_status.entries[0].name, "sub");
        assert_eq!(root_status.entries[0].kind, GitEntryStatusKind::Untracked);

        // 进入未跟踪子目录查看：目录整体未跟踪，前端据此为所有子项标 U。
        let sub_status = git_status(&path_to_string(&sub))
            .expect("status call")
            .expect("inside repo");
        assert!(sub_status.directory_untracked);
        assert!(sub_status.entries.is_empty());
    }

    #[test]
    fn mixed_children_aggregate_to_modified_on_directory() {
        let temp = TempRepo::create("aggregate");
        let sub = temp.path.join("sub");
        fs::create_dir_all(&sub).expect("create sub");
        fs::write(sub.join("tracked.txt"), "v1").expect("write file");
        commit_all(&temp.repo(), "initial");

        // 子目录内一个修改文件 + 一个未跟踪文件：目录徽标聚合为 M。
        fs::write(sub.join("tracked.txt"), "v2").expect("edit file");
        fs::write(sub.join("new.txt"), "untracked").expect("write file");

        let status = git_status(&temp.path_string())
            .expect("status call")
            .expect("inside repo");
        let sub_entry = status
            .entries
            .iter()
            .find(|entry| entry.name == "sub")
            .expect("sub badge");
        assert_eq!(sub_entry.kind, GitEntryStatusKind::Modified);
    }
}
