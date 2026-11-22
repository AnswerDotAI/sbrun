use sbrun::{CliCommand, ConfigMode, RunTarget, admin, config, dedup_validate_env_names, host, parse_cli, pathutil::{self, AllowedWrites}, prompt};
use std::{ffi::{OsStr, OsString}, fs, path::{Path, PathBuf}};

fn args(xs: &[&str]) -> Vec<OsString> { xs.iter().map(OsString::from).collect() }

#[test]
fn parses_cli_commands_and_rejects_invalid_flags() {
    let CliCommand::Run { target, options } = parse_cli(args(&["sbrun", "python3", "-c", "print(1)"])).unwrap() else { panic!() };
    assert!(matches!(options.config, ConfigMode::Default));
    let RunTarget::Exec(argv) = target else { panic!() };
    assert_eq!(argv, args(&["python3", "-c", "print(1)"]));

    let CliCommand::Run { target: RunTarget::ShellCommand(cmd), .. } = parse_cli(args(&["sbrun", "-c", "echo hi"])).unwrap() else { panic!() };
    assert_eq!(cmd, "echo hi");

    let CliCommand::Run { options, .. } = parse_cli(args(&["sbrun", "-w", "/tmp", "--unset-env", "FOO", "--", "echo", "hi"])).unwrap() else { panic!() };
    assert_eq!(options.write, vec![PathBuf::from("/tmp")]);
    assert_eq!(options.unset_env, vec!["FOO".to_string()]);

    let CliCommand::Run { options, .. } = parse_cli(args(&["sbrun", "--no-config", "echo", "hi"])).unwrap() else { panic!() };
    assert!(matches!(options.config, ConfigMode::None));

    let CliCommand::Run { target: RunTarget::Exec(argv), .. } = parse_cli(args(&["sbrun", "--", "--bogus"])).unwrap() else { panic!() };
    assert_eq!(argv, args(&["--bogus"]));

    assert!(matches!(parse_cli(args(&["sbrun", "--help"])).unwrap(), CliCommand::Help));
    assert!(matches!(parse_cli(args(&["sbrun", "--version"])).unwrap(), CliCommand::Version));
    assert!(matches!(parse_cli(args(&["sbrun", "--kernel-install"])).unwrap(), CliCommand::KernelInstall));
    assert!(matches!(parse_cli(args(&["sbrun", "--prompt-init"])).unwrap(), CliCommand::PromptInit(None)));
    assert!(matches!(parse_cli(args(&["sbrun", "--prompt-init=zsh"])).unwrap(), CliCommand::PromptInit(Some(ref s)) if s == "zsh"));

    for (argv, needle) in [
        (&["sbrun", "--kernel-install", "--no-config"][..], "--kernel-install cannot be combined"),
        (&["sbrun", "--prompt-init", "--no-config"][..], "--prompt-init cannot be combined"), (&["sbrun", "--bogus"][..], "unknown option --bogus"),
        (&["sbrun", "-wc", "/tmp"][..], "unknown option"), (&["sbrun", "--no-config=x"][..], "does not take a value"),
    ] {
        let err = parse_cli(args(argv)).unwrap_err();
        assert!(err.to_string().contains(needle), "{err}");
    }
}

#[test]
fn validates_and_deduplicates_env_names() {
    let (mut ed, mut ue) = (vec!["FOO".into(), "_BAR".into(), "FOO".into()], vec!["A123_B".into(), "A123_B".into()]);
    dedup_validate_env_names(&mut ed, &mut ue).unwrap();
    assert_eq!(ed, vec!["FOO", "_BAR"]);
    assert_eq!(ue, vec!["A123_B"]);

    for bad in ["", "1BAD", "NO-DASH", "NO.DOT"] { assert!(dedup_validate_env_names(&mut vec![bad.into()], &mut vec![]).is_err(), "{bad}"); }
    for reserved in ["PATH", "HOME", "SBRUN_ACTIVE"] { assert!(dedup_validate_env_names(&mut vec![], &mut vec![reserved.into()]).is_err(), "{reserved}"); }
    assert!(dedup_validate_env_names(&mut vec!["FOO".into()], &mut vec!["FOO".into()]).is_err());
}

