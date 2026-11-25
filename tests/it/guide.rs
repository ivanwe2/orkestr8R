//! Keeps `ork guide`, the skill and docs/cli.md in step with the commands: a command that isn't
//! documented, or documented under another name, fails here.

use std::path::PathBuf;

use orkestr8r::cli::{self, commands::guide::GUIDE, exit};
use orkestr8r::env::Env;

use crate::support::cli::run_cli;

fn repo_file(rel: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(rel);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn command_names() -> Vec<String> {
    cli::command()
        .get_subcommands()
        .map(|c| c.get_name().to_string())
        .collect()
}

#[test]
fn every_command_is_in_the_guide_and_in_docs_cli_md() {
    let cli_md = repo_file("docs/cli.md");
    let names = command_names();
    assert!(names.len() >= 12, "{names:?}");

    for name in &names {
        let mention = format!("ork {name}");
        assert!(
            GUIDE.contains(&mention),
            "the guide doesn't mention `{mention}`"
        );
        assert!(
            cli_md.contains(&mention),
            "docs/cli.md doesn't mention `{mention}`"
        );
    }
}

#[test]
fn the_v0_1_commands_are_all_there() {
    let names = command_names();
    for name in [
        "init", "doctor", "models", "ask", "run", "jobs", "status", "diff", "logs", "apply",
        "discard", "guide",
    ] {
        assert!(names.iter().any(|n| n == name), "no `ork {name}` command");
    }
}

#[test]
fn the_skill_points_to_ork_guide() {
    let skill = repo_file("skills/orkestr8r/SKILL.md");

    assert!(
        skill.starts_with("---\nname: orkestr8r\ndescription: "),
        "{skill}"
    );
    assert!(skill.contains("delegate") || skill.contains("Delegate"));
    assert!(skill.contains("ork guide"));
}

#[tokio::test]
async fn ork_guide_prints_the_guide() {
    let out = run_cli(&["guide"], &std::env::temp_dir(), Env::default()).await;

    assert_eq!(out.code, exit::OK);
    assert_eq!(out.stdout, GUIDE);
    assert!(out.stdout.starts_with("# ork: the guide for AI agents"));
}
