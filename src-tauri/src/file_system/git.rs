use super::error::FileSystemError;
use super::types::path_to_string;
use git2::build::CheckoutBuilder;
use git2::{BranchType, ErrorCode, Oid, Reference, Repository, Status, StatusOptions};
use serde::{Deserialize, Serialize};
use specta::Type;
use std::collections::HashMap;
use std::fs;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};
use tauri::Manager;

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
/// 执行，不会阻塞异步运行时或 UI；扫描结果按工作区根缓存（见
/// [`GitStatusCache`]），所以同一个工作区里的相邻目录只付一次扫描。
#[tauri::command]
#[specta::specta]
pub async fn get_git_status(
    path: String,
    app: tauri::AppHandle,
) -> Result<Option<GitDirectoryStatus>, FileSystemError> {
    tauri::async_runtime::spawn_blocking(move || {
        let cache = app.state::<GitStatusCache>();
        git_status_cached(&path, &cache)
    })
    .await
    .map_err(|error| FileSystemError::Internal(error.to_string()))?
}

/// How long one worktree-wide scan may keep answering directory reads.
///
/// The same five seconds the frontend already declares as its own staleness
/// budget for these badges (`GIT_STALE_TIME_MS` in
/// `src/features/explorer/git-status.tsx`): within it, navigating around one
/// work tree answers from a single scan; past it a directory read scans again
/// rather than showing badges older than the window the UI already tolerates.
const SCAN_MAX_AGE: Duration = Duration::from_secs(5);

/// Worktrees whose scans are kept at once. Only reached by reading several
/// repositories inside one reuse window — the workspace overview probes the
/// recent directories — and the map is emptied by every reported change anyway.
const MAX_CACHED_WORKTREES: usize = 8;

/// One classified record of a worktree-wide scan: a path relative to the
/// worktree root, `/`-separated as git2 spells it, and the badge it classifies
/// to. Deletions and ignored paths are not records — they have no badge.
type StatusRecords = Arc<[(String, GitEntryStatusKind)]>;

/// The worktree-wide status scan behind every per-directory badge, kept per
/// worktree root.
///
/// libgit2 answers a status query by looking at every tracked file, while a
/// directory listing only ever consumes the badges of its own direct children.
/// A work tree of 20,000 tracked files therefore spends ~36 ms of a blocking
/// thread and of the disk on a directory whose answer is two badges, and a
/// monorepo pays it again for every step of a navigation
/// (`docs/performance-opportunities.md` §2). One scan can answer every directory
/// of the worktree, so it is computed once and filtered per directory instead.
#[derive(Default)]
pub struct GitStatusCache {
    /// Counts the filesystem changes the directory watchers reported. Their
    /// events are what refresh the badges mid-session, and they are also what
    /// invalidates this cache: a change a watcher saw cannot be answered from a
    /// scan taken before it.
    changes: AtomicU64,
    /// One slot per worktree root, holding that root's most recent scan. A
    /// slot's own lock is held across a scan, so two views in one worktree pay
    /// for one.
    worktrees: Mutex<HashMap<String, Arc<Mutex<Option<CachedScan>>>>>,
}

/// A scan, plus the keys that would make it wrong: a change a watcher reported,
/// an index rewritten behind our back (staging, checkout, a commit made in the
/// integrated terminal), or HEAD moved onto another tree.
struct CachedScan {
    epoch: u64,
    index: Option<(SystemTime, u64)>,
    head: Option<Oid>,
    scanned_at: Instant,
    records: StatusRecords,
}

impl GitStatusCache {
    /// Records that a watcher saw the filesystem change. Called from the local
    /// directory watcher, beside the event that refreshes the listing.
    pub fn note_change(&self) {
        self.changes.fetch_add(1, AtomicOrdering::AcqRel);
        // A change anywhere can affect any worktree's badges, and a scan is
        // large: keeping them would only postpone the rescan.
        self.worktrees
            .lock()
            .expect("git status cache lock poisoned")
            .clear();
    }

