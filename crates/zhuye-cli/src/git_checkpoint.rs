//! `/checkpoint` 的 git 落地:快照提交 / 列表 / 回退。
//!
//! 此前 `/checkpoint` 的每个按钮都是占位通知 —— Create 打印
//! "git auto-commit not yet wired",Rewind 打印 "git reset --hard not yet wired
//! (sha saved)",Refresh 打印 "(placeholder)"。一个每个按钮都说不支持的 UI
//! 比没有这个 UI 更糟:用户会以为自己有检查点,实际上什么都没有。
//!
//! ## 安全约束
//!
//! `git reset --hard` 是破坏性操作,所以:
//!
//! 1. **回退前先做一次快照提交** —— 用户反悔了还能再回退到回退前的状态,
//!    不会把未提交的工作一次性烧掉。
//! 2. **只接受本模块自己创建的 checkpoint 提交** —— 校验 subject 前缀,
//!    避免畸形 sha 或用户手挑的任意提交被 `reset --hard` 打进去。
//! 3. **非 git 仓库 / git 不在 PATH 都明确报错**,不静默成功。

use std::path::Path;
use std::process::Command;

/// checkpoint 提交的 subject 前缀。`list` / `rewind` 都靠它识别自己的提交。
pub const CHECKPOINT_SUBJECT_PREFIX: &str = "reflect-checkpoint:";

/// checkpoint 的 git tag 前缀。
///
/// ## 为什么必须打 tag
///
/// 只靠 subject 前缀 + `git log` 列 checkpoint 是不够的:`rewind` 会先做一次
/// 安全快照提交,然后 `reset --hard` 回旧提交 —— 这一步之后,安全快照提交
/// **不在当前分支的历史上**了,`git log` 从 HEAD 出发看不到它,于是用户
/// 想「撤销这次回退」时那个 sha 已经列不出来、也 rewind 不过去。
///
/// tag 是一个独立存活于 `refs/tags/` 的引用,`reset --hard` 不会动它,
/// 所以 checkpoint 在回退之后依然可达、可列、可再回退。
pub const CHECKPOINT_TAG_PREFIX: &str = "reflect-cp/";

/// 一个 checkpoint 条目。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Checkpoint {
    pub sha: String,
    /// 提交 subject(去掉前缀后的 label 部分)。
    pub subject: String,
    /// 提交时间(ISO 8601)。
    pub committed_at: String,
}

/// git 操作失败的原因。给用户可操作的信息,不是裸的 stderr。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GitError {
    /// `git` 可执行文件起不来(不在 PATH / 无执行权限)。
    GitUnavailable(String),
    /// 当前目录不在 git 仓库里。
    NotARepo,
    /// 目标 sha 不是本模块创建的 checkpoint 提交。
    NotACheckpoint(String),
    /// 其它 git 失败(带 stderr)。
    Failed(String),
}

impl std::fmt::Display for GitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GitError::GitUnavailable(e) => write!(f, "git 不可用: {e}"),
            GitError::NotARepo => write!(f, "当前目录不是 git 仓库"),
            GitError::NotACheckpoint(sha) => {
                write!(f, "{sha} 不是 reflect 创建的 checkpoint")
            }
            GitError::Failed(e) => write!(f, "git 失败: {e}"),
        }
    }
}

fn run_git(cwd: &Path, args: &[&str]) -> Result<String, GitError> {
    let out = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .map_err(|e| GitError::GitUnavailable(e.to_string()))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).to_string())
    } else {
        let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
        // `rev-parse --show-toplevel` 在非仓库里退出码非 0,归到 NotARepo。
        if args.first() == Some(&"rev-parse") && stderr.contains("not a git repository") {
            return Err(GitError::NotARepo);
        }
        Err(GitError::Failed(stderr))
    }
}

/// 是否是 git 仓库。
pub fn is_repo(cwd: &Path) -> bool {
    run_git(cwd, &["rev-parse", "--show-toplevel"]).is_ok()
}

