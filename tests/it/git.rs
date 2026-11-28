use std::collections::HashSet;

use orkestr8r::env::Env;
use orkestr8r::git::Git;
use orkestr8r::jobs::{JobStore, new_job_id};
use orkestr8r::orders::{FileChange, JobStatus, Order, Report, TokenCount};
use orkestr8r::process::ProcessRunner;

use crate::support::temp_repo::{TempRepo, git as run_git};

fn new_git() -> Git {
    Git::new(ProcessRunner::new(Env::from_process()))
}

/// A repo with one commit, and a job worktree `.ork/worktrees/<id>` on `ork/<id>` from HEAD.
async fn repo_with_worktree(id: &str) -> (TempRepo, Git, String, std::path::PathBuf) {
    let repo = TempRepo::with_files(&[("src/calc.rs", "pub fn one() -> i32 {\n    1\n}\n")]);
    let git = new_git();
    let base = git.head_sha(&repo.path).await.unwrap();
    let worktree = repo.path.join(".ork/worktrees").join(id);
    git.worktree_add(&repo.path, &worktree, &format!("ork/{id}"), &base)
        .await
        .unwrap();
    (repo, git, base, worktree)
}

#[tokio::test]
async fn a_worktree_is_made_from_head_on_its_own_branch() {
    let (repo, git, base, worktree) = repo_with_worktree("ab23").await;

    assert_eq!(git.current_branch(&worktree).await.unwrap(), "ork/ab23");
    assert_eq!(git.head_sha(&worktree).await.unwrap(), base);
    assert!(worktree.join("src/calc.rs").is_file());
    assert_eq!(
        git.repo_root(&worktree.join("src"))
            .await
            .unwrap()
            .file_name()
            .unwrap(),
        "ab23"
    );
    assert_ne!(git.current_branch(&repo.path).await.unwrap(), "ork/ab23");
}

#[tokio::test]
async fn commit_all_returns_none_when_nothing_changed() {
    let (_repo, git, base, worktree) = repo_with_worktree("ab23").await;

    assert_eq!(git.commit_all(&worktree, "nothing").await.unwrap(), None);

    std::fs::write(worktree.join("src/new.rs"), "pub fn two() {}\n").unwrap();
    let sha = git
        .commit_all(&worktree, "ork(ab23): add two")
        .await
        .unwrap()
        .unwrap();
    assert_ne!(sha, base);
    let log = last_commit(&worktree);
    assert!(log.contains("ork(ab23): add two"), "{log}");
}

/// The last commit's subject and author.
fn last_commit(dir: &std::path::Path) -> String {
    run_git(dir, &["log", "-1", "--format=%s by %an <%ae>"])
}

#[tokio::test]
async fn commits_are_made_by_orkestr8r() {
    let (_repo, git, _, worktree) = repo_with_worktree("ab23").await;
    std::fs::write(worktree.join("x.txt"), "x\n").unwrap();

    git.commit_all(&worktree, "ork(ab23): x").await.unwrap();

    assert_eq!(
        last_commit(&worktree).trim(),
        "ork(ab23): x by orkestr8r <orkestr8r@localhost>"
    );
}

#[tokio::test]
async fn num_stat_counts_lines_and_binary_files_as_zero() {
    let (repo, git, base, worktree) = repo_with_worktree("ab23").await;
    std::fs::write(
        worktree.join("src/calc.rs"),
        "pub fn one() -> i32 {\n    1 + 0\n}\n\npub fn two() {}\n",
    )
    .unwrap();
    std::fs::write(worktree.join("logo.bin"), [0u8, 159, 146, 150, 0, 1]).unwrap();
    git.commit_all(&worktree, "change").await.unwrap();

    let mut changes = git.num_stat(&repo.path, &base, "ork/ab23").await.unwrap();
    changes.sort_by(|a, b| a.path.cmp(&b.path));

    assert_eq!(
        changes,
        [
            FileChange {
                path: "logo.bin".into(),
                added: 0,
                removed: 0
            },
            FileChange {
                path: "src/calc.rs".into(),
                added: 3,
                removed: 1
            },
        ]
    );
    let stat = git.diff(&repo.path, &base, "ork/ab23", true).await.unwrap();
    assert!(stat.contains("2 files changed"), "{stat}");
}

#[tokio::test]
async fn a_no_ff_merge_makes_a_merge_commit() {
    let (repo, git, _, worktree) = repo_with_worktree("ab23").await;
    std::fs::write(worktree.join("src/new.rs"), "pub fn two() {}\n").unwrap();
    git.commit_all(&worktree, "add two").await.unwrap();

    git.merge(&repo.path, "ork/ab23", false, "ork(ab23): add two")
        .await
        .unwrap();

    assert_eq!(repo.read("src/new.rs"), "pub fn two() {}\n");
    let parents = repo.git(&["rev-list", "--parents", "-n", "1", "HEAD"]);
    assert_eq!(
        parents.split_whitespace().count(),
        3,
        "a merge commit has two parents"
    );
    assert_eq!(
        repo.git(&["log", "-1", "--format=%s"]).trim(),
        "ork(ab23): add two"
    );
}

