use super::error::FileSystemError;
use super::types::path_to_string;
use git2::build::CheckoutBuilder;
use git2::{BranchType, ErrorCode, Reference, Repository, Status, StatusOptions};
use serde::{Deserialize, Serialize};
use specta::Type;
use std::collections::HashMap;
use std::process::Command;

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
    let Some(repo) = discover_worktree(dir)? else {
        return Ok(None);
    };
    // 裸仓库没有工作区可供装饰。
    let workdir = path_to_string(repo.workdir().expect("workdir verified above"));

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

/// 分支条目：`name` 为完整简名（本地 "main"，远程 "origin/main"），
/// 远程分支的 `short_name` 去掉远程前缀。
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct GitBranchInfo {
    pub name: String,
    pub short_name: String,
    pub is_current: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct GitBranches {
    /// 仓库工作区根路径。
    pub root: String,
    /// 当前分支名（分离 HEAD 时为短提交号，见 `detached`）。
    pub branch: String,
    pub detached: bool,
    /// 当前分支是否配置了上游分支。
    pub has_upstream: bool,
    /// 相对上游的领先/落后提交数（无上游时均为 0）。
    pub ahead: u32,
    pub behind: u32,
    /// 配置的远程仓库名称。
    pub remotes: Vec<String>,
    pub local: Vec<GitBranchInfo>,
    /// 已知的远程跟踪分支（来自 refs/remotes/*，可能尚未拉取最新）。
    pub remote: Vec<GitBranchInfo>,
}

/// 列出仓库的本地与远程分支、当前分支及其相对上游的领先/落后计数。
/// 目录不在 Git 工作区内时返回 `None`。
#[tauri::command]
#[specta::specta]
pub async fn list_git_branches(path: String) -> Result<Option<GitBranches>, FileSystemError> {
    tauri::async_runtime::spawn_blocking(move || branches(&path))
        .await
        .map_err(|error| FileSystemError::Internal(error.to_string()))?
}

/// 切换分支。`remote` 为真时 `name` 形如 "origin/feature"，
/// 会创建同名的本地跟踪分支再切换。
#[tauri::command]
#[specta::specta]
pub async fn git_checkout_branch(
    path: String,
    name: String,
    remote: bool,
) -> Result<(), FileSystemError> {
    tauri::async_runtime::spawn_blocking(move || checkout_branch(&path, &name, remote))
        .await
        .map_err(|error| FileSystemError::Internal(error.to_string()))?
}

/// 基于当前 HEAD 创建分支；`checkout` 为真时同时切换过去。
#[tauri::command]
#[specta::specta]
pub async fn git_create_branch(
    path: String,
    name: String,
    checkout: bool,
) -> Result<(), FileSystemError> {
    tauri::async_runtime::spawn_blocking(move || create_branch(&path, &name, checkout))
        .await
        .map_err(|error| FileSystemError::Internal(error.to_string()))?
}

/// 从远程仓库获取更新（fetch --prune）。走 git 命令行，复用系统凭据
/// （凭据管理器 / SSH agent），本地操作仍由 git2 完成。
#[tauri::command]
#[specta::specta]
pub async fn git_fetch(path: String) -> Result<(), FileSystemError> {
    tauri::async_runtime::spawn_blocking(move || fetch(&path))
        .await
        .map_err(|error| FileSystemError::Internal(error.to_string()))?
}

/// 推送当前分支；无上游时自动 `push --set-upstream` 到首个远程。
#[tauri::command]
#[specta::specta]
pub async fn git_push(path: String) -> Result<(), FileSystemError> {
    tauri::async_runtime::spawn_blocking(move || push(&path))
        .await
        .map_err(|error| FileSystemError::Internal(error.to_string()))?
}

/// 拉取当前分支（fetch + merge/rebase，遵循仓库配置）。
#[tauri::command]
#[specta::specta]
pub async fn git_pull(path: String) -> Result<(), FileSystemError> {
    tauri::async_runtime::spawn_blocking(move || pull(&path))
        .await
        .map_err(|error| FileSystemError::Internal(error.to_string()))?
}

/// 同步：有上游时先拉取再推送；无上游时发布分支（push --set-upstream）。
#[tauri::command]
#[specta::specta]
pub async fn git_sync(path: String) -> Result<(), FileSystemError> {
    tauri::async_runtime::spawn_blocking(move || sync(&path))
        .await
        .map_err(|error| FileSystemError::Internal(error.to_string()))?
}

fn branches(dir: &str) -> Result<Option<GitBranches>, FileSystemError> {
    let Some(repo) = discover_worktree(dir)? else {
        return Ok(None);
    };

    let detached = repo.head_detached().unwrap_or(false);
    let branch = branch_name(&repo);

    let remotes: Vec<String> = safe_remotes(&repo);

    let mut ahead = 0;
    let mut behind = 0;
    let mut has_upstream = false;
    let mut local: Vec<GitBranchInfo> = Vec::new();
    if let Ok(iter) = repo.branches(Some(BranchType::Local)) {
        for entry in iter.flatten() {
            let (git_branch, _) = entry;
            let Some(name) = git_branch.name().ok().flatten() else {
                continue;
            };
            let is_current = git_branch.is_head();
            if is_current && let Ok(upstream) = git_branch.upstream() {
                has_upstream = true;
                if let (Some(local_oid), Some(upstream_oid)) =
                    (git_branch.get().target(), upstream.get().target())
                    && let Ok((ahead_count, behind_count)) =
                        repo.graph_ahead_behind(local_oid, upstream_oid)
                {
                    ahead = ahead_count as u32;
                    behind = behind_count as u32;
                }
            }
            local.push(GitBranchInfo {
                name: name.to_string(),
                short_name: name.to_string(),
                is_current,
            });
        }
    }
    // 当前分支置顶，其余按名称排序。
    local.sort_by(|left, right| {
        right
            .is_current
            .cmp(&left.is_current)
            .then_with(|| left.name.cmp(&right.name))
    });

    let mut remote: Vec<GitBranchInfo> = Vec::new();
    if let Ok(iter) = repo.branches(Some(BranchType::Remote)) {
        for entry in iter.flatten() {
            let (git_branch, _) = entry;
            let Some(name) = git_branch.name().ok().flatten() else {
                continue;
            };
            // refs/remotes/origin/HEAD 指向默认分支，不是可切换的分支。
            if name.ends_with("/HEAD") {
                continue;
            }
            let short_name = name
                .split_once('/')
                .map(|(_, short)| short)
                .unwrap_or(name)
                .to_string();
            remote.push(GitBranchInfo {
                name: name.to_string(),
                short_name,
                is_current: false,
            });
        }
    }
    remote.sort_by(|left, right| left.name.cmp(&right.name));

    let root = path_to_string(repo.workdir().expect("workdir verified above"));
    Ok(Some(GitBranches {
        root,
        branch,
        detached,
        has_upstream,
        ahead,
        behind,
        remotes,
        local,
        remote,
    }))
}

fn checkout_branch(dir: &str, name: &str, remote_branch: bool) -> Result<(), FileSystemError> {
    let Some(repo) = discover_worktree(dir)? else {
        return Err(FileSystemError::NotFound("git.not_a_worktree".into()));
    };

    // 目标提交与目标引用名。检出顺序很关键：先 checkout_tree（以当前
    // HEAD 为基线更新工作区），再 set_head —— 反过来会让基线等于目标，
    // 检出变成空操作。
    let (commit, reference_name) = if remote_branch {
        // "origin/feature" → 创建本地跟踪分支 "feature"。
        let (_, short) = name
            .split_once('/')
            .ok_or_else(|| FileSystemError::InvalidInput("git.branch_not_found".into()))?;
        let remote_ref = repo
            .find_branch(name, BranchType::Remote)
            .map_err(|_| FileSystemError::InvalidInput("git.branch_not_found".into()))?;
        if repo.find_branch(short, BranchType::Local).is_ok() {
            return Err(FileSystemError::AlreadyExists("git.branch_exists".into()));
        }
        let commit = remote_ref
            .get()
            .peel_to_commit()
            .map_err(|error| FileSystemError::Internal(format!("git.checkout_failed: {error}")))?;
        let mut local = repo
            .branch(short, &commit, false)
            .map_err(|error| FileSystemError::Internal(format!("git.checkout_failed: {error}")))?;
        local
            .set_upstream(Some(name))
            .map_err(|error| FileSystemError::Internal(format!("git.checkout_failed: {error}")))?;
        let name = local
            .get()
            .name()
            .expect("branch reference has a name")
            .to_string();
        (commit, name)
    } else {
        let reference = repo
            .find_reference(&format!("refs/heads/{name}"))
            .map_err(|_| FileSystemError::InvalidInput("git.branch_not_found".into()))?;
        let commit = reference
            .peel_to_commit()
            .map_err(|_| FileSystemError::InvalidInput("git.branch_not_found".into()))?;
        let name = reference
            .name()
            .map_err(|_| FileSystemError::InvalidInput("git.branch_not_found".into()))?
            .to_string();
        (commit, name)
    };

    // SAFE 策略：工作区有会被覆盖的未提交修改时失败，不会丢失数据。
    repo.checkout_tree(commit.as_object(), Some(&mut CheckoutBuilder::new()))
        .map_err(|error| FileSystemError::Internal(format!("git.checkout_failed: {error}")))?;
    repo.set_head(&reference_name)
        .map_err(|error| FileSystemError::Internal(format!("git.checkout_failed: {error}")))?;
    Ok(())
}

fn create_branch(dir: &str, name: &str, checkout: bool) -> Result<(), FileSystemError> {
    let Some(repo) = discover_worktree(dir)? else {
        return Err(FileSystemError::NotFound("git.not_a_worktree".into()));
    };

    // 拒绝以 '-' 开头的名称：该名称会原样传给 git 命令行（push -u），
    // 也要求以字母数字开头以避免参数注入。
    if name.is_empty()
        || !name
            .chars()
            .next()
            .is_some_and(|first| first.is_ascii_alphanumeric())
        || !Reference::is_valid_name(&format!("refs/heads/{name}"))
    {
        return Err(FileSystemError::InvalidInput("git.invalid_branch_name".into()));
    }

    let head = repo
        .head()
        .map_err(|_| FileSystemError::InvalidInput("git.empty_repo".into()))?;
    let commit = head
        .peel_to_commit()
        .map_err(|_| FileSystemError::InvalidInput("git.empty_repo".into()))?;

    let branch = repo.branch(name, &commit, false).map_err(|error| {
        if error.code() == ErrorCode::Exists {
            FileSystemError::AlreadyExists("git.branch_exists".into())
        } else {
            FileSystemError::Internal(format!("git.create_branch_failed: {error}"))
        }
    })?;

    if checkout {
        let reference_name = branch.get().name().expect("branch reference has a name");
        repo.set_head(reference_name)
            .map_err(|error| FileSystemError::Internal(format!("git.create_branch_failed: {error}")))?;
        repo.checkout_head(Some(&mut CheckoutBuilder::new()))
            .map_err(|error| {
                FileSystemError::Internal(format!("git.create_branch_failed: {error}"))
            })?;
    }
    Ok(())
}

fn fetch(dir: &str) -> Result<(), FileSystemError> {
    let Some(repo) = discover_worktree(dir)? else {
        return Err(FileSystemError::NotFound("git.not_a_worktree".into()));
    };
    let root = path_to_string(repo.workdir().expect("workdir verified above"));
    if safe_remotes(&repo).is_empty() {
        return Err(FileSystemError::InvalidInput("git.no_remote".into()));
    }
    run_git(&root, &["fetch", "--prune"], "git.fetch_failed")?;
    Ok(())
}

fn push(dir: &str) -> Result<(), FileSystemError> {
    let (repo, root, branch) = repo_for_network(dir)?;
    if upstream_of(&repo, &branch)?.is_empty() {
        let remote = safe_remotes(&repo)
            .into_iter()
            .next()
            .ok_or_else(|| FileSystemError::InvalidInput("git.no_remote".into()))?;
        run_git(
            &root,
            &["push", "--set-upstream", &remote, &branch],
            "git.push_failed",
        )?;
    } else {
        run_git(&root, &["push"], "git.push_failed")?;
    }
    Ok(())
}

fn pull(dir: &str) -> Result<(), FileSystemError> {
    let (repo, root, branch) = repo_for_network(dir)?;
    if upstream_of(&repo, &branch)?.is_empty() {
        return Err(FileSystemError::InvalidInput("git.no_upstream".into()));
    }
    run_git(&root, &["pull", "--no-edit"], "git.pull_failed")?;
    Ok(())
}

fn sync(dir: &str) -> Result<(), FileSystemError> {
    let (repo, root, branch) = repo_for_network(dir)?;
    if upstream_of(&repo, &branch)?.is_empty() {
        // 分支尚未发布：先推送建立上游，之后无需再拉取。
        let remote = safe_remotes(&repo)
            .into_iter()
            .next()
            .ok_or_else(|| FileSystemError::InvalidInput("git.no_remote".into()))?;
        run_git(
            &root,
            &["push", "--set-upstream", &remote, &branch],
            "git.push_failed",
        )?;
    } else {
        run_git(&root, &["pull", "--no-edit"], "git.pull_failed")?;
        run_git(&root, &["push"], "git.push_failed")?;
    }
    Ok(())
}

/// 打开工作区并返回 (仓库, 根路径, 当前分支名)；分离 HEAD 或空仓库时报错。
fn repo_for_network(dir: &str) -> Result<(Repository, String, String), FileSystemError> {
    let Some(repo) = discover_worktree(dir)? else {
        return Err(FileSystemError::NotFound("git.not_a_worktree".into()));
    };
    if repo.head_detached().unwrap_or(false) {
        return Err(FileSystemError::InvalidInput("git.detached_head".into()));
    }
    let branch = current_branch(&repo)?;
    let root = path_to_string(repo.workdir().expect("workdir verified above"));
    Ok((repo, root, branch))
}

fn current_branch(repo: &Repository) -> Result<String, FileSystemError> {
    if let Ok(head) = repo.head()
        && let Ok(shorthand) = head.shorthand()
        && !shorthand.is_empty()
    {
        return Ok(shorthand.to_string());
    }
    if let Ok(head) = repo.find_reference("HEAD")
        && let Ok(Some(target)) = head.symbolic_target()
        && let Some(branch) = target.rsplit('/').next()
        && !branch.is_empty()
    {
        return Ok(branch.to_string());
    }
    Err(FileSystemError::InvalidInput("git.detached_head".into()))
}

/// 当前分支的上游完整引用名；未配置时为空字符串。
fn upstream_of(repo: &Repository, branch: &str) -> Result<String, FileSystemError> {
    repo.branch_upstream_name(&format!("refs/heads/{branch}"))
        .map(|buf| buf.as_str().unwrap_or_default().to_string())
        .map_err(|error| FileSystemError::Internal(format!("git.push_failed: {error}")))
}

/// 远程名称列表，过滤掉以 '-' 开头（可能被解析为命令行参数）的名称。
fn safe_remotes(repo: &Repository) -> Vec<String> {
    repo.remotes()
        .map(|remotes| {
            remotes
                .iter()
                .flatten()
                .flatten()
                .filter(|name| !name.starts_with('-'))
                .map(String::from)
                .collect()
        })
        .unwrap_or_default()
}

/// 在 `dir` 下发现 Git 工作区；不在工作区内时返回 `None`（裸仓库同样返回 `None`）。
fn discover_worktree(dir: &str) -> Result<Option<Repository>, FileSystemError> {
    let repo = match Repository::discover(dir) {
        Ok(repo) => repo,
        Err(_) => return Ok(None),
    };
    if repo.workdir().is_none() {
        return Ok(None);
    }
    Ok(Some(repo))
}

/// 执行 git 命令行子命令，失败时返回 `error_code: <stderr>`。
/// 推送/拉取等网络操作依赖用户系统里的凭据配置（凭据管理器、SSH 等），
/// 这是 git2 无法覆盖的部分；`GIT_TERMINAL_PROMPT=0` 让隐藏终端的
/// 交互式提示直接失败而不是挂起。
fn run_git(root: &str, args: &[&str], error_code: &str) -> Result<String, FileSystemError> {
    let mut command = Command::new("git");
    command
        .arg("-C")
        .arg(root)
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0");

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // CREATE_NO_WINDOW：不闪出控制台窗口。
        command.creation_flags(0x0800_0000);
    }

    let output = command.output().map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            FileSystemError::Internal("git.git_not_found".into())
        } else {
            FileSystemError::Internal(format!("git.git_not_found: {error}"))
        }
    })?;

    if output.status.success() {
        return Ok(String::from_utf8_lossy(&output.stdout).trim().to_string());
    }

    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    // pull 的进度/冲突信息可能在 stdout，stderr 为空时回退到它。
    let detail = if stderr.is_empty() { stdout } else { stderr };
    Err(FileSystemError::Internal(format!("{error_code}: {detail}")))
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
        if repo.head_detached().unwrap_or(false)
            && let Some(target) = head.target()
        {
            let commit = target.to_string();
            return commit[..7.min(commit.len())].to_string();
        }
        if let Ok(shorthand) = head.shorthand()
            && !shorthand.is_empty()
        {
            return shorthand.to_string();
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

            // 显式指定初始分支为 main，避免受系统默认配置（如 master）影响。
            let repo = Repository::init_opts(
                &path,
                git2::RepositoryInitOptions::new().initial_head("main"),
            )
            .expect("init repo");
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

    /// 提交两个版本：第一个提交（旧）与 HEAD（新）都修改 a.txt。
    /// 返回旧提交号，供模拟远程分支引用使用。
    fn commit_two_versions(temp: &TempRepo) -> git2::Oid {
        fs::write(temp.path.join("a.txt"), "v1").expect("write file");
        commit_all(&temp.repo(), "first");
        let old_commit = temp
            .repo()
            .head()
            .expect("head")
            .target()
            .expect("target");

        fs::write(temp.path.join("a.txt"), "v2").expect("write file");
        commit_all(&temp.repo(), "second");
        old_commit
    }

    #[test]
    fn lists_local_and_remote_branches() {
        let temp = TempRepo::create("branches");
        let old_commit = commit_two_versions(&temp);

        let repo = temp.repo();
        let head_commit = repo.head().expect("head").target().expect("target");
        repo.branch("dev", &repo.find_commit(head_commit).expect("commit"), false)
            .expect("create branch");
        repo.remote("origin", "https://example.com/repo.git")
            .expect("add remote");
        repo.reference(
            "refs/remotes/origin/main",
            old_commit,
            true,
            "simulate fetch",
        )
        .expect("remote ref");

        let branches = branches(&temp.path_string())
            .expect("branches call")
            .expect("inside repo");
        assert_eq!(branches.branch, "main");
        assert!(!branches.detached);
        assert_eq!(branches.remotes, vec!["origin".to_string()]);
        assert!(!branches.has_upstream);

        let local_names: Vec<&str> = branches
            .local
            .iter()
            .map(|branch| branch.name.as_str())
            .collect();
        assert_eq!(local_names, vec!["main", "dev"]);
        assert!(branches.local[0].is_current);
        assert!(!branches.local[1].is_current);

        assert_eq!(branches.remote.len(), 1);
        assert_eq!(branches.remote[0].name, "origin/main");
        assert_eq!(branches.remote[0].short_name, "main");
    }

    #[test]
    fn create_branch_with_and_without_checkout() {
        let temp = TempRepo::create("create-branch");
        commit_two_versions(&temp);

        // 不切换：HEAD 仍在 main。
        create_branch(&temp.path_string(), "dev", false).expect("create branch");
        let repo = temp.repo();
        assert_eq!(repo.head().expect("head").shorthand().expect("name"), "main");

        // 切换：HEAD 移到新分支。
        create_branch(&temp.path_string(), "feature", true).expect("create and checkout");
        assert_eq!(
            repo.head().expect("head").shorthand().expect("name"),
            "feature"
        );
    }

    #[test]
    fn create_branch_rejects_duplicates_and_invalid_names() {
        let temp = TempRepo::create("create-branch-invalid");
        commit_two_versions(&temp);

        let duplicate = create_branch(&temp.path_string(), "main", false)
            .expect_err("duplicate rejected");
        assert!(matches!(duplicate, FileSystemError::AlreadyExists(_)));

        for invalid in ["", "-dash", "bad..name", "a b?"] {
            let error =
                create_branch(&temp.path_string(), invalid, false).expect_err("invalid name");
            assert!(
                matches!(error, FileSystemError::InvalidInput(_)),
                "{invalid:?} should be invalid"
            );
        }
    }

    #[test]
    fn checkout_local_branch_updates_worktree() {
        let temp = TempRepo::create("checkout-local");
        let old_commit = commit_two_versions(&temp);

        let repo = temp.repo();
        repo.branch(
            "old",
            &repo.find_commit(old_commit).expect("commit"),
            false,
        )
        .expect("create branch");

        checkout_branch(&temp.path_string(), "old", false).expect("checkout");
        assert_eq!(
            fs::read_to_string(temp.path.join("a.txt")).expect("read file"),
            "v1"
        );
        assert_eq!(
            repo.head().expect("head").shorthand().expect("name"),
            "old"
        );
    }

    #[test]
    fn checkout_keeps_uncommitted_changes_to_unrelated_files() {
        let temp = TempRepo::create("checkout-safe");
        let old_commit = commit_two_versions(&temp);

        let repo = temp.repo();
        repo.branch(
            "old",
            &repo.find_commit(old_commit).expect("commit"),
            false,
        )
        .expect("create branch");

        // 与分支差异无关的未提交修改应当保留（SAFE 检出）。
        fs::write(temp.path.join("notes.txt"), "scratch").expect("write file");
        checkout_branch(&temp.path_string(), "old", false).expect("checkout");
        assert_eq!(
            fs::read_to_string(temp.path.join("notes.txt")).expect("read file"),
            "scratch"
        );
    }

    #[test]
    fn checkout_refuses_to_discard_conflicting_changes() {
        let temp = TempRepo::create("checkout-conflict");
        let old_commit = commit_two_versions(&temp);

        let repo = temp.repo();
        repo.branch(
            "old",
            &repo.find_commit(old_commit).expect("commit"),
            false,
        )
        .expect("create branch");

        // a.txt 在两个分支间不同，且工作区还有未提交修改：检出必须失败。
        fs::write(temp.path.join("a.txt"), "dirty").expect("write file");
        let error = checkout_branch(&temp.path_string(), "old", false).expect_err("conflict");
        assert!(matches!(error, FileSystemError::Internal(_)));
        assert_eq!(
            fs::read_to_string(temp.path.join("a.txt")).expect("read file"),
            "dirty"
        );
    }

    #[test]
    fn checkout_remote_branch_creates_tracking_branch() {
        let temp = TempRepo::create("checkout-remote");
        let old_commit = commit_two_versions(&temp);

        let repo = temp.repo();
        repo.remote("origin", "https://example.com/repo.git")
            .expect("add remote");
        repo.reference(
            "refs/remotes/origin/feature",
            old_commit,
            true,
            "simulate fetch",
        )
        .expect("remote ref");

        checkout_branch(&temp.path_string(), "origin/feature", true).expect("checkout remote");

        let repo = temp.repo();
        let head = repo.head().expect("head");
        assert_eq!(head.shorthand().expect("name"), "feature");
        assert_eq!(
            fs::read_to_string(temp.path.join("a.txt")).expect("read file"),
            "v1"
        );
        let local = repo.find_branch("feature", BranchType::Local).expect("local");
        assert_eq!(
            local
                .upstream()
                .expect("upstream")
                .name()
                .expect("name")
                .expect("branch name"),
            "origin/feature"
        );

        // 同名本地分支已存在：再次从远程切换报错。
        let error = checkout_branch(&temp.path_string(), "origin/feature", true)
            .expect_err("duplicate local");
        assert!(matches!(error, FileSystemError::AlreadyExists(_)));
    }

    #[test]
    fn ahead_behind_counts_against_upstream() {
        let temp = TempRepo::create("ahead-behind");
        let old_commit = commit_two_versions(&temp);

        let repo = temp.repo();
        repo.remote("origin", "https://example.com/repo.git")
            .expect("add remote");
        repo.reference(
            "refs/remotes/origin/main",
            old_commit,
            true,
            "simulate fetch",
        )
        .expect("remote ref");
        // 让 main 跟踪 origin/main（branch.<name>.remote/merge 配置）。
        let mut config = repo.config().expect("config");
        config.set_str("branch.main.remote", "origin").expect("set remote");
        config
            .set_str("branch.main.merge", "refs/heads/main")
            .expect("set merge");
        drop(config);

        let branches = branches(&temp.path_string())
            .expect("branches call")
            .expect("inside repo");
        assert!(branches.has_upstream);
        assert_eq!(branches.ahead, 1);
        assert_eq!(branches.behind, 0);
    }

    #[test]
    fn branches_returns_none_outside_worktree() {
        let dir =
            std::env::temp_dir().join(format!("dae-git-test-{}-branches-outside", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("create dir");

        let result = branches(&path_to_string(&dir)).expect("branches call");
        let _ = fs::remove_dir_all(&dir);
        assert!(result.is_none());
    }
}
