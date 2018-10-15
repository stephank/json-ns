use ::{Context, Processor, TargetContext};
use colored::Colorize;
use json::{self, Value};
use std::ffi::OsStr;
use std::fs::{File, read_dir};
use std::io::Read;
use std::path::PathBuf;

// Parse a target context specification.
fn parse_target(input: &str) -> Result<TargetContext, ()> {
    let mut target = TargetContext::default();

    let input = input.trim();
    if input == "-" {
        return Ok(target);
    }

    for line in input.lines() {
        let mut parts = line.splitn(2, ": ");
        let prefix = parts.next().unwrap();
        let suffix = parts.next().ok_or(())?;
        target.add_rule(prefix, suffix);
    }

    Ok(target)
}

// Run a directory of `*.txt` test files.
fn run_dir(dir: &str) {
    let mut entries: Vec<PathBuf> = read_dir(dir)
        .expect("could not read tests dir")
        .map(|res| res.expect("could not iterate test dir"))
        .map(|entry| entry.path())
        .filter(|path| path.extension() == Some(OsStr::new("txt")))
        .collect();
    entries.sort_unstable();

    let mut num_passed = 0;
    for path in &entries {
        let mut processor = Processor::new();
        let mut data = String::new();
        File::open(path)
            .expect("could not open test")
            .read_to_string(&mut data)
            .expect("could not read test");

        let mut parts = data.split("\n\n");
        let name = parts.next()
            .expect("test has no name");
        let context = parts.next()
            .expect("test has no context");
        let target = parts.next()
            .expect("test has no target");
        let input = parts.next()
            .expect("test has no input");
        let expect = parts.next()
            .expect("test has no expectation");

        let stem = path.file_stem().and_then(|s| s.to_str())
            .expect("test has invalid filename");
        let name = format!("{} [{}]", name.trim(), stem.yellow());
        processor.context = json::from_str(context)
            .map(|value: Value| Context::from(&value))
            .expect("test has invalid input");
        processor.target = parse_target(target)
            .expect("test has invalid target");
        let input: Value = json::from_str(input)
            .expect("test has invalid input");
        let expect: Value = json::from_str(expect)
            .expect("test has invalid expectation");

        let output = processor.process_value(&input);
        if output == expect {
            num_passed += 1;
            eprintln!(" {} {}", "✔ PASS:".green(), name);
        } else {
            eprintln!(" {} {}", "✖ FAIL:".red(), name);
            eprintln!("{}", json::to_string_pretty(&output)
                .expect("cannot serialize output value"));
        }
    }

    eprintln!("\n ▓ {} out of {} passed\n", num_passed, entries.len());
    assert!(num_passed == entries.len(), "all tests must pass");
}

#[test]
fn test() {
    run_dir("tests");
}