#[tokio::test]
async fn a_squash_merge_makes_one_plain_commit() {
    let (repo, git, _, worktree) = repo_with_worktree("ab23").await;
    std::fs::write(worktree.join("src/new.rs"), "pub fn two() {}\n").unwrap();
    git.commit_all(&worktree, "add two").await.unwrap();

    git.merge(&repo.path, "ork/ab23", true, "ork(ab23): add two")
        .await
        .unwrap();

    assert_eq!(repo.read("src/new.rs"), "pub fn two() {}\n");
    let parents = repo.git(&["rev-list", "--parents", "-n", "1", "HEAD"]);
    assert_eq!(parents.split_whitespace().count(), 2, "one parent");
    assert_eq!(
        repo.git(&["log", "-1", "--format=%s"]).trim(),
        "ork(ab23): add two"
    );
}

#[tokio::test]
async fn a_conflict_aborts_and_changes_nothing() {
    for squash in [false, true] {
        let (repo, git, _, worktree) = repo_with_worktree("ab23").await;
        std::fs::write(
            worktree.join("src/calc.rs"),
            "pub fn one() -> i32 {\n    2\n}\n",
        )
        .unwrap();
        git.commit_all(&worktree, "two on the branch")
            .await
            .unwrap();
        repo.write("src/calc.rs", "pub fn one() -> i32 {\n    3\n}\n");
        repo.commit("three on main");
        let head = git.head_sha(&repo.path).await.unwrap();

        let err = git
            .merge(&repo.path, "ork/ab23", squash, "ork(ab23)")
            .await
            .unwrap_err();

        assert!(err.to_string().contains("conflict"), "{err}");
        assert_eq!(git.head_sha(&repo.path).await.unwrap(), head);
        assert!(
            git.is_clean(&repo.path).await.unwrap(),
            "squash={squash}: {}",
            repo.git(&["status"])
        );
        assert_eq!(
            repo.read("src/calc.rs"),
            "pub fn one() -> i32 {\n    3\n}\n"
        );
    }
}

#[tokio::test]
async fn removing_the_worktree_and_the_branch() {
    let (repo, git, _, worktree) = repo_with_worktree("ab23").await;
    std::fs::write(worktree.join("dirty.txt"), "uncommitted\n").unwrap();

    git.worktree_remove(&repo.path, &worktree).await.unwrap();
    git.branch_delete(&repo.path, "ork/ab23", true)
        .await
        .unwrap();

    assert!(!worktree.exists());
    assert!(
        repo.git(&["branch", "--list", "ork/ab23"])
            .trim()
            .is_empty()
    );
}

#[tokio::test]
async fn the_exclude_line_is_added_once() {
    let repo = TempRepo::with_files(&[("a.txt", "a\n")]);
    let git = new_git();

    assert!(git.add_exclude(&repo.path, ".ork/").await.unwrap());
    assert!(!git.add_exclude(&repo.path, ".ork/").await.unwrap());

    let exclude = std::fs::read_to_string(repo.path.join(".git/info/exclude")).unwrap();
    assert_eq!(exclude.lines().filter(|l| *l == ".ork/").count(), 1);
}

#[tokio::test]
async fn is_clean_ignores_dot_ork() {
    let repo = TempRepo::with_files(&[("a.txt", "a\n")]);
    let git = new_git();
    repo.write(".ork/jobs/ab23/state.json", "{}");

    assert!(git.is_clean(&repo.path).await.unwrap());

    repo.write("b.txt", "untracked\n");
    assert!(!git.is_clean(&repo.path).await.unwrap());
}

#[tokio::test]
async fn ls_files_lists_tracked_and_untracked_but_not_ignored() {
    let repo = TempRepo::with_files(&[("a.txt", "a\n"), (".gitignore", "target/\n")]);
    repo.write("b.txt", "new\n");
    repo.write("target/out.bin", "x");

    let mut files = new_git().ls_files(&repo.path).await.unwrap();
    files.sort();

    assert_eq!(files, [".gitignore", "a.txt", "b.txt"]);
}

#[test]
fn job_ids_are_unique_and_skip_taken_ones() {
    let mut seen = HashSet::new();
    for _ in 0..2000 {
        let id = new_job_id(|id| seen.contains(id));
        assert_eq!(id.len(), 4);
        assert!(
            id.chars()
                .all(|c| "abcdefghjkmnpqrstuvwxyz23456789".contains(c)),
            "{id}"
        );
        assert!(seen.insert(id));
    }
}

