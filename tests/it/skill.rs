//! `ork skill install` and `ork skill list`: the skill goes where each agent reads it, for you or
//! for one repo, and says when it's missing or out of date.

use std::path::{Path, PathBuf};

use orkestr8r::cli::exit;
use orkestr8r::env::Env;
use orkestr8r::skill::{self, Agent, Installed, Scope};
use serde_json::Value;
use tempfile::TempDir;

use crate::support::cli::run_cli;
use crate::support::temp_repo::TempRepo;
use crate::support::workspace::json_lines;

/// A home directory of its own, and nothing on PATH.
fn home_env(home: &Path) -> Env {
    let home = home.display().to_string();
    Env::from_pairs([("USERPROFILE", home.clone()), ("HOME", home)])
}

fn skill_at(dir: &Path, rel: &str) -> PathBuf {
    dir.join(rel).join("orkestr8r").join("SKILL.md")
}

fn result(stdout: &str) -> Value {
    json_lines(stdout).pop().expect("a result line")
}

#[tokio::test]
async fn install_writes_the_skill_where_claude_code_reads_it() {
    let home = TempDir::new().unwrap();
    let out = run_cli(
        &["skill", "install", "claude"],
        home.path(),
        home_env(home.path()),
    )
    .await;

    assert_eq!(out.code, exit::OK, "{}", out.stderr);
    let path = skill_at(home.path(), ".claude/skills");
    assert_eq!(std::fs::read_to_string(&path).unwrap(), skill::SKILL);
    assert!(out.stdout.contains("Claude Code"), "{}", out.stdout);
    assert!(out.stdout.contains("installed"), "{}", out.stdout);
    // Only the agent asked for.
    assert!(!home.path().join(".agents").exists());
    assert!(!home.path().join(".gemini").exists());
}

#[tokio::test]
async fn each_agent_gets_its_own_folder() {
    let home = TempDir::new().unwrap();
    let out = run_cli(
        &["skill", "install", "all", "--json"],
        home.path(),
        home_env(home.path()),
    )
    .await;

    assert_eq!(out.code, exit::OK, "{}", out.stderr);
    for rel in [
        ".claude/skills",
        ".config/opencode/skills",
        ".agents/skills",
        ".gemini/skills",
    ] {
        assert!(skill_at(home.path(), rel).is_file(), "nothing in {rel}");
    }
    let result = result(&out.stdout);
    let installed = result["data"]["installed"].as_array().unwrap();
    assert_eq!(installed.len(), 5, "{result}");
    // Codex and "other" share ~/.agents/skills: the second finds it there already.
    let changes: Vec<&str> = installed
        .iter()
        .map(|i| i["change"].as_str().unwrap())
        .collect();
    assert_eq!(
        changes,
        [
            "installed",
            "installed",
            "installed",
            "installed",
            "unchanged"
        ]
    );
}

#[tokio::test]
async fn the_agents_own_variables_move_their_folders() {
    let home = TempDir::new().unwrap();
    let claude = home.path().join("claude-config");
    let xdg = home.path().join("xdg");
    let mut env = home_env(home.path());
    env.set("CLAUDE_CONFIG_DIR", claude.display().to_string());
    env.set("XDG_CONFIG_HOME", xdg.display().to_string());

    let out = run_cli(
        &["skill", "install", "claude", "opencode"],
        home.path(),
        env,
    )
    .await;

    assert_eq!(out.code, exit::OK, "{}", out.stderr);
    assert!(skill_at(&claude, "skills").is_file());
    assert!(skill_at(&xdg, "opencode/skills").is_file());
    assert!(!home.path().join(".claude").exists());
}

#[tokio::test]
async fn an_old_skill_is_updated_and_a_current_one_left_alone() {
    let home = TempDir::new().unwrap();
    let path = skill_at(home.path(), ".claude/skills");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, "---\nname: orkestr8r\ndescription: old\n---\n").unwrap();
    let env = home_env(home.path());
    let targets = |env: &Env| skill::targets(env, home.path(), Scope::User, None).unwrap();
    assert_eq!(targets(&env)[0].installed, Installed::Outdated);

    let first = run_cli(
        &["skill", "install", "claude", "--json"],
        home.path(),
        env.clone(),
    )
    .await;
    let second = run_cli(
        &["skill", "install", "claude", "--json"],
        home.path(),
        env.clone(),
    )
    .await;

    assert_eq!(
        result(&first.stdout)["data"]["installed"][0]["change"],
        "updated"
    );
    assert_eq!(
        result(&second.stdout)["data"]["installed"][0]["change"],
        "unchanged"
    );
    assert_eq!(std::fs::read_to_string(&path).unwrap(), skill::SKILL);
    assert_eq!(targets(&env)[0].installed, Installed::Current);
}

#[tokio::test]
async fn without_agents_it_asks_which_and_names_the_ones_it_found() {
    let home = TempDir::new().unwrap();
    std::fs::create_dir_all(home.path().join(".claude")).unwrap();
    std::fs::create_dir_all(home.path().join(".codex")).unwrap();

    let out = run_cli(
        &["skill", "install", "--json"],
        home.path(),
        home_env(home.path()),
    )
    .await;

    assert_eq!(out.code, exit::ERROR);
    let error = result(&out.stdout)["error"].as_str().unwrap().to_string();
    assert!(error.contains("ork skill install claude"), "{error}");
    assert!(error.contains("On this machine: claude, codex."), "{error}");
    // Nothing written.
    assert!(!home.path().join(".claude/skills").exists());
}