    /// The worktree-wide records for `repo`, reusing the last scan when nothing
    /// has invalidated it. `max_age` is a parameter so a test can expire a scan
    /// without waiting out the window the app uses.
    fn records(
        &self,
        repo: &Repository,
        root: &str,
        max_age: Duration,
    ) -> Result<StatusRecords, FileSystemError> {
        let epoch = self.changes.load(AtomicOrdering::Acquire);
        let index = index_key(repo);
        let head = repo.head().ok().and_then(|head| head.target());

        let slot = {
            let mut worktrees = self
                .worktrees
                .lock()
                .expect("git status cache lock poisoned");
            if worktrees.len() >= MAX_CACHED_WORKTREES {
                worktrees.clear();
            }
            Arc::clone(worktrees.entry(root.to_owned()).or_default())
        };

        // Held across the scan: a second view of this worktree waits here and
        // then finds the scan rather than paying for its own.
        let mut slot = slot.lock().expect("git status scan lock poisoned");
        if let Some(cached) = slot.as_ref()
            && cached.epoch == epoch
            && cached.index == index
            && cached.head == head
            && cached.scanned_at.elapsed() < max_age
        {
            return Ok(Arc::clone(&cached.records));
        }

        // The keys were read before the scan, so a change landing while it runs
        // leaves an entry the next request refuses — never one that passes for
        // newer than it is.
        let records: StatusRecords = scan_worktree_status(repo)?.into();
        *slot = Some(CachedScan {
            epoch,
            index,
            head,
            scanned_at: Instant::now(),
            records: Arc::clone(&records),
        });
        Ok(records)
    }
}

/// The index as (modified, length). Rewritten by anything that stages, checks
/// out or commits — including the same operations run in the integrated
/// terminal, which no watcher reports.
fn index_key(repo: &Repository) -> Option<(SystemTime, u64)> {
    let metadata = fs::metadata(repo.path().join("index")).ok()?;
    Some((metadata.modified().ok()?, metadata.len()))
}

/// 读取一个目录的 Git 装饰信息，扫描结果按工作区根从 `cache` 复用。
fn git_status_cached(
    dir: &str,
    cache: &GitStatusCache,
) -> Result<Option<GitDirectoryStatus>, FileSystemError> {
    let Some(repo) = discover_worktree(dir)? else {
        return Ok(None);
    };
    // 裸仓库没有工作区可供装饰。
    let workdir = path_to_string(repo.workdir().expect("workdir verified above"));

    // 目录不在工作区内时直接返回。原先这一步在 `statuses` 之后，等于先付一次
    // 全工作区扫描再把结果丢掉；软链接与 junction 都可能让路径落到工作区外。
    let Some(relative_segments) = relative_segments(&workdir, dir) else {
        return Ok(None);
    };

    let branch = branch_name(&repo);
    let records = cache.records(&repo, &workdir, SCAN_MAX_AGE)?;

    Ok(Some(derive_directory_status(
        &workdir,
        &relative_segments,
        branch,
        &records,
    )))
}

/// The reference read, with no cache in the way: discover, scan the whole
/// worktree, filter. `get_git_status` goes through [`GitStatusCache`] instead;
/// this stays as the behavioural reference the cache's tests compare against,
/// and as the scan the measurement harness prices.
#[cfg(test)]
fn git_status(dir: &str) -> Result<Option<GitDirectoryStatus>, FileSystemError> {
    let Some(repo) = discover_worktree(dir)? else {
        return Ok(None);
    };
    let workdir = path_to_string(repo.workdir().expect("workdir verified above"));
    let Some(relative_segments) = relative_segments(&workdir, dir) else {
        return Ok(None);
    };

    let branch = branch_name(&repo);
    let records = scan_worktree_status(&repo)?;

    Ok(Some(derive_directory_status(
        &workdir,
        &relative_segments,
        branch,
        &records,
    )))
}

