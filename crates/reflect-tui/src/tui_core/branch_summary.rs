//! 分支和拉取请求元数据，用于 TUI 状态行项目。
//!
//! 本模块拥有 TUI `git-branch`、`pull-request-number` 和 `branch-changes` 状态行项目背后的
//! git 和 GitHub 探针。它故意只与 `WorkspaceCommandExecutor` 对话，而不是与 `tokio::process::Command`，
//! 所以相同的查找逻辑在 TUI 连接到嵌入式或远程 app-server 时都能工作。
//!
//! 所有查找都是尽力而为。失败的命令、缺失的 `git` 或 `gh`、未认证的 GitHub CLI、
//! 非 git 目录或模糊的仓库状态应导致可选元数据缺失，而不是用户可见的错误。
//! 状态行然后可以渲染可用的部分，而不阻塞其余 UI。

#[cfg(test)]
use std::collections::VecDeque;
use std::path::Path;

use serde::Deserialize;

use crate::tui_core::workspace_command::WorkspaceCommand;
#[cfg(test)]
use crate::tui_core::workspace_command::WorkspaceCommandError;
use crate::tui_core::workspace_command::WorkspaceCommandExecutor;
use crate::tui_core::workspace_command::WorkspaceCommandOutput;

/// `HEAD` 与分支比较基准之间的新增和删除行数。
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct GitBranchDiffStats {
    /// 当前分支上已提交变更的总新增行数。
    pub(crate) additions: u64,
    /// 当前分支上已提交变更的总删除行数。
    pub(crate) deletions: u64,
}

/// 状态行为一个工作目录缓存的组合 git 元数据。
///
/// 当另一个探测失败时，摘要可能只包含其中一个字段。渲染器应将
/// 缺失的字段视为省略的可选 UI，而不是硬查找失败。
#[derive(Clone, Debug, Default)]
pub(crate) struct StatusLineGitSummary {
    /// 与当前分支或 HEAD 提交关联的打开的拉取请求。
    pub(crate) pull_request: Option<StatusLinePullRequest>,
    /// `HEAD` 与仓库默认分支合并基础之间的新增和删除行数。
    pub(crate) branch_change_stats: Option<GitBranchDiffStats>,
}

