use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use orkestr8r::cli::exit;
use orkestr8r::config::{self, DefaultsConfig, OrkConfig};
use orkestr8r::env::Env;
use serde_json::{Value, json};
use tempfile::TempDir;

use crate::support::cli::run_cli;

/// A temp dir with a repo in `repo/` (just a `.git` dir unless `git init` is needed) and the user
/// config directory in `config/`. The env points the user file there on every OS.
struct Setup {
    dir: TempDir,
    env: Env,
}

impl Setup {
    fn new() -> Self {
        let dir = TempDir::new().unwrap();
        fs::create_dir_all(dir.path().join("repo/.git")).unwrap();
        let config_home = dir.path().join("config").display().to_string();
        // the real environment (PATH finds git), minus ORK_CONFIG, with the user file in config/
        let process = Env::from_process();
        let env = Env::from_pairs(process.iter().filter(|(name, _)| *name != "ORK_CONFIG"))
            .with("APPDATA", &config_home)
            .with("XDG_CONFIG_HOME", &config_home);
        Self { dir, env }
    }

    /// The same, with a real repository (for `ork init`, which updates `.git/info/exclude`).
    fn with_git() -> Self {
        let setup = Self::new();
        fs::remove_dir_all(setup.repo().join(".git")).unwrap();
        let status = Command::new("git")
            .args(["init", "-q"])
            .current_dir(setup.repo())
            .status()
            .unwrap();
        assert!(status.success());
        setup
    }

    fn repo(&self) -> PathBuf {
        self.dir.path().join("repo")
    }

    fn user_file(&self) -> PathBuf {
        config::user_path(&self.env).unwrap()
    }

    fn repo_file(&self) -> PathBuf {
        self.repo().join("ork.json")
    }
}

fn write_json(path: &Path, value: Value) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, serde_json::to_string_pretty(&value).unwrap()).unwrap();
}

fn grunt_config(model: &str) -> Value {
    json!({
        "servers": { "box": { "url": "http://box:9876/v1" } },
        "roles": { "grunt": { "server": "box", "model": model } }
    })
}

#[test]
fn later_files_win_and_defaults_fill_the_rest() {
    let setup = Setup::new();
    let mut user = grunt_config("user-model");
    user["defaults"] = json!({ "max_turns": 10 });
    user["allow_commands"] = json!(["cargo build", "cargo test"]);
    write_json(&setup.user_file(), user);
    write_json(
        &setup.repo_file(),
        json!({ "roles": { "grunt": { "model": "repo-model" } }, "allow_commands": ["npm test"] }),
    );

    // from a subdirectory: the repo root is found by walking up
    let cwd = setup.repo().join("src/deep");
    fs::create_dir_all(&cwd).unwrap();
    let loaded = config::load(&cwd, &setup.env).unwrap();

    let grunt = &loaded.config.roles["grunt"];
    assert_eq!(grunt.model, "repo-model");
    assert_eq!(grunt.server, "box", "kept from the user file");
    assert_eq!(loaded.config.defaults.max_turns, 10);
    assert_eq!(loaded.config.defaults.max_repairs, 2, "built-in default");
    assert_eq!(
        loaded.config.allow_commands,
        ["npm test"],
        "arrays are replaced, not merged"
    );
    assert_eq!(loaded.sources, [setup.user_file(), setup.repo_file()]);
}

#[test]
fn ork_config_replaces_both_files() {
    let setup = Setup::new();
    write_json(&setup.user_file(), grunt_config("user-model"));
    write_json(&setup.repo_file(), grunt_config("repo-model"));
    let other = setup.dir.path().join("other.json");
    write_json(&other, grunt_config("other-model"));

    let env = setup
        .env
        .clone()
        .with("ORK_CONFIG", other.display().to_string());
    let loaded = config::load(&setup.repo(), &env).unwrap();

    assert_eq!(loaded.config.roles["grunt"].model, "other-model");
    assert_eq!(loaded.sources, [other]);
}

#[test]
fn a_missing_ork_config_file_is_an_error() {
    let setup = Setup::new();
    let env = setup.env.clone().with("ORK_CONFIG", "nope.json");

    let err = config::load(&setup.repo(), &env).unwrap_err();

    assert!(err.to_string().contains("ORK_CONFIG"), "{err}");
}

#[test]
fn bad_json_names_the_file() {
    let setup = Setup::new();
    fs::write(setup.repo_file(), "{ not json").unwrap();

    let err = config::load(&setup.repo(), &setup.env).unwrap_err();

    assert!(err.to_string().contains("ork.json"), "{err}");
}