/// The scan that costs O(worktree): every status record libgit2 reports,
/// classified. The per-directory filter afterwards costs O(records).
fn scan_worktree_status(
    repo: &Repository,
) -> Result<Vec<(String, GitEntryStatusKind)>, FileSystemError> {
    let mut options = StatusOptions::new();
    options
        .include_untracked(true)
        .recurse_untracked_dirs(false)
        .include_ignored(false);

    // 注意：这里**不能**用 `options.pathspec(..)` 把扫描限制到某个目录。
    // 试过，被现有的 `untracked_subdirectory_aggregates_on_parent_and_resolves_inside`
    // 当场拦下：pathspec 会让 libgit2 从该目录内部起步，未跟踪目录不再被折叠成
    // 一条记录而是展开成子项，`directory_untracked` 因此变 false，前端「整个目录
    // 未跟踪则所有子项标 U」的契约就破了。要拿这个收益得先决定那个契约怎么改。
    // （另：真要用 pathspec，字面路径要用 `disable_pathspec_match(true)` 选项，见
    // git2 `StatusOptions::pathspec` 的文档，而不是 `:(literal)` 字符串前缀。）
    // 现在这条路由缓存替代：整棵树只扫一次，`derive_directory_status` 从这一份
    // 结果里按目录过滤，得到的答案与逐目录扫描逐字相同。
    let statuses = repo
        .statuses(Some(&mut options))
        .map_err(|error| FileSystemError::Internal(error.to_string()))?;

    Ok(statuses
        .iter()
        .filter_map(|entry| {
            let path = entry.path().ok()?;
            let kind = classify(entry.status())?;
            Some((path.to_owned(), kind))
        })
        .collect())
}