/// `pull-request-number` 状态行项目显示的打开的 GitHub 拉取请求。
///
/// URL 与编号一起保留，以便可点击的渲染器可以打开同一 PR。调用者应仅为打开的 PR 构造此结构；
/// 已关闭或合并的 PR 会被此模块过滤掉。
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct StatusLinePullRequest {
    /// GitHub 拉取请求编号。
    pub(crate) number: u64,
    /// 拉取请求的浏览器 URL。
    pub(crate) url: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct DefaultBranch {
    /// 用于合并基础比较的 Git 引用。
    ///
    /// 这可能是远程跟踪引用如 `refs/remotes/origin/main`，避免与过时的或不存在
    /// 的本地 `main` 分支进行比较。
    merge_ref: String,
}

#[derive(Deserialize)]
struct GhPullRequestView {
    number: u64,
    url: String,
    state: String,
}

#[derive(Deserialize)]
struct GhPullRequestApiItem {
    number: u64,
    #[serde(rename = "html_url")]
    url: String,
    state: String,
}

#[derive(Deserialize)]
struct GhRepoView {
    #[serde(rename = "nameWithOwner")]
    name_with_owner: Option<String>,
    parent: Option<GhRepoParent>,
}

#[derive(Deserialize)]
struct GhRepoParent {
    #[serde(rename = "nameWithOwner")]
    name_with_owner: String,
}

/// 返回一个状态行工作目录中签出的分支名称。
///
/// 分离的 HEAD、非 git 目录和命令失败返回 `None`，这样渲染器可以
/// 省略分支项目，而不会暴露后台查找错误。
pub(crate) async fn current_branch_name(
    runner: &dyn WorkspaceCommandExecutor,
    cwd: &Path,
) -> Option<String> {
    let output = run_git_command(runner, cwd, &["branch", "--show-current"])
        .await
        .ok()?;
    if !output.success() {
        return None;
    }

    Some(output.stdout.trim().to_string()).filter(|name| !name.is_empty())
}

/// 解析一个状态行工作目录的 PR 和分支变更元数据。
///
/// PR 和 diff 统计探测并发运行，因为每个都是独立的且都是可选的。
/// 返回的摘要适合按 `cwd` 缓存；如果异步查找完成前活动状态行 cwd 发生变化，调用者应丢弃它。
pub(crate) async fn status_line_git_summary(
    runner: &dyn WorkspaceCommandExecutor,
    cwd: &Path,
) -> StatusLineGitSummary {
    let (pull_request, branch_change_stats) = tokio::join!(
        open_pull_request(runner, cwd),
        branch_diff_stats_to_default_branch(runner, cwd),
    );
    StatusLineGitSummary {
        pull_request,
        branch_change_stats,
    }
}

/// 统计 `HEAD` 与仓库默认分支之间的已提交行变更。
///
/// 比较基准是与已验证默认分支引用的合并基础。未提交的工作树编辑被故意忽略，因为状态行项目总结的是签出的分支，而不是当前脏工作树。
async fn branch_diff_stats_to_default_branch(
    runner: &dyn WorkspaceCommandExecutor,
    cwd: &Path,
) -> Option<GitBranchDiffStats> {
    let git_dir = run_git_command(runner, cwd, &["rev-parse", "--git-dir"])
        .await
        .ok()?;
    if !git_dir.success() {
        return None;
    }

    let default_branch = get_default_branch(runner, cwd).await?;
    let merge_base = run_git_command(
        runner,
        cwd,
        &["merge-base", "HEAD", &default_branch.merge_ref],
    )
    .await
    .ok()?;
    if !merge_base.success() {
        return None;
    }
    let merge_base = merge_base.stdout.trim();
    if merge_base.is_empty() {
        return None;
    }

    let range = format!("{merge_base}..HEAD");
    let numstat = run_git_command(runner, cwd, &["diff", "--numstat", &range])
        .await
        .ok()?;
    if !numstat.success() {
        return None;
    }

    let mut additions = 0_u64;
    let mut deletions = 0_u64;
    for line in numstat.stdout.lines() {
        let mut columns = line.split('\t');
        additions += columns
            .next()
            .and_then(|value| value.parse().ok())
            .unwrap_or(0);
        deletions += columns
            .next()
            .and_then(|value| value.parse().ok())
            .unwrap_or(0);
    }

    Some(GitBranchDiffStats {
        additions,
        deletions,
    })
}

/// 按默认分支发现使用的顺序返回 git 远程仓库。
///
/// `origin` 优先级更高，因为大多数仓库将其用作规范的上游。仍然尝试其他远程仓库，
/// 这样具有不同命名上游的分支或企业布局在其远程 HEAD 配置时可以生成分支变更统计。
async fn get_git_remotes(runner: &dyn WorkspaceCommandExecutor, cwd: &Path) -> Option<Vec<String>> {
    let output = run_git_command(runner, cwd, &["remote"]).await.ok()?;
    if !output.success() {
        return None;
    }

    let mut remotes: Vec<String> = output.stdout.lines().map(str::to_string).collect();
    if let Some(pos) = remotes.iter().position(|remote| remote == "origin") {
        let origin = remotes.remove(pos);
        remotes.insert(0, origin);
    }
    Some(remotes)
}

/// 解析分支变更比较应使用的默认分支引用。
///
/// 查找优先使用远程跟踪引用而不是本地分支，这样仅功能克隆和过时的
/// 本地 `main` 分支不会膨胀状态行差异。当没有可用的远程默认分支时，
/// 本地 `main` 或 `master` 作为最后手段。
async fn get_default_branch(
    runner: &dyn WorkspaceCommandExecutor,
    cwd: &Path,
) -> Option<DefaultBranch> {
    let remotes = get_git_remotes(runner, cwd).await.unwrap_or_default();
    for remote in remotes {
        if let Some(branch) =
            get_remote_default_branch_from_symbolic_ref(runner, cwd, &remote).await
        {
            return Some(branch);
        }

        if let Some(branch) = get_remote_default_branch_from_remote_show(runner, cwd, &remote).await
        {
            return Some(branch);
        }
    }

    get_default_branch_local(runner, cwd).await
}

/// 将远程仓库的符号 HEAD 解析为具体的远程跟踪引用。
///
/// 返回的引用在使用前会验证。没有这个检查，旧的 `origin/HEAD` 可能指向
/// 不再存在的引用，导致后续的合并基础探测在更不明显的地方失败。
async fn get_remote_default_branch_from_symbolic_ref(
    runner: &dyn WorkspaceCommandExecutor,
    cwd: &Path,
    remote: &str,
) -> Option<DefaultBranch> {
    let remote_head = format!("refs/remotes/{remote}/HEAD");
    let output = run_git_command(runner, cwd, &["symbolic-ref", "--quiet", &remote_head])
        .await
        .ok()?;
    if !output.success() {
        return None;
    }

    let trimmed = output.stdout.trim();
    let remote_ref_prefix = format!("refs/remotes/{remote}/");
    trimmed.strip_prefix(&remote_ref_prefix)?;
    if !git_ref_exists(runner, cwd, trimmed).await {
        return None;
    }

    Some(DefaultBranch {
        merge_ref: trimmed.to_string(),
    })
}

/// 解析 `git remote show` 输出以发现远程仓库的默认分支引用。
///
/// 当 `refs/remotes/<remote>/HEAD` 未配置但 `git remote show` 仍能报告
/// 上游 HEAD 分支时，这是备用方案。具体的远程跟踪引用在接收前必须已存在于本地。
async fn get_remote_default_branch_from_remote_show(
    runner: &dyn WorkspaceCommandExecutor,
    cwd: &Path,
    remote: &str,
) -> Option<DefaultBranch> {
    let output = run_git_command(runner, cwd, &["remote", "show", remote])
        .await
        .ok()?;
    if !output.success() {
        return None;
    }

    for line in output.stdout.lines() {
        let line = line.trim();
        let Some(rest) = line.strip_prefix("HEAD branch:") else {
            continue;
        };
        let name = rest.trim();
        let remote_ref = format!("refs/remotes/{remote}/{name}");
        if !name.is_empty() && git_ref_exists(runner, cwd, &remote_ref).await {
            return Some(DefaultBranch {
                merge_ref: remote_ref,
            });
        }
    }

    None
}

/// 当找不到远程默认分支时，回退到本地 `main` 或 `master`。
async fn get_default_branch_local(
    runner: &dyn WorkspaceCommandExecutor,
    cwd: &Path,
) -> Option<DefaultBranch> {
    for candidate in ["main", "master"] {
        let local_ref = format!("refs/heads/{candidate}");
        if git_ref_exists(runner, cwd, &local_ref).await {
            return Some(DefaultBranch {
                merge_ref: local_ref,
            });
        }
    }

    None
}

/// 检查 git 引用是否存在于状态行工作目录中。
async fn git_ref_exists(
    runner: &dyn WorkspaceCommandExecutor,
    cwd: &Path,
    reference: &str,
) -> bool {
    run_git_command(
        runner,
        cwd,
        &["rev-parse", "--verify", "--quiet", reference],
    )
    .await
    .is_ok_and(|output| output.success())
}

/// 解析与当前检出关联的打开的 PR。
///
/// 先尝试基于分支的查找，因为它便宜且镜像 `gh pr view`。基于提交的
/// 查找用作后备，这样即使 `gh` 从当前检出入推断分支，分支工作流仍能找到针对上游
/// 仓库打开的 PR。
async fn open_pull_request(
    runner: &dyn WorkspaceCommandExecutor,
    cwd: &Path,
) -> Option<StatusLinePullRequest> {
    if let Some(pull_request) = open_pull_request_for_current_branch(runner, cwd).await {
        return Some(pull_request);
    }

    open_pull_request_for_head_commit(runner, cwd).await
}

/// 使用 GitHub CLI 的当前分支 PR 查找。
async fn open_pull_request_for_current_branch(
    runner: &dyn WorkspaceCommandExecutor,
    cwd: &Path,
) -> Option<StatusLinePullRequest> {
    let output = run_gh_command(runner, cwd, &["pr", "view", "--json", "number,url,state"])
        .await
        .ok()?;
    if !output.success() {
        return None;
    }
    pull_request_from_view_output(&output.stdout)
}

/// 在上游/分支仓库搜索顺序中查找 `HEAD` 的打开 PR。
async fn open_pull_request_for_head_commit(
    runner: &dyn WorkspaceCommandExecutor,
    cwd: &Path,
) -> Option<StatusLinePullRequest> {
    let head_sha = current_head_sha(runner, cwd).await?;
    for repo in gh_repo_search_order(runner, cwd).await? {
        let endpoint = format!("repos/{repo}/commits/{head_sha}/pulls");
        let output = run_gh_command(
            runner,
            cwd,
            &[
                "api",
                "-H",
                "Accept: application/vnd.github+json",
                &endpoint,
            ],
        )
        .await
        .ok()?;
        if output.success()
            && let Some(pull_request) = pull_request_from_api_output(&output.stdout)
        {
            return Some(pull_request);
        }
    }

    None
}

/// 返回当前 `HEAD` SHA，用于基于提交的 PR 查找。
async fn current_head_sha(runner: &dyn WorkspaceCommandExecutor, cwd: &Path) -> Option<String> {
    let output = run_git_command(runner, cwd, &["rev-parse", "HEAD"])
        .await
        .ok()?;
    if !output.success() {
        return None;
    }

    Some(output.stdout.trim().to_string()).filter(|sha| !sha.is_empty())
}

/// 返回用于提交关联 PR 查询的仓库列表，父仓库在分支仓库之前。
async fn gh_repo_search_order(
    runner: &dyn WorkspaceCommandExecutor,
    cwd: &Path,
) -> Option<Vec<String>> {
    let output = run_gh_command(
        runner,
        cwd,
        &["repo", "view", "--json", "nameWithOwner,parent"],
    )
    .await
    .ok()?;
    if !output.success() {
        return None;
    }

    repo_search_order_from_output(&output.stdout)
}

/// 解析 `gh pr view --json number,url,state` 输出以获取打开的 PR。
fn pull_request_from_view_output(stdout: &str) -> Option<StatusLinePullRequest> {
    let pull_request = serde_json::from_str::<GhPullRequestView>(stdout).ok()?;
    pull_request
        .state
        .eq_ignore_ascii_case("open")
        .then_some(StatusLinePullRequest {
            number: pull_request.number,
            url: pull_request.url,
        })
}

/// 解析 GitHub REST 提交到 PR 响应并返回第一个打开的 PR。
fn pull_request_from_api_output(stdout: &str) -> Option<StatusLinePullRequest> {
    serde_json::from_str::<Vec<GhPullRequestApiItem>>(stdout)
        .ok()?
        .into_iter()
        .find(|pull_request| pull_request.state.eq_ignore_ascii_case("open"))
        .map(|pull_request| StatusLinePullRequest {
            number: pull_request.number,
            url: pull_request.url,
        })
}

/// 将 `gh repo view` 输出解析为后备 PR 查找的仓库搜索顺序。
///
/// 父仓库优先排序匹配上游 PR 工作流：分支可能从派生仓库检出，
/// 但打开的 PR 位于父仓库上。
fn repo_search_order_from_output(stdout: &str) -> Option<Vec<String>> {
    let repo = serde_json::from_str::<GhRepoView>(stdout).ok()?;
    let mut repos = Vec::new();
    if let Some(parent) = repo.parent {
        repos.push(parent.name_with_owner);
    }
    if let Some(name_with_owner) = repo.name_with_owner
        && !repos.iter().any(|repo| repo == &name_with_owner)
    {
        repos.push(name_with_owner);
    }
    if repos.is_empty() {
        return None;
    }

    Some(repos)
}

/// 通过工作区命令抽象运行 git 命令。
async fn run_git_command(
    runner: &dyn WorkspaceCommandExecutor,
    cwd: &Path,
    args: &[&str],
) -> Result<WorkspaceCommandOutput, crate::tui_core::workspace_command::WorkspaceCommandError> {
    let mut argv = Vec::with_capacity(args.len() + 1);
    argv.push("git".to_string());
    argv.extend(args.iter().map(|arg| (*arg).to_string()));
    runner
        .run(
            WorkspaceCommand::new(argv)
                .cwd(cwd.to_path_buf())
                .env("GIT_OPTIONAL_LOCKS", "0"),
        )
        .await
}

/// 通过工作区命令抽象运行 GitHub CLI 命令。
///
/// 禁用提示，因为状态行探测是后台 UI 工作。需要认证或用户输入的命令应该失败并隐藏可选的 PR 项目。
async fn run_gh_command(
    runner: &dyn WorkspaceCommandExecutor,
    cwd: &Path,
    args: &[&str],
) -> Result<WorkspaceCommandOutput, crate::tui_core::workspace_command::WorkspaceCommandError> {
    let mut argv = Vec::with_capacity(args.len() + 1);
    argv.push("gh".to_string());
    argv.extend(args.iter().map(|arg| (*arg).to_string()));
    runner
        .run(
            WorkspaceCommand::new(argv)
                .cwd(cwd.to_path_buf())
                .env("GH_PROMPT_DISABLED", "1")
                .env("GIT_TERMINAL_PROMPT", "0"),
        )
        .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui_core::workspace_command::WorkspaceCommand;
    use pretty_assertions::assert_eq;
    use std::future::Future;
    use std::pin::Pin;
    use std::sync::Mutex;

    #[tokio::test]
    async fn branch_diff_stats_prefers_remote_default_ref_over_stale_local_branch() {
        let runner = FakeRunner::new(vec![
            response(
                &["git", "rev-parse", "--git-dir"],
                /*exit_code*/ 0,
                ".git\n",
            ),
            response(&["git", "remote"], /*exit_code*/ 0, "origin\n"),
            response(
                &["git", "symbolic-ref", "--quiet", "refs/remotes/origin/HEAD"],
                /*exit_code*/ 0,
                "refs/remotes/origin/main\n",
            ),
            response(
                &[
                    "git",
                    "rev-parse",
                    "--verify",
                    "--quiet",
                    "refs/remotes/origin/main",
                ],
                /*exit_code*/ 0,
                "remote-main-sha\n",
            ),
            response(
                &["git", "merge-base", "HEAD", "refs/remotes/origin/main"],
                /*exit_code*/ 0,
                "base-sha\n",
            ),
            response(
                &["git", "diff", "--numstat", "base-sha..HEAD"],
                /*exit_code*/ 0,
                "1\t0\tfile\n",
            ),
        ]);

        let stats = branch_diff_stats_to_default_branch(&runner, Path::new("/repo"))
            .await
            .expect("branch diff stats");

        assert_eq!(
            stats,
            GitBranchDiffStats {
                additions: 1,
                deletions: 0,
            }
        );
        assert!(runner.saw(&["git", "merge-base", "HEAD", "refs/remotes/origin/main"]));
    }

    #[tokio::test]
    async fn open_pull_request_uses_current_branch_view_first() {
        let runner = FakeRunner::new(vec![response(
            &["gh", "pr", "view", "--json", "number,url,state"],
            /*exit_code*/ 0,
            r#"{"number":20252,"url":"https://github.com/asketisch/reflect/pull/20252","state":"OPEN"}"#,
        )]);

        let pull_request = open_pull_request(&runner, Path::new("/repo"))
            .await
            .expect("pull request");

        assert_eq!(
            pull_request,
            StatusLinePullRequest {
                number: 20_252,
                url: "https://github.com/asketisch/reflect/pull/20252".to_string(),
            }
        );
        assert!(!runner.saw(&["git", "rev-parse", "HEAD"]));
    }

    #[tokio::test]
    async fn open_pull_request_falls_back_to_parent_repo_commit_lookup() {
        let runner = FakeRunner::new(vec![
            response(
                &["gh", "pr", "view", "--json", "number,url,state"],
                /*exit_code*/ 1,
                "",
            ),
            response(
                &["git", "rev-parse", "HEAD"],
                /*exit_code*/ 0,
                "head-sha\n",
            ),
            response(
                &["gh", "repo", "view", "--json", "nameWithOwner,parent"],
                /*exit_code*/ 0,
                r#"{"nameWithOwner":"asketisch/reflect-tui","parent":{"nameWithOwner":"asketisch/reflect"}}"#,
            ),
            response(
                &[
                    "gh",
                    "api",
                    "-H",
                    "Accept: application/vnd.github+json",
                    "repos/asketisch/reflect/commits/head-sha/pulls",
                ],
                /*exit_code*/ 0,
                r#"[{"number":20252,"html_url":"https://github.com/asketisch/reflect/pull/20252","state":"open"}]"#,
            ),
        ]);

        let pull_request = open_pull_request(&runner, Path::new("/repo"))
            .await
            .expect("pull request");

        assert_eq!(
            pull_request,
            StatusLinePullRequest {
                number: 20_252,
                url: "https://github.com/asketisch/reflect/pull/20252".to_string(),
            }
        );
        assert!(runner.saw(&[
            "gh",
            "api",
            "-H",
            "Accept: application/vnd.github+json",
            "repos/asketisch/reflect/commits/head-sha/pulls",
        ]));
    }

    #[test]
    fn status_line_pr_view_parser_requires_open_pr() {
        assert_eq!(
            pull_request_from_view_output(
                r#"{"number":20252,"url":"https://github.com/asketisch/reflect/pull/20252","state":"OPEN"}"#
            ),
            Some(StatusLinePullRequest {
                number: 20_252,
                url: "https://github.com/asketisch/reflect/pull/20252".to_string(),
            })
        );

        assert_eq!(
            pull_request_from_view_output(
                r#"{"number":20252,"url":"https://github.com/asketisch/reflect/pull/20252","state":"MERGED"}"#
            ),
            None
        );
    }

    #[test]
    fn status_line_pr_fallback_searches_parent_repo_first() {
        assert_eq!(
            repo_search_order_from_output(
                r#"{"nameWithOwner":"asketisch/reflect-tui","parent":{"nameWithOwner":"asketisch/reflect"}}"#
            ),
            Some(vec!["asketisch/reflect".to_string(), "asketisch/reflect-tui".to_string()])
        );
    }

    fn response(argv: &[&str], exit_code: i32, stdout: &str) -> FakeResponse {
        FakeResponse {
            argv: argv.iter().map(|arg| (*arg).to_string()).collect(),
            output: WorkspaceCommandOutput {
                exit_code,
                stdout: stdout.to_string(),
                stderr: String::new(),
            },
        }
    }

    struct FakeResponse {
        argv: Vec<String>,
        output: WorkspaceCommandOutput,
    }

    struct FakeRunner {
        responses: Mutex<VecDeque<FakeResponse>>,
        seen: Mutex<Vec<Vec<String>>>,
    }

    impl FakeRunner {
        fn new(responses: Vec<FakeResponse>) -> Self {
            Self {
                responses: Mutex::new(responses.into()),
                seen: Mutex::new(Vec::new()),
            }
        }

        fn saw(&self, argv: &[&str]) -> bool {
            let argv: Vec<String> = argv.iter().map(|arg| (*arg).to_string()).collect();
            self.seen
                .lock()
                .expect("seen lock")
                .iter()
                .any(|seen| seen == &argv)
        }
    }

    impl WorkspaceCommandExecutor for FakeRunner {
        fn run(
            &self,
            command: WorkspaceCommand,
        ) -> Pin<
            Box<
                dyn Future<Output = Result<WorkspaceCommandOutput, WorkspaceCommandError>>
                    + Send
                    + '_,
            >,
        > {
            self.seen
                .lock()
                .expect("seen lock")
                .push(command.argv.clone());
            Box::pin(async move {
                let mut responses = self.responses.lock().expect("responses lock");
                let index = responses
                    .iter()
                    .position(|response| response.argv == command.argv)
                    .unwrap_or_else(|| panic!("missing fake response for {:?}", command.argv));
                let response = responses.remove(index).expect("fake response");
                Ok(response.output)
            })
        }
    }
}