#[test]
fn a_missing_grunt_is_named_by_its_path() {
    let problems = OrkConfig::default().validate();

    assert!(
        problems.iter().any(|p| p.starts_with("roles.grunt:")),
        "{problems:?}"
    );
}

#[test]
fn every_problem_comes_with_its_path() {
    let config: OrkConfig = serde_json::from_value(json!({
        "servers": { "box": { "url": "box:9876" } },
        "roles": {
            "grunt": { "server": "elsewhere", "model": "" },
        },
        "defaults": { "max_turns": 0, "max_repairs": 0 }
    }))
    .unwrap();

    let problems = config.validate();

    assert_eq!(
        problems,
        [
            "roles.grunt.server: \"elsewhere\" is not in servers",
            "roles.grunt.model: missing",
            "servers.box.url: \"box:9876\" is not an http:// or https:// URL",
            "defaults.max_turns: must be more than 0",
        ],
        "max_repairs may be 0"
    );
}

#[test]
fn defaults_survive_a_file_with_only_servers_and_roles() {
    let setup = Setup::new();
    write_json(&setup.repo_file(), grunt_config("qwen"));

    let loaded = config::load(&setup.repo(), &setup.env).unwrap();

    assert_eq!(loaded.config.defaults, DefaultsConfig::default());
    assert_eq!(loaded.config.roles["grunt"].context_tokens, 32_768);
    assert_eq!(loaded.config.roles["grunt"].temperature, 0.2);
    assert!(
        loaded.config.validate().is_empty(),
        "{:?}",
        loaded.config.validate()
    );
}

#[test]
fn the_api_key_comes_from_the_environment_only() {
    let server = config::ServerConfig {
        url: "http://box/v1".into(),
        key_env: Some("LLAMA_API_KEY".into()),
    };

    assert_eq!(config::api_key(&server, &Env::default()), None);
    let env = Env::default().with("LLAMA_API_KEY", "secret");
    assert_eq!(config::api_key(&server, &env).as_deref(), Some("secret"));
}

fn keyed_server(url: &str, key_env: &str) -> Value {
    json!({ "servers": { "box": { "url": url, "key_env": key_env } } })
}

#[test]
fn the_user_files_key_is_kept() {
    let setup = Setup::new();
    write_json(
        &setup.user_file(),
        keyed_server("http://box:9876/v1", "LLAMA_API_KEY"),
    );
    // the repo file may still set other things, even the same url
    write_json(
        &setup.repo_file(),
        json!({ "servers": { "box": { "url": "http://box:9876/v1" } }, "defaults": { "max_turns": 5 } }),
    );

    let loaded = config::load(&setup.repo(), &setup.env).unwrap();

    let key_env = loaded.config.servers["box"].key_env.as_deref();
    assert_eq!(key_env, Some("LLAMA_API_KEY"));
    assert!(loaded.warnings.is_empty(), "{:?}", loaded.warnings);
}

#[test]
fn the_repo_file_cannot_send_the_key_elsewhere() {
    let setup = Setup::new();
    write_json(
        &setup.user_file(),
        keyed_server("http://box:9876/v1", "LLAMA_API_KEY"),
    );
    write_json(
        &setup.repo_file(),
        json!({ "servers": { "box": { "url": "https://attacker.example/v1" } } }),
    );

    let loaded = config::load(&setup.repo(), &setup.env).unwrap();

    assert_eq!(loaded.config.servers["box"].key_env, None);
    assert_eq!(loaded.warnings.len(), 1);
    assert!(loaded.warnings[0].starts_with("servers.box.key_env: ignored"));
}

#[test]
fn the_repo_file_cannot_choose_which_variable_is_the_key() {
    let setup = Setup::new();
    write_json(
        &setup.repo_file(),
        keyed_server("https://attacker.example/v1", "AWS_SECRET_ACCESS_KEY"),
    );

    let loaded = config::load(&setup.repo(), &setup.env).unwrap();

    assert_eq!(loaded.config.servers["box"].key_env, None);
    assert_eq!(loaded.warnings.len(), 1);
}

#[test]
fn ork_config_is_trusted_like_the_user_file() {
    let setup = Setup::new();
    let own = setup.dir.path().join("own.json");
    write_json(&own, keyed_server("http://box:9876/v1", "LLAMA_API_KEY"));

    let env = setup
        .env
        .clone()
        .with("ORK_CONFIG", own.display().to_string());
    let loaded = config::load(&setup.repo(), &env).unwrap();

    let key_env = loaded.config.servers["box"].key_env.as_deref();
    assert_eq!(key_env, Some("LLAMA_API_KEY"));
}