/// 创建 checkpoint:`git add -A` + `git commit`。
///
/// 没有改动时不创建空提交,返回 `Ok(None)` —— 调用方据此提示「无改动」。
/// `label` 为空时用默认文案。
pub fn create(cwd: &Path, label: &str) -> Result<Option<Checkpoint>, GitError> {
    if !is_repo(cwd) {
        return Err(GitError::NotARepo);
    }
    // 暂存全部改动(含新文件)。不传路径,让 git 自己按 .gitignore 过滤。
    run_git(cwd, &["add", "-A"])?;
    // 有暂存内容才提交,避免 `nothing to commit` 的非零退出。
    let staged = run_git(cwd, &["diff", "--cached", "--name-only"])?;
    if staged.trim().is_empty() {
        return Ok(None);
    }
    let label = label.trim();
    let subject = if label.is_empty() {
        CHECKPOINT_SUBJECT_PREFIX.to_string()
    } else {
        format!("{CHECKPOINT_SUBJECT_PREFIX} {label}")
    };
    // `--no-verify` 跳过 pre-commit hook:checkpoint 是安全网,不该被项目
    // 自己的 lint/format hook 挡住而静默失败。
    run_git(
        cwd,
        &["commit", "--no-verify", "-m", &subject, "--allow-empty"],
    )?;
    let sha = run_git(cwd, &["rev-parse", "HEAD"])?.trim().to_string();
    // 打 tag 让这个 checkpoint 在后续 reset --hard 之后依然可达、可列、可再回退。
    // `-f` 允许同名短 sha 标签被覆盖(极端情况下短 sha 碰撞)。
    let short = &sha[..8.min(sha.len())];
    let tag = format!("{CHECKPOINT_TAG_PREFIX}{short}");
    let _ = run_git(cwd, &["tag", "-f", &tag, &sha]);
    let meta = run_git(cwd, &["log", "-1", "--format=%cI%x1f%s"])?;
    let (committed_at, subject) = match meta.trim().split_once('\u{1f}') {
        Some((t, s)) => (t.to_string(), s.to_string()),
        None => (String::new(), subject.clone()),
    };
    Ok(Some(Checkpoint {
        sha,
        subject: subject
            .strip_prefix(CHECKPOINT_SUBJECT_PREFIX)
            .unwrap_or(&subject)
            .trim()
            .to_string(),
        committed_at,
    }))
}

/// 列出最近的 checkpoint 提交(最新在前)。
///
/// 走 tag 而非 `git log`:回退之后的安全快照已不在分支历史上,只有 tag
/// 还指着它(见 [`CHECKPOINT_TAG_PREFIX`] 的说明)。
pub fn list(cwd: &Path, limit: usize) -> Result<Vec<Checkpoint>, GitError> {
    if !is_repo(cwd) {
        return Err(GitError::NotARepo);
    }
    // 一次 for-each-ref 拿全字段:%1f 作字段分隔符。
    let out = run_git(
        cwd,
        &[
            "for-each-ref",
            // 必须带 refs/tags/ 前缀:for-each-ref 的 pattern 按完整 refname
            // 匹配,裸 `reflect-cp/*` 匹配不到任何东西(实测返回空)。
            &format!("refs/tags/{CHECKPOINT_TAG_PREFIX}*"),
            // 按被标记提交的 committer date 倒序。用 `-creatordate` 时同一秒内
            // 创建的多个 checkpoint 顺序不稳定(落到 refname 字典序),列表会乱。
            "--sort=-committerdate",
            "--format=%(objectname)%1f%(creatordate:iso-strict)%1f%(contents:subject)",
        ],
    )?;
    let mut checkpoints = Vec::new();
    for line in out.lines() {
        let Some((sha, rest)) = line.split_once('\u{1f}') else {
            continue;
        };
        let Some((committed_at, subject)) = rest.split_once('\u{1f}') else {
            continue;
        };
        let Some(label) = subject.strip_prefix(CHECKPOINT_SUBJECT_PREFIX) else {
            continue; // 只收自己创建的提交
        };
        checkpoints.push(Checkpoint {
            sha: sha.to_string(),
            subject: label.trim().to_string(),
            committed_at: committed_at.to_string(),
        });
        if checkpoints.len() >= limit {
            break;
        }
    }
    Ok(checkpoints)
}

