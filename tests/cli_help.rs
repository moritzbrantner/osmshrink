use std::process::Command;

fn run_help(args: &[&str]) -> String {
    let output = Command::new(env!("CARGO_BIN_EXE_osmshrink"))
        .args(args)
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "`osmshrink {}` failed\nstdout:\n{}\nstderr:\n{}",
        args.join(" "),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    String::from_utf8(output.stdout).unwrap()
}

#[test]
fn root_help_lists_commands_and_examples() {
    let help = run_help(&["--help"]);

    assert!(help.contains("Usage: osmshrink [OPTIONS] <COMMAND>"));
    assert!(help.contains("Commands:"));
    assert!(help.contains("fetch"));
    assert!(help.contains("filter"));
    assert!(help.contains("run"));
    assert!(help.contains("validate-spec"));
    assert!(help.contains("Examples:"));
    assert!(help.contains("Use `osmshrink <COMMAND> --help`"));
}

#[test]
fn subcommand_help_is_available() {
    let commands = [
        (
            "fetch",
            "Usage: osmshrink fetch [OPTIONS] --output <PBF> <SOURCE>",
        ),
        (
            "filter",
            "Usage: osmshrink filter [OPTIONS] --input <PBF> --output <FILE> [FILTER]",
        ),
        (
            "run",
            "Usage: osmshrink run [OPTIONS] --source <SOURCE> --output <FILE> [FILTER]",
        ),
        (
            "inspect",
            "Usage: osmshrink inspect [OPTIONS] --input <PBF>",
        ),
        (
            "validate-spec",
            "Usage: osmshrink validate-spec [OPTIONS] [FILTER]",
        ),
    ];

    for (command, usage) in commands {
        let help = run_help(&[command, "--help"]);
        assert!(
            help.contains(usage),
            "{command} help missing expected usage"
        );
        assert!(
            help.contains("-h, --help"),
            "{command} help missing help flag"
        );
    }
}

#[test]
fn run_help_uses_distinct_source_and_spec_short_flags() {
    let help = run_help(&["run", "--help"]);

    assert!(help.contains("-s, --source <SOURCE>"));
    assert!(help.contains("-f, --spec <FILE>"));
    assert!(help.contains("[aliases: --filter-file]"));
}