#[tokio::test]
async fn init_key_env_needs_user() {
    let setup = Setup::with_git();

    let out = run_cli(
        &[
            "init",
            "--url",
            "http://box/v1",
            "--model",
            "qwen",
            "--key-env",
            "LLAMA_API_KEY",
        ],
        &setup.repo(),
        setup.env.clone(),
    )
    .await;

    assert_eq!(out.code, exit::ERROR);
    assert!(
        out.stderr.contains("--key-env needs --user"),
        "{}",
        out.stderr
    );
    assert!(!setup.repo_file().exists());
}

fn exclude_lines(repo: &Path) -> usize {
    let text = fs::read_to_string(repo.join(".git/info/exclude")).unwrap();
    text.lines().filter(|line| line.trim() == ".ork/").count()
}

#[tokio::test]
async fn init_writes_the_repo_file_and_the_exclude_line_once() {
    let setup = Setup::with_git();
    fs::write(setup.repo().join("Cargo.toml"), "[package]\n").unwrap();
    let args = [
        "init",
        "--url",
        "http://box:9876/v1",
        "--model",
        "qwen",
        "--json",
    ];

    let out = run_cli(&args, &setup.repo(), setup.env.clone()).await;

    assert_eq!(out.code, exit::OK, "{}{}", out.stdout, out.stderr);
    let result: Value = serde_json::from_str(out.stdout.lines().last().unwrap()).unwrap();
    assert_eq!(
        result["data"]["path"],
        setup.repo_file().display().to_string()
    );
    let written: Value =
        serde_json::from_str(&fs::read_to_string(setup.repo_file()).unwrap()).unwrap();
    assert_eq!(
        written,
        json!({
            "servers": { "box": { "url": "http://box:9876/v1" } },
            "roles": { "grunt": { "server": "box", "model": "qwen" } },
            "allow_commands": ["cargo build", "cargo test", "cargo clippy", "cargo fmt"]
        })
    );
    assert_eq!(exclude_lines(&setup.repo()), 1);

    // again, without --force: refused, nothing changes
    let again = run_cli(&args, &setup.repo(), setup.env.clone()).await;
    assert_eq!(again.code, exit::ERROR);
    assert!(again.stdout.contains("exists"), "{}", again.stdout);

    // with --force: rewritten, and the exclude line is still there once
    let mut forced = args.to_vec();
    forced.push("--force");
    let forced = run_cli(&forced, &setup.repo(), setup.env.clone()).await;
    assert_eq!(forced.code, exit::OK, "{}", forced.stdout);
    assert_eq!(exclude_lines(&setup.repo()), 1);
}

#[tokio::test]
async fn init_warns_when_no_build_files_are_found() {
    let setup = Setup::with_git();

    let out = run_cli(
        &[
            "init",
            "--url",
            "http://box:9876/v1",
            "--model",
            "qwen",
            "--json",
        ],
        &setup.repo(),
        setup.env.clone(),
    )
    .await;

    assert_eq!(out.code, exit::OK, "{}", out.stdout);
    let first: Value = serde_json::from_str(out.stdout.lines().next().unwrap()).unwrap();
    assert_eq!(first["event"], "warning");
    assert_eq!(first["code"], "no_allow_commands");
}

#[tokio::test]
async fn init_user_writes_the_user_file() {
    let setup = Setup::new();

    let out = run_cli(
        &[
            "init",
            "--user",
            "--url",
            "http://box:9876/v1",
            "--model",
            "qwen",
            "--key-env",
            "LLAMA_API_KEY",
        ],
        &setup.repo(),
        setup.env.clone(),
    )
    .await;

    assert_eq!(out.code, exit::OK, "{}", out.stderr);
    let written: Value =
        serde_json::from_str(&fs::read_to_string(setup.user_file()).unwrap()).unwrap();
    assert_eq!(written["servers"]["box"]["key_env"], "LLAMA_API_KEY");
    assert!(
        written.get("allow_commands").is_none(),
        "commands belong to a repo, not the user file: {written}"
    );
    assert!(!setup.repo_file().exists());
    // and the key the user file names is the one that's used
    let loaded = config::load(&setup.repo(), &setup.env).unwrap();
    assert_eq!(
        loaded.config.servers["box"].key_env.as_deref(),
        Some("LLAMA_API_KEY")
    );
    assert_eq!(
        out.stdout,
        format!("wrote {}\n", setup.user_file().display())
    );
}