/// 回退到指定 checkpoint:`git reset --hard <sha>`。
///
/// **回退前先做一次快照提交**,把当前工作区状态存下来 —— 用户反悔了还能
/// 再回退到回退前的状态。返回 `(回退到的 sha, 回退前快照的 sha)`。
pub fn rewind(cwd: &Path, sha: &str) -> Result<(Checkpoint, Option<Checkpoint>), GitError> {
    if !is_repo(cwd) {
        return Err(GitError::NotARepo);
    }
    // 校验目标确实是本模块创建的 checkpoint,避免畸形/任意 sha 被打进
    // `reset --hard`。
    let target = list(cwd, 200)?
        .into_iter()
        .find(|c| c.sha == sha || c.sha.starts_with(sha))
        .ok_or_else(|| GitError::NotACheckpoint(sha.to_string()))?;

    // 先存当前状态。有未提交改动时这一步尤其重要。
    let safety = create(cwd, "pre-rewind safety snapshot")?;

    run_git(cwd, &["reset", "--hard", &target.sha])?;
    Ok((target, safety))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 建一个临时 git 仓库。git 不可用时跳过(CI 环境可能没有)。
    fn temp_repo(tag: &str) -> Option<std::path::PathBuf> {
        let dir = std::env::temp_dir().join(format!("reflect-checkpoint-{tag}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).ok()?;
        let ok = |args: &[&str]| {
            Command::new("git")
                .args(args)
                .current_dir(&dir)
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false)
        };
        if !ok(&["init", "-q"]) {
            return None;
        }
        // 固定身份,避免依赖用户全局 git config。
        ok(&["config", "user.email", "test@example.com"]);
        ok(&["config", "user.name", "Test"]);
        Some(dir)
    }

    fn write(dir: &Path, name: &str, body: &str) {
        std::fs::write(dir.join(name), body).unwrap();
    }

    #[test]
    fn create_returns_none_when_nothing_changed() {
        let Some(dir) = temp_repo("nochange") else {
            return;
        };
        write(&dir, "a.txt", "one");
        let first = create(&dir, "initial").unwrap();
        assert!(first.is_some(), "首次有改动应创建");
        // 再次创建,无改动 → None(不造空提交)。
        assert_eq!(create(&dir, "again").unwrap(), None);
    }

    #[test]
    fn list_only_returns_reflect_checkpoints() {
        let Some(dir) = temp_repo("list") else {
            return;
        };
        write(&dir, "a.txt", "one");
        create(&dir, "first").unwrap();
        // git 的日期粒度是秒,同一秒内的两个提交排序不稳定 —— 睡 1s 让
        // 「最新在前」这个断言有意义。
        std::thread::sleep(std::time::Duration::from_secs(1));
        write(&dir, "a.txt", "two");
        create(&dir, "second").unwrap();
        // 混进一个非 checkpoint 提交,必须被过滤掉。
        write(&dir, "b.txt", "x");
        run_git(&dir, &["add", "-A"]).unwrap();
        run_git(&dir, &["commit", "-m", "unrelated work"]).unwrap();

        let list = list(&dir, 50).unwrap();
        assert_eq!(list.len(), 2, "只应列出 reflect 自己的提交: {list:?}");
        assert_eq!(list[0].subject, "second", "最新在前");
        assert_eq!(list[1].subject, "first");
    }

    #[test]
    fn rewind_restores_files_and_takes_safety_snapshot() {
        let Some(dir) = temp_repo("rewind") else {
            return;
        };
        write(&dir, "a.txt", "v1");
        let cp1 = create(&dir, "v1").unwrap().unwrap();
        write(&dir, "a.txt", "v2");
        create(&dir, "v2").unwrap().unwrap();

        // 干净树上回退:safety 为 None 是正确的 —— 没有未提交改动可丢,
        // reset --hard 不会烧掉任何东西。
        let (target, safety) = rewind(&dir, &cp1.sha).unwrap();
        assert_eq!(target.subject, "v1");
        assert_eq!(safety, None, "干净树无需安全快照");
        assert_eq!(
            std::fs::read_to_string(dir.join("a.txt")).unwrap(),
            "v1",
            "文件应被 reset 回 v1"
        );
    }

    #[test]
    fn rewind_on_dirty_tree_snapshots_uncommitted_work() {
        // 关键安全属性:有未提交改动时回退,必须先把当前状态存成 checkpoint,
        // 否则 `reset --hard` 会把用户没提交的工作一次性烧掉且无法恢复。
        let Some(dir) = temp_repo("rewind-dirty") else {
            return;
        };
        write(&dir, "a.txt", "v1");
        let cp1 = create(&dir, "v1").unwrap().unwrap();
        write(&dir, "a.txt", "v2");
        create(&dir, "v2").unwrap().unwrap();
        // 制造未提交改动(真实场景:agent 改到一半,用户决定回退)。
        write(&dir, "a.txt", "v2-with-uncommitted-edits");

        let (target, safety) = rewind(&dir, &cp1.sha).unwrap();
        assert_eq!(target.subject, "v1");
        let safety = safety.expect("脏树回退前必须先存快照");
        assert_eq!(safety.subject, "pre-rewind safety snapshot");
        assert_eq!(
            std::fs::read_to_string(dir.join("a.txt")).unwrap(),
            "v1",
            "文件应被 reset 回 v1"
        );

        // 反悔:再回退到安全快照,应拿回未提交的编辑。
        let (back, _) = rewind(&dir, &safety.sha).unwrap();
        assert_eq!(back.subject, "pre-rewind safety snapshot");
        assert_eq!(
            std::fs::read_to_string(dir.join("a.txt")).unwrap(),
            "v2-with-uncommitted-edits",
            "安全快照必须能恢复未提交的工作"
        );
    }

    #[test]
    fn rewind_rejects_non_checkpoint_sha() {
        let Some(dir) = temp_repo("reject") else {
            return;
        };
        write(&dir, "a.txt", "one");
        run_git(&dir, &["add", "-A"]).unwrap();
        run_git(&dir, &["commit", "-m", "unrelated"]).unwrap();
        let head = run_git(&dir, &["rev-parse", "HEAD"])
            .unwrap()
            .trim()
            .to_string();

        // 非 checkpoint 提交不得被 reset --hard 打进去。
        assert_eq!(
            rewind(&dir, &head),
            Err(GitError::NotACheckpoint(head.clone()))
        );
        // 畸形 sha 同样拒绝。
        assert!(matches!(
            rewind(&dir, "deadbeef"),
            Err(GitError::NotACheckpoint(_))
        ));
    }

    #[test]
    fn non_repo_reports_not_a_repo() {
        let dir = std::env::temp_dir().join("reflect-checkpoint-nonrepo");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(create(&dir, "x"), Err(GitError::NotARepo));
        assert_eq!(list(&dir, 10), Err(GitError::NotARepo));
        assert!(!is_repo(&dir));
    }
}