#[tokio::test]
async fn an_unknown_agent_is_refused() {
    let home = TempDir::new().unwrap();
    let out = run_cli(
        &["skill", "install", "vim"],
        home.path(),
        home_env(home.path()),
    )
    .await;

    assert_eq!(out.code, exit::ERROR);
    assert!(out.stderr.contains("claude"), "{}", out.stderr);
}

#[tokio::test]
async fn repo_puts_it_in_the_repo_for_everyone() {
    let home = TempDir::new().unwrap();
    let repo = TempRepo::new();
    let subdir = repo.path.join("src");
    std::fs::create_dir_all(&subdir).unwrap();

    let out = run_cli(
        &["skill", "install", "claude", "codex", "--repo"],
        &subdir,
        home_env(home.path()),
    )
    .await;

    assert_eq!(out.code, exit::OK, "{}", out.stderr);
    assert!(skill_at(&repo.path, ".claude/skills").is_file());
    assert!(skill_at(&repo.path, ".agents/skills").is_file());
    assert!(!home.path().join(".claude").exists());

    let outside = run_cli(
        &["skill", "install", "claude", "--repo"],
        home.path(),
        home_env(home.path()),
    )
    .await;
    assert_eq!(outside.code, exit::ERROR);
    assert!(
        outside.stderr.contains("not inside a git repository"),
        "{}",
        outside.stderr
    );
}

/// A cloned repo's `.claude/skills` that links into your home must not get your files
/// overwritten. (Unix: Windows needs admin rights to make links.)
#[cfg(unix)]
#[tokio::test]
async fn repo_refuses_to_write_through_a_link() {
    let home = TempDir::new().unwrap();
    let repo = TempRepo::new();
    let mine = home.path().join("dotfiles");
    std::fs::create_dir_all(mine.join("orkestr8r")).unwrap();
    std::fs::write(mine.join("orkestr8r/SKILL.md"), "my own file").unwrap();
    std::fs::create_dir_all(repo.path.join(".claude")).unwrap();
    std::os::unix::fs::symlink(&mine, repo.path.join(".claude/skills")).unwrap();

    let out = run_cli(
        &["skill", "install", "claude", "--repo"],
        &repo.path,
        home_env(home.path()),
    )
    .await;

    assert_eq!(out.code, exit::ERROR);
    assert!(out.stderr.contains("is a link"), "{}", out.stderr);
    assert_eq!(
        std::fs::read_to_string(mine.join("orkestr8r/SKILL.md")).unwrap(),
        "my own file"
    );

    // A link to a file, the SKILL.md itself.
    std::fs::remove_file(repo.path.join(".claude/skills")).unwrap();
    let skill_dir = repo.path.join(".claude/skills/orkestr8r");
    std::fs::create_dir_all(&skill_dir).unwrap();
    std::os::unix::fs::symlink(mine.join("orkestr8r/SKILL.md"), skill_dir.join("SKILL.md"))
        .unwrap();
    let out = run_cli(
        &["skill", "install", "claude", "--repo"],
        &repo.path,
        home_env(home.path()),
    )
    .await;
    assert_eq!(out.code, exit::ERROR);
    assert_eq!(
        std::fs::read_to_string(mine.join("orkestr8r/SKILL.md")).unwrap(),
        "my own file"
    );
}

#[tokio::test]
async fn list_shows_every_agent_found_and_installed() {
    let home = TempDir::new().unwrap();
    std::fs::create_dir_all(home.path().join(".gemini")).unwrap();
    let env = home_env(home.path());
    run_cli(&["skill", "install", "claude"], home.path(), env.clone()).await;

    let out = run_cli(&["skill", "list", "--json"], home.path(), env.clone()).await;

    assert_eq!(out.code, exit::OK, "{}", out.stderr);
    let agents = result(&out.stdout)["data"]["agents"].clone();
    let row = |id: &str| {
        agents
            .as_array()
            .unwrap()
            .iter()
            .find(|a| a["agent"] == id)
            .unwrap()
            .clone()
    };
    // The skill install created ~/.claude, so Claude Code counts as found now.
    assert_eq!(row("claude")["skill"], "current");
    assert_eq!(row("gemini")["found"], true);
    assert_eq!(row("gemini")["skill"], "missing");
    assert_eq!(row("opencode")["found"], false);

    let human = run_cli(&["skill", "list"], home.path(), env).await;
    assert!(human.stdout.contains("Gemini CLI"), "{}", human.stdout);
    assert!(human.stdout.contains("current"), "{}", human.stdout);
}

#[test]
fn every_agent_has_an_id_that_reads_back() {
    for agent in Agent::ALL {
        assert_eq!(Agent::from_id(agent.id()), Some(agent));
    }
    assert_eq!(Agent::from_id("Claude"), Some(Agent::Claude));
    assert_eq!(Agent::from_id("vim"), None);
}