#[test]
fn loads_and_validates_config_files() {
    let empty = config::load(&ConfigMode::None, Some(Path::new("/home/test"))).unwrap();
    assert!(empty.required.is_empty() && empty.optional.is_empty());
    assert!(config::load(&ConfigMode::Explicit(PathBuf::from("/nonexistent/config.toml")), None).is_err());

    let dir = std::env::temp_dir().join(format!("sbrun-cfg-test-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let cfg_path = dir.join("config.toml");

    fs::write(&cfg_path, "version = 1\nwrite = [\"/tmp\", \"~/rel\"]\noptional_write = [\"/nonexistent\"]\n").unwrap();
    let cfg = config::load(&ConfigMode::Explicit(cfg_path.clone()), None).unwrap();
    assert_eq!(cfg.required, vec![PathBuf::from("/tmp"), PathBuf::from("~/rel")]);
    assert_eq!(cfg.optional, vec![PathBuf::from("/nonexistent")]);

    fs::write(&cfg_path, "version = 99\n").unwrap();
    assert!(config::load(&ConfigMode::Explicit(cfg_path.clone()), None).is_err());

    fs::write(&cfg_path, "version = 1\nwrite = [\"relative/path\"]\n").unwrap();
    assert!(config::load(&ConfigMode::Explicit(cfg_path), None).is_err());

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn expands_home_and_checks_allowed_paths() {
    let home = Path::new("/home/test");
    assert_eq!(pathutil::expand_home(Path::new("~/foo"), Some(home)).unwrap(), PathBuf::from("/home/test/foo"));
    assert_eq!(pathutil::expand_home(Path::new("~"), Some(home)).unwrap(), PathBuf::from("/home/test"));
    assert_eq!(pathutil::expand_home(Path::new("/tmp/foo"), Some(home)).unwrap(), PathBuf::from("/tmp/foo"));
    assert!(pathutil::expand_home(Path::new("~/foo"), None).is_err());
    assert!(pathutil::expand_home(Path::new("~other/foo"), Some(home)).is_err());

    let allowed = AllowedWrites { dirs: vec![PathBuf::from("/extra")], files: vec![PathBuf::from("/etc/hosts")] };
    assert!(pathutil::path_is_allowed(Path::new("/work/sub/file"), Path::new("/work"), &allowed));
    assert!(pathutil::path_is_allowed(Path::new("/extra/sub"), Path::new("/work"), &allowed));
    assert!(pathutil::path_is_allowed(Path::new("/etc/hosts"), Path::new("/work"), &allowed));
    assert!(!pathutil::path_is_allowed(Path::new("/etc/passwd"), Path::new("/work"), &allowed));
}

#[test]
fn prompt_init_and_host_helpers() {
    let bash = prompt::init_script(Some("bash")).unwrap();
    assert!(bash.contains("PROMPT_COMMAND") && bash.contains("\u{1F512} "));
    let zsh = prompt::init_script(Some("/bin/zsh")).unwrap();
    assert!(zsh.contains("precmd_functions") && zsh.contains("\u{1F512} "));
    assert!(prompt::init_script(Some("fish")).unwrap_err().to_string().contains("only supports bash and zsh"));

    assert_eq!(host::history_file_name(Path::new("/bin/bash")), ".bash_history");
    assert_eq!(host::history_file_name(Path::new("/bin/zsh")), ".zsh_history");
    assert_eq!(host::history_file_name(Path::new("/bin/fish")), ".sh_history");
    assert!(host::shell_is_bash(Path::new("/bin/bash")) && !host::shell_is_bash(Path::new("/bin/zsh")));
    assert_eq!(host::login_arg0(Path::new("/bin/bash")), OsStr::new("-bash"));
    assert_eq!(host::login_arg0(Path::new("/usr/bin/zsh")), OsStr::new("-zsh"));
    assert!(host::current().unwrap().shell.is_absolute());
}

#[test]
fn kernel_install_platform_and_privilege_guard() {
    #[cfg(not(target_os = "linux"))]
    assert!(admin::kernel_install().unwrap_err().to_string().contains("only available on Linux"));

    #[cfg(target_os = "linux")]
    if unsafe { libc::getuid() } != 0 || unsafe { libc::geteuid() } != 0 {
        assert!(admin::kernel_install().unwrap_err().to_string().contains("--kernel-install requires running as root"));
    }
}

#[cfg(target_os = "macos")]
#[test]
fn seatbelt_profile_escapes_and_validates_paths() {
    use std::os::unix::ffi::OsStrExt;
    let bad = PathBuf::from(OsStr::from_bytes(b"/tmp/\xff"));
    assert!(sbrun::profile::build(Path::new("/work"), &[bad], &[], None).is_err());
    assert!(sbrun::profile::build(Path::new("/work"), &[], &[PathBuf::from("/tmp/a\nb")], None).is_err());

    let profile = sbrun::profile::build(Path::new("/work"), &[PathBuf::from(r#"/a"b\c"#)], &[], None).unwrap();
    assert!(profile.contains(r#"(subpath "/a\"b\\c")"#), "{profile}");
}

#[cfg(target_os = "linux")]
#[test]
fn linux_sandbox_tmp_overlap_and_privilege_modes() {
    use sbrun::sandbox::{PrivilegeMode, mode_from_ids, needs_preflight_drop, tmp_overlaps_writes};
    let no: Vec<PathBuf> = vec![];
    assert!(tmp_overlaps_writes(Path::new("/work"), &[PathBuf::from("/tmp")], &no));
    assert!(tmp_overlaps_writes(Path::new("/work"), &[PathBuf::from("/tmp/foo")], &no));
    assert!(tmp_overlaps_writes(Path::new("/tmp/proj"), &no, &no));
    assert!(tmp_overlaps_writes(Path::new("/work"), &no, &[PathBuf::from("/tmp/f")]));
    assert!(!tmp_overlaps_writes(Path::new("/work"), &[PathBuf::from("/data")], &no));

    assert_eq!(mode_from_ids((1000, 1000)), PrivilegeMode::Unprivileged);
    assert!(!needs_preflight_drop((1000, 1000)));
    assert_eq!(mode_from_ids((1000, 0)), PrivilegeMode::Privileged);
    assert!(needs_preflight_drop((1000, 0)));
    assert_eq!(mode_from_ids((0, 0)), PrivilegeMode::Privileged);
    assert!(!needs_preflight_drop((0, 0)));
}