fn order() -> Order {
    Order {
        goal: "Add Sum to calc".into(),
        write_scope: vec!["src/**".into()],
        checks: vec!["git grep -q Sum".into()],
        ..Default::default()
    }
}

#[test]
fn the_job_store_keeps_state_order_and_report() {
    let repo = TempRepo::new();
    let store = JobStore::new(&repo.path);

    let first = store.create(&order(), "abc123").unwrap();
    std::thread::sleep(std::time::Duration::from_millis(5));
    let second = store.create(&order(), "abc123").unwrap();

    assert_eq!(first.status, JobStatus::Queued, "a new job waits to start");
    assert_eq!(first.branch, format!("ork/{}", first.id));
    assert_eq!(first.worktree, format!(".ork/worktrees/{}", first.id));
    assert_eq!(store.load(&first.id).unwrap(), first);
    assert_eq!(store.load_order(&first.id).unwrap(), order());
    let listed: Vec<String> = store.list().unwrap().into_iter().map(|j| j.id).collect();
    assert_eq!(
        listed,
        [second.id.clone(), first.id.clone()],
        "newest first"
    );

    let updated = store
        .set_status(&first.id, JobStatus::GaveUp, Some("max_turns".into()))
        .unwrap();
    assert_eq!(store.load(&first.id).unwrap(), updated);
    assert_eq!(updated.reason.as_deref(), Some("max_turns"));
    assert!(updated.updated_at >= first.updated_at);
    assert_eq!(first.finished_at, None, "a new job is still running");
    let finished = updated.finished_at.expect("giving up ends the run");
    let discarded = store
        .set_status(&first.id, JobStatus::Discarded, None)
        .unwrap();
    assert_eq!(
        discarded.finished_at,
        Some(finished),
        "discarding later doesn't move the end"
    );

    assert_eq!(store.load_report(&first.id).unwrap(), None);
    let report = Report {
        job: first.id.clone(),
        status: JobStatus::GaveUp,
        summary: String::new(),
        files_changed: vec![],
        checks: vec![],
        repairs_used: 0,
        turns: 40,
        duration_s: 0.0,
        tool_calls: 0,
        failed_tool_calls: 0,
        files_read: vec![],
        failing_before: vec![],
        tokens: TokenCount {
            input: 1,
            output: 2,
        },
        model: "m".into(),
        role: "grunt".into(),
        branch: first.branch.clone(),
        open_questions: vec![],
        reason: Some("max_turns".into()),
    };
    store.write_report(&first.id, &report).unwrap();
    assert_eq!(store.load_report(&first.id).unwrap(), Some(report));

    assert!(
        store
            .load("zzzz")
            .unwrap_err()
            .to_string()
            .contains("no job zzzz")
    );
}

#[test]
fn state_survives_a_crash_mid_write() {
    let repo = TempRepo::new();
    let store = JobStore::new(&repo.path);
    let job = store.create(&order(), "abc123").unwrap();
    let paths = store.paths(&job.id);

    // a crash after the temp file was half written, before the rename
    let tmp = paths.state.with_file_name("state.json.tmp");
    std::fs::write(&tmp, "{\"id\": \"ab").unwrap();

    assert_eq!(store.load(&job.id).unwrap(), job, "the old state is whole");
    assert_eq!(store.list().unwrap().len(), 1);
    // and the next write goes through
    store
        .set_status(&job.id, JobStatus::Error, Some("cancelled".into()))
        .unwrap();
    assert_eq!(store.load(&job.id).unwrap().status, JobStatus::Error);
}

#[test]
fn job_ids_from_the_command_line_cannot_leave_the_jobs_directory() {
    let repo = TempRepo::new();
    let store = JobStore::new(&repo.path);
    repo.write("secret/state.json", "{}");

    for id in ["../../secret", "..", "a/b", "AB12", ""] {
        let err = store.load(id).unwrap_err().to_string();
        assert!(err.contains("is not a job id"), "{id:?}: {err}");
    }
}

#[test]
fn a_state_file_that_describes_another_job_is_refused() {
    let repo = TempRepo::new();
    let store = JobStore::new(&repo.path);
    let job = store.create(&order(), "abc1234").unwrap();
    let path = store.paths(&job.id).state;
    let original = std::fs::read_to_string(&path).unwrap();

    // a branch that git would take as an option, a sha that isn't one, another job's id
    for (from, to) in [
        (
            format!("\"ork/{}\"", job.id),
            "\"--upload-pack=touch /tmp/x\"".to_string(),
        ),
        ("\"abc1234\"".to_string(), "\"--output=/tmp/x\"".to_string()),
        (
            format!("\"id\": \"{}\"", job.id),
            "\"id\": \"zzzz\"".to_string(),
        ),
    ] {
        std::fs::write(&path, original.replace(&from, &to)).unwrap();
        let err = store.load(&job.id).unwrap_err().to_string();
        assert!(err.contains("is damaged"), "{to}: {err}");
    }
}
