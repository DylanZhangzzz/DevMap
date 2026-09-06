use std::process::Command;

fn main() {
    let revision = Command::new("git")
        .args(["describe", "--always", "--dirty", "--abbrev=8"])
        .output()
        .ok()
        .filter(|result| result.status.success())
        .and_then(|result| String::from_utf8(result.stdout).ok())
        .map(|value| value.trim().to_owned())
        .filter(|value| {
            value
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || ".-_".contains(c))
        })
        .unwrap_or_else(|| "unavailable".to_owned());
    println!("cargo:rustc-env=DEVMAP_BUILD_REVISION={revision}");
}
