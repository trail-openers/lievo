use std::path::PathBuf;
use std::process::Command;

pub fn run_lievo(args: &[&str]) -> (String, String, i32) {
    let bin_path = if PathBuf::from("./target/debug/lievo").exists() {
        "./target/debug/lievo".to_string()
    } else {
        "cargo".to_string()
    };

    let result = if bin_path == "cargo" {
        Command::new("cargo")
            .args(["run", "--bin", "lievo", "--"])
            .args(args)
            .output()
            .expect("failed to run lievo")
    } else {
        Command::new(&bin_path)
            .args(args)
            .output()
            .expect("failed to run lievo")
    };

    let stdout = String::from_utf8_lossy(&result.stdout).to_string();
    let stderr = String::from_utf8_lossy(&result.stderr).to_string();
    let status = result.status.code().unwrap_or(1);

    (stdout, stderr, status)
}