#[tokio::test]
async fn init_refuses_a_bad_url() {
    let setup = Setup::with_git();

    let out = run_cli(
        &["init", "--url", "box:9876", "--model", "qwen"],
        &setup.repo(),
        setup.env.clone(),
    )
    .await;

    assert_eq!(out.code, exit::ERROR);
    assert!(out.stderr.contains("servers.box.url"), "{}", out.stderr);
    assert!(!setup.repo_file().exists());
}

/// The kc-commerce case: a Node repo with no ork.json of its own, and a user file that lists
/// another repo's commands.
fn node_repo(setup: &Setup, lockfile: &str) {
    let scripts = json!({
        "dev": "next dev",
        "build": "next build",
        "test": "pnpm run test:int && pnpm run test:e2e",
        "test:int": "vitest run",
        "lint": "eslint .",
        "lint:fix": "eslint . --fix",
        "typecheck": "tsc --noEmit",
        "test:$(boom)": "echo no"
    });
    write_json(
        &setup.repo().join("package.json"),
        json!({ "name": "shop", "scripts": scripts }),
    );
    fs::write(setup.repo().join(lockfile), "").unwrap();
}

#[test]
fn a_repo_without_its_own_list_gets_the_commands_its_build_files_imply() {
    let setup = Setup::new();
    let mut user = grunt_config("qwen");
    user["allow_commands"] = json!(["cargo test"]);
    write_json(&setup.user_file(), user);
    node_repo(&setup, "package-lock.json");

    let loaded = config::load(&setup.repo(), &setup.env).unwrap();

    assert_eq!(
        loaded.detected,
        [
            "npm test",
            "npm run lint",
            "npm run lint:fix",
            "npm run test",
            "npm run test:int",
            "npm run typecheck"
        ],
        "the checks among the scripts; not dev, build, or an odd name"
    );
    assert_eq!(
        loaded.config.allow_commands[0], "cargo test",
        "the user's stay"
    );
    assert_eq!(loaded.config.allow_commands.len(), 7);
}

#[test]
fn the_lockfile_names_the_package_manager() {
    let setup = Setup::new();
    node_repo(&setup, "pnpm-lock.yaml");
    let loaded = config::load(&setup.repo(), &setup.env).unwrap();
    assert!(
        loaded.detected.contains(&"pnpm run test:int".to_string()),
        "{:?}",
        loaded.detected
    );
    assert!(loaded.detected.iter().all(|c| c.starts_with("pnpm ")));

    let python = Setup::new();
    fs::write(python.repo().join("pyproject.toml"), "[project]\n").unwrap();
    let loaded = config::load(&python.repo(), &python.env).unwrap();
    assert_eq!(loaded.detected, ["pytest", "python -m pytest"]);
}

#[test]
fn a_repo_that_lists_its_commands_gets_exactly_those() {
    let setup = Setup::new();
    node_repo(&setup, "package-lock.json");
    write_json(
        &setup.repo_file(),
        json!({ "allow_commands": ["npm run test:int"] }),
    );

    let loaded = config::load(&setup.repo(), &setup.env).unwrap();

    assert!(loaded.detected.is_empty());
    assert_eq!(loaded.config.allow_commands, ["npm run test:int"]);

    // ORK_CONFIG is exact too.
    let exact = setup.dir.path().join("exact.json");
    write_json(&exact, json!({ "allow_commands": ["git status"] }));
    let env = setup
        .env
        .clone()
        .with("ORK_CONFIG", exact.display().to_string());
    let loaded = config::load(&setup.repo(), &env).unwrap();
    assert!(loaded.detected.is_empty());
    assert_eq!(loaded.config.allow_commands, ["git status"]);
}

#[tokio::test]
async fn doctor_says_which_commands_came_from_the_repo() {
    let setup = Setup::new();
    // cargo is on PATH wherever these tests run (npm may not be).
    fs::write(
        setup.repo().join("Cargo.toml"),
        "[package]
",
    )
    .unwrap();

    let out = run_cli(&["doctor", "--json"], &setup.repo(), setup.env.clone()).await;

    let result: Value = serde_json::from_str(out.stdout.lines().last().unwrap()).unwrap();
    let commands = result["data"]["checks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|check| check["name"] == "commands")
        .unwrap()
        .clone();
    let said = format!("{} {}", commands["detail"], commands["hint"]);
    assert!(said.contains("cargo test"), "{said}");
    assert!(said.contains("from this repo's build files"), "{said}");
}