/// Filters one worktree-wide scan down to `relative_segments`: the direct
/// children of that directory, each aggregating its descendants (M > A > U),
/// and whether the directory itself is one git folded into a single untracked
/// record.
fn derive_directory_status(
    workdir: &str,
    relative_segments: &[String],
    branch: String,
    records: &[(String, GitEntryStatusKind)],
) -> GitDirectoryStatus {
    let depth = relative_segments.len();

    // 聚合当前目录直接子项的状态：文件取自身状态，目录取后代状态中
    // 优先级最高者（M > A > U）。
    let mut directory_untracked = false;
    let mut badges: HashMap<String, GitEntryStatusKind> = HashMap::new();
    for (status_path, kind) in records {
        let kind = *kind;

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
                .zip(relative_segments)
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
            .zip(relative_segments)
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

    GitDirectoryStatus {
        root: workdir.to_owned(),
        branch,
        entries,
        directory_untracked,
    }
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
        return Err(FileSystemError::InvalidInput(
            "git.invalid_branch_name".into(),
        ));
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
        repo.set_head(reference_name).map_err(|error| {
            FileSystemError::Internal(format!("git.create_branch_failed: {error}"))
        })?;
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
    use std::path::{Path, PathBuf};

    struct TempRepo {
        path: PathBuf,
    }

    impl TempRepo {
        fn create(name: &str) -> Self {
            let path =
                std::env::temp_dir().join(format!("dae-git-test-{}-{name}", std::process::id()));
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

        repo.commit(
            Some("HEAD"),
            &signature,
            &signature,
            message,
            &tree,
            &parents,
        )
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
        index
            .add_path(std::path::Path::new("a.txt"))
            .expect("stage");
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

    /// The cache's contract: one scan answers every directory of a worktree.
    #[test]
    fn the_cache_answers_a_second_directory_from_the_scan_it_already_has() {
        let temp = TempRepo::create("cache-reuse");
        let sub = temp.path.join("sub");
        fs::create_dir_all(&sub).expect("create sub");
        fs::write(sub.join("b.txt"), "hello").expect("write file");

        let cache = GitStatusCache::default();
        let root_status = git_status_cached(&temp.path_string(), &cache)
            .expect("status call")
            .expect("inside repo");
        assert_eq!(root_status.entries.len(), 1);
        assert_eq!(root_status.entries[0].name, "sub");

        // The directory filter works from the worktree's spelling, so both
        // discoveries have to agree on it — otherwise every directory would key
        // its own slot and no scan would ever be shared.
        let from_root = discover_worktree(&temp.path_string())
            .expect("discover")
            .expect("worktree");
        let from_sub = discover_worktree(&path_to_string(&sub))
            .expect("discover")
            .expect("worktree");
        let workdir = path_to_string(from_root.workdir().expect("workdir"));
        assert_eq!(
            workdir,
            path_to_string(from_sub.workdir().expect("workdir"))
        );

        // The same records come back, so the second directory was answered by
        // the first directory's scan rather than by a new one.
        let first = cache
            .records(&from_root, &workdir, SCAN_MAX_AGE)
            .expect("records");
        let second = cache
            .records(&from_sub, &workdir, SCAN_MAX_AGE)
            .expect("records");
        assert!(Arc::ptr_eq(&first, &second));

        // And the badges it derives are the ones the uncached read derives.
        let cached = git_status_cached(&path_to_string(&sub), &cache)
            .expect("status call")
            .expect("inside repo");
        let reference = git_status(&path_to_string(&sub))
            .expect("status call")
            .expect("inside repo");
        assert_eq!(cached.branch, reference.branch);
        assert_eq!(cached.root, reference.root);
        assert_eq!(cached.directory_untracked, reference.directory_untracked);
        assert_eq!(cached.entries.len(), reference.entries.len());
    }

    /// A change the watcher reported has to invalidate the scan, because a new
    /// file leaves nothing in the index for the other keys to notice.
    #[test]
    fn a_reported_change_invalidates_the_scan() {
        let temp = TempRepo::create("cache-change");
        let cache = GitStatusCache::default();
        let dir = temp.path_string();

        let before = git_status_cached(&dir, &cache)
            .expect("status call")
            .expect("inside repo");
        assert!(before.entries.is_empty());

        fs::write(temp.path.join("new.txt"), "new").expect("write file");
        let unreported = git_status_cached(&dir, &cache)
            .expect("status call")
            .expect("inside repo");
        assert!(
            unreported.entries.is_empty(),
            "only a reported change can catch an untracked file"
        );

        cache.note_change();
        let reported = git_status_cached(&dir, &cache)
            .expect("status call")
            .expect("inside repo");
        assert_eq!(reported.entries.len(), 1);
        assert_eq!(reported.entries[0].name, "new.txt");
        assert_eq!(reported.entries[0].kind, GitEntryStatusKind::Untracked);
    }

    /// Operations git performs outside a watcher's view — staging, committing,
    /// checking out, including in the integrated terminal — are caught by the
    /// index and HEAD keys instead.
    #[test]
    fn git_operations_no_watcher_reported_invalidate_the_scan() {
        let temp = TempRepo::create("cache-index");
        let cache = GitStatusCache::default();
        let dir = temp.path_string();
        fs::write(temp.path.join("a.txt"), "hello").expect("write file");

        let untracked = git_status_cached(&dir, &cache)
            .expect("status call")
            .expect("inside repo");
        assert_eq!(untracked.entries[0].kind, GitEntryStatusKind::Untracked);

        let repo = temp.repo();
        let mut index = repo.index().expect("index");
        index
            .add_path(std::path::Path::new("a.txt"))
            .expect("stage");
        index.write().expect("write index");

        let staged = git_status_cached(&dir, &cache)
            .expect("status call")
            .expect("inside repo");
        assert_eq!(staged.entries[0].kind, GitEntryStatusKind::Added);

        // A commit moves HEAD onto another tree, and the badge goes away.
        commit_all(&temp.repo(), "initial");
        let committed = git_status_cached(&dir, &cache)
            .expect("status call")
            .expect("inside repo");
        assert!(committed.entries.is_empty());
    }

    /// A scan past the reuse window is replaced rather than reused, so a
    /// directory read never shows badges older than the window.
    #[test]
    fn a_scan_older_than_the_window_is_not_reused() {
        let temp = TempRepo::create("cache-expiry");
        let cache = GitStatusCache::default();
        let repo = temp.repo();
        let workdir = path_to_string(repo.workdir().expect("workdir"));

        // A zero window stands in for an elapsed one: the window the app allows
        // is five seconds, which a test cannot wait out.
        let first = cache
            .records(&repo, &workdir, Duration::ZERO)
            .expect("records");
        let second = cache
            .records(&repo, &workdir, Duration::ZERO)
            .expect("records");
        assert!(!Arc::ptr_eq(&first, &second), "an expired scan is replaced");

        let third = cache
            .records(&repo, &workdir, SCAN_MAX_AGE)
            .expect("records");
        let fourth = cache
            .records(&repo, &workdir, SCAN_MAX_AGE)
            .expect("records");
        assert!(Arc::ptr_eq(&third, &fourth), "a fresh scan is reused");
    }

    /// Two readers of one worktree pay for one scan: the worktree's lock is held
    /// across the scan, so the second reader waits and then finds it — which is
    /// the shape of the workspace overview probing several recent directories.
    #[test]
    fn two_readers_of_one_worktree_share_a_scan() {
        let temp = TempRepo::create("cache-concurrent");
        fs::write(temp.path.join("a.txt"), "hello").expect("write file");

        let cache = GitStatusCache::default();
        let dir = temp.path_string();
        let records: Vec<StatusRecords> = std::thread::scope(|scope| {
            let readers: Vec<_> = (0..4)
                .map(|_| {
                    scope.spawn(|| {
                        let repo = discover_worktree(&dir)
                            .expect("discover")
                            .expect("worktree");
                        let workdir = path_to_string(repo.workdir().expect("workdir"));
                        cache
                            .records(&repo, &workdir, SCAN_MAX_AGE)
                            .expect("records")
                    })
                })
                .collect();
            readers
                .into_iter()
                .map(|reader| reader.join().expect("reader"))
                .collect()
        });

        // Every reader received the same records object, so one scan served
        // them rather than each producing its own.
        for other in &records[1..] {
            assert!(Arc::ptr_eq(&records[0], other));
        }
    }

    /// The contract that rejected the pathspec road
    /// (`docs/performance-opportunities.md` §2): a directory git folded into one
    /// untracked record has to keep resolving to `directory_untracked` when it
    /// is the directory being listed — including when the scan answering it was
    /// taken for another directory.
    #[test]
    fn the_cached_scan_keeps_the_untracked_directory_contract() {
        let temp = TempRepo::create("cache-untracked-directory");
        let sub = temp.path.join("sub");
        fs::create_dir_all(&sub).expect("create sub");
        fs::write(sub.join("b.txt"), "hello").expect("write file");

        let cache = GitStatusCache::default();
        let root_status = git_status_cached(&temp.path_string(), &cache)
            .expect("status call")
            .expect("inside repo");
        assert!(!root_status.directory_untracked);
        assert_eq!(root_status.entries.len(), 1);
        assert_eq!(root_status.entries[0].name, "sub");
        assert_eq!(root_status.entries[0].kind, GitEntryStatusKind::Untracked);

        let sub_status = git_status_cached(&path_to_string(&sub), &cache)
            .expect("status call")
            .expect("inside repo");
        assert!(sub_status.directory_untracked);
        assert!(sub_status.entries.is_empty());
    }

    /// 提交两个版本：第一个提交（旧）与 HEAD（新）都修改 a.txt。
    /// 返回旧提交号，供模拟远程分支引用使用。
    fn commit_two_versions(temp: &TempRepo) -> git2::Oid {
        fs::write(temp.path.join("a.txt"), "v1").expect("write file");
        commit_all(&temp.repo(), "first");
        let old_commit = temp.repo().head().expect("head").target().expect("target");

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
        repo.branch(
            "dev",
            &repo.find_commit(head_commit).expect("commit"),
            false,
        )
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
        assert_eq!(
            repo.head().expect("head").shorthand().expect("name"),
            "main"
        );

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

        let duplicate =
            create_branch(&temp.path_string(), "main", false).expect_err("duplicate rejected");
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
        repo.branch("old", &repo.find_commit(old_commit).expect("commit"), false)
            .expect("create branch");

        checkout_branch(&temp.path_string(), "old", false).expect("checkout");
        assert_eq!(
            fs::read_to_string(temp.path.join("a.txt")).expect("read file"),
            "v1"
        );
        assert_eq!(repo.head().expect("head").shorthand().expect("name"), "old");
    }

    #[test]
    fn checkout_keeps_uncommitted_changes_to_unrelated_files() {
        let temp = TempRepo::create("checkout-safe");
        let old_commit = commit_two_versions(&temp);

        let repo = temp.repo();
        repo.branch("old", &repo.find_commit(old_commit).expect("commit"), false)
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
        repo.branch("old", &repo.find_commit(old_commit).expect("commit"), false)
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
        let local = repo
            .find_branch("feature", BranchType::Local)
            .expect("local");
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
        config
            .set_str("branch.main.remote", "origin")
            .expect("set remote");
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

    /// The median of `samples`, in milliseconds, after sorting them.
    fn median_ms(samples: &mut [u128]) -> f64 {
        samples.sort_unstable();
        samples[samples.len() / 2] as f64 / 1_000_000.0
    }

    /// Times `body` `runs` times and returns the samples in nanoseconds.
    fn time_runs(runs: usize, mut body: impl FnMut()) -> Vec<u128> {
        let mut samples = Vec::with_capacity(runs);
        for _ in 0..runs {
            let started = std::time::Instant::now();
            body();
            samples.push(started.elapsed().as_nanos());
        }
        samples
    }

    /// The directories a navigation would visit: the worktree root, then up to
    /// four directories drawn from the index, shallowest first so the list
    /// reads like a descent rather than a hash order.
    fn sample_directories(repo: &Repository, root: &Path) -> Vec<PathBuf> {
        use std::collections::BTreeSet;

        let mut seen: BTreeSet<String> = BTreeSet::new();
        if let Ok(index) = repo.index() {
            for entry in index.iter() {
                let path = String::from_utf8_lossy(&entry.path).replace('\\', "/");
                let mut parts: Vec<&str> =
                    path.split('/').filter(|part| !part.is_empty()).collect();
                parts.pop(); // the file itself
                for depth in 1..=parts.len().min(3) {
                    seen.insert(parts[..depth].join("/"));
                }
            }
        }

        let mut relative: Vec<String> = seen.into_iter().collect();
        relative.sort_by(|left, right| {
            left.matches('/')
                .count()
                .cmp(&right.matches('/').count())
                .then_with(|| left.cmp(right))
        });
        relative.truncate(4);

        let mut directories = vec![root.to_path_buf()];
        directories.extend(relative.iter().map(|relative| root.join(relative)));
        directories.retain(|directory| directory.is_dir());
        directories
    }

    /// One raw scan at `dir`: how many status records it produced, and whether
    /// it folded `dir` itself into a single untracked record. `pathspec`
    /// restricts the scan to a literal path, which is the alternative §2 of
    /// `docs/performance-opportunities.md` declined.
    fn scan_records(dir: &Path, pathspec: Option<&Path>) -> (usize, bool) {
        let repo = Repository::discover(dir).expect("discover");
        let mut options = StatusOptions::new();
        options
            .include_untracked(true)
            .recurse_untracked_dirs(false)
            .include_ignored(false);
        if let Some(pathspec) = pathspec {
            options.disable_pathspec_match(true).pathspec(pathspec);
        }

        let statuses = repo.statuses(Some(&mut options)).expect("statuses");
        let records = statuses.len();

        let workdir = path_to_string(repo.workdir().expect("workdir"));
        let expected =
            relative_segments(&workdir, &path_to_string(dir)).expect("probe is in the worktree");
        let depth = expected.len();
        let self_is_untracked = depth > 0
            && statuses.iter().any(|entry| {
                entry.status().contains(Status::WT_NEW)
                    && entry.path().is_ok_and(|path| {
                        let parts: Vec<&str> =
                            path.split('/').filter(|part| !part.is_empty()).collect();
                        parts.len() == depth
                            && parts
                                .iter()
                                .zip(&expected)
                                .all(|(part, expected)| part.eq_ignore_ascii_case(expected))
                    })
            });

        (records, self_is_untracked)
    }

    /// Path of the synthetic worktree, built on the first run and reused
    /// afterwards: creating 20,000 files costs ~40 s, which is not something to
    /// pay on every run of a measurement. It lives in the temp directory under a
    /// name that records its size, and is left there to be reused (or deleted)
    /// by hand.
    fn synthetic_worktree(files: usize) -> PathBuf {
        const DIRECTORIES: usize = 200;

        let path = std::env::temp_dir().join(format!("dae-git-bench-{files}"));
        let per_directory = files.div_ceil(DIRECTORIES);
        let marker = path
            .join("d003")
            .join("nested")
            .join(format!("n{:04}.txt", 9));
        if marker.is_file() {
            println!("reused the worktree at {}", path.display());
            return path;
        }

        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).expect("create temp dir");
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

        for directory in 0..DIRECTORIES {
            let directory_path = path.join(format!("d{directory:03}"));
            fs::create_dir_all(&directory_path).expect("create directory");
            for index in 0..per_directory {
                fs::write(directory_path.join(format!("f{index:04}.txt")), "x")
                    .expect("write file");
            }
        }
        commit_all(&repo, "bench");

        let nested = path.join("d003").join("nested");
        fs::create_dir_all(&nested).expect("create untracked directory");
        for index in 0..10 {
            fs::write(path.join("d000").join(format!("f{index:04}.txt")), "edited")
                .expect("edit tracked file");
            fs::write(path.join("d001").join(format!("new{index:04}.txt")), "new")
                .expect("write untracked file");
            fs::write(nested.join(format!("n{index:04}.txt")), "new").expect("write file");
        }

        path
    }

    /// Prints what one directory's Git decoration costs: `scan` is the
    /// whole-worktree status behind it (the shape `get_git_status` had before
    /// the per-worktree cache), and `cached` is the same directory read through
    /// [`GitStatusCache`] in navigation order — the first read of a worktree
    /// pays the scan, every later one only the filter.
    ///
    /// ```bash
    /// cd src-tauri
    /// # A real repository: the worktree root plus four directories from it.
    /// DAE_GIT_BENCH=K:/code/dae \
    ///   cargo test --lib --release -- --ignored --nocapture reports_the_cost_of_a_git_status_scan
    /// # Or a synthetic worktree of `DAE_GIT_BENCH_FILES` tracked files
    /// # (20,000 by default), built once in the temp directory and reused.
    /// DAE_GIT_BENCH_FILES=20000 \
    ///   cargo test --lib --release -- --ignored --nocapture reports_the_cost_of_a_git_status_scan
    /// ```
    ///
    /// `tracked` is how many files a scan has to look at and `records` is how
    /// many of them have something to report: the cost follows the first number
    /// while the answer only uses the second. `porcelain` is
    /// `git status --porcelain` in a child process — the magnitude's floor,
    /// with git's untracked cache and index short-circuits available to it and
    /// not to libgit2. `pathspec` is the same scan limited to one directory, the
    /// road §2 declined: kept measured so the price of the contract standing in
    /// its way stays visible. Compare this test against itself, before and after
    /// a change; absolute milliseconds are per-machine.
    #[test]
    #[ignore = "prints measurements for DAE_GIT_BENCH (a repository) or DAE_GIT_BENCH_FILES"]
    fn reports_the_cost_of_a_git_status_scan() {
        let explicit = std::env::var_os("DAE_GIT_BENCH");
        let root = match &explicit {
            Some(dir) => PathBuf::from(dir),
            None => {
                let files = std::env::var("DAE_GIT_BENCH_FILES")
                    .ok()
                    .and_then(|value| value.parse().ok())
                    .unwrap_or(20_000);
                let started = std::time::Instant::now();
                let path = synthetic_worktree(files);
                println!(
                    "synthetic {files} tracked files, ready in {:.1} s",
                    started.elapsed().as_secs_f64()
                );
                path
            }
        };

        let repo = Repository::discover(&root).expect("discover a worktree");
        let workdir = path_to_string(repo.workdir().expect("workdir"));
        let tracked = repo.index().expect("index").len();
        let (records, _) = scan_records(&root, None);

        println!("worktree  {}", workdir.trim_end_matches(['/', '\\']));
        println!("tracked   {tracked}");
        println!("records   {records}");

        // `get_git_status` per directory: what one navigation pays today.
        let directories = sample_directories(&repo, repo.workdir().expect("workdir"));
        for directory in &directories {
            let dir = path_to_string(directory);
            let entries = git_status(&dir)
                .expect("status")
                .map_or(0, |status| status.entries.len());
            let mut samples = time_runs(5, || {
                git_status(&dir).expect("status");
            });
            let relative = dir
                .trim_start_matches(&workdir)
                .trim_start_matches(['/', '\\'])
                .replace('\\', "/");
            println!(
                "scan      {:>8.2} ms  entries={entries}  {}",
                median_ms(&mut samples),
                if relative.is_empty() {
                    "(root)"
                } else {
                    &relative
                }
            );
        }

        // The same five directories through the cache `get_git_status` uses, in
        // the order a navigation takes them: the first read of a worktree pays
        // for the scan, every later one only for the filter. One run for the
        // scan (the scan is the whole sample) and five for the hits.
        let cache = GitStatusCache::default();
        for (position, directory) in directories.iter().enumerate() {
            let dir = path_to_string(directory);
            let mut samples = time_runs(if position == 0 { 1 } else { 5 }, || {
                git_status_cached(&dir, &cache).expect("status");
            });
            let entries = git_status_cached(&dir, &cache)
                .expect("status")
                .map_or(0, |status| status.entries.len());
            let relative = dir
                .trim_start_matches(&workdir)
                .trim_start_matches(['/', '\\'])
                .replace('\\', "/");
            println!(
                "cached    {:>8.2} ms  entries={entries}  {}{}",
                median_ms(&mut samples),
                if relative.is_empty() {
                    "(root)"
                } else {
                    &relative
                },
                if position == 0 {
                    "  (the scan)"
                } else {
                    "  (reused)"
                }
            );
        }

        // What the reused reads above still pay for: discovering and opening the
        // repository, the branch, and the keys compared against the cache. The
        // scan is the only part the cache removes.
        let mut samples = time_runs(5, || {
            let repo = discover_worktree(&path_to_string(&directories[0]))
                .expect("discover")
                .expect("worktree");
            branch_name(&repo);
            index_key(&repo);
            repo.head().ok().and_then(|head| head.target());
        });
        println!(
            "open      {:>8.2} ms  (worktree + branch + keys, no scan)",
            median_ms(&mut samples)
        );

        // The magnitude's floor: git's own status, in a child process.
        let mut samples = time_runs(5, || {
            Command::new("git")
                .arg("-C")
                .arg(workdir.trim_end_matches(['/', '\\']))
                .args(["status", "--porcelain"])
                .output()
                .expect("git status --porcelain");
        });
        println!(
            "porcelain {:>8.2} ms  (git status --porcelain, the floor)",
            median_ms(&mut samples)
        );

        // The declined alternative, measured: a pathspec-limited scan, and what
        // it does to the answer at a directory whose own untracked records are
        // the ones a listing depends on.
        let mut probes: Vec<PathBuf> = directories.iter().skip(1).cloned().collect();
        if let Some(nested) = directories.last().map(|dir| dir.join("nested"))
            && nested.is_dir()
        {
            probes.push(nested);
        }

        for probe in &probes {
            let relative = path_to_string(probe)
                .trim_start_matches(&workdir)
                .trim_start_matches(['/', '\\'])
                .replace('\\', "/");
            let mut limited = time_runs(5, || {
                scan_records(probe, Some(Path::new(&relative)));
            });
            let mut full = time_runs(5, || {
                scan_records(probe, None);
            });
            let (full_records, full_untracked) = scan_records(probe, None);
            let (limited_records, limited_untracked) =
                scan_records(probe, Some(Path::new(&relative)));
            println!(
                "pathspec  {:>8.2} ms  vs {:>8.2} ms full  records {limited_records} vs {full_records}  directory_untracked {limited_untracked} vs {full_untracked}  {relative}",
                median_ms(&mut limited),
                median_ms(&mut full),
            );
        }
    }

    #[test]
    fn branches_returns_none_outside_worktree() {
        let dir = std::env::temp_dir().join(format!(
            "dae-git-test-{}-branches-outside",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("create dir");

        let result = branches(&path_to_string(&dir)).expect("branches call");
        let _ = fs::remove_dir_all(&dir);
        assert!(result.is_none());
    }
}
