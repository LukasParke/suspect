//! Native test tooling shared by the Swift v2 runtime and public-codec gates.
use std::{
    path::{Path, PathBuf},
    process::{Command, Output},
};

pub fn root(label: &str) -> PathBuf {
    let base = std::env::var_os("SUSPECT_SWIFT_V2_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("opencode/swift-v2-gates"));
    std::fs::create_dir_all(&base).unwrap();
    tempfile::Builder::new()
        .prefix(label)
        .tempdir_in(base)
        .unwrap()
        .keep()
        .canonicalize()
        .unwrap()
}
fn find_tool(name: &str) -> PathBuf {
    let output = Command::new("xcrun")
        .args(["--find", name])
        .output()
        .expect("Xcode native tool is required");
    assert!(output.status.success());
    PathBuf::from(String::from_utf8(output.stdout).unwrap().trim())
}
pub fn sdk() -> PathBuf {
    std::env::var_os("SUSPECT_SWIFT_SDKROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            let output = Command::new("xcrun")
                .args(["--sdk", "macosx", "--show-sdk-path"])
                .output()
                .unwrap();
            assert!(output.status.success());
            PathBuf::from(String::from_utf8(output.stdout).unwrap().trim())
        })
}
pub fn compiler_path() -> PathBuf {
    std::env::var_os("SUSPECT_SWIFTC_BIN")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            std::env::var_os("SUSPECT_SWIFT_BIN")
                .map(PathBuf::from)
                .map(|p| p.with_file_name("swiftc"))
                .unwrap_or_else(|| find_tool("swiftc"))
        })
}
pub fn swift(action: &str) -> Command {
    let binary = std::env::var_os("SUSPECT_SWIFT_BIN")
        .map(PathBuf::from)
        .unwrap_or_else(|| find_tool("swift"));
    let mut command = Command::new(binary);
    command
        .arg(action)
        .arg("--sdk")
        .arg(sdk())
        .env("SDKROOT", sdk())
        .env("SWIFT_EXEC", compiler_path());
    if action == "test" {
        command.arg("--disable-swift-testing");
    }
    command
}
pub fn compiler() -> Command {
    let mut command = Command::new(compiler_path());
    command
        .args(["-swift-version", "6", "-sdk"])
        .arg(sdk())
        .env("SDKROOT", sdk());
    command
}
pub fn checked(command: &mut Command, root: &Path) -> Output {
    let output = command.output().expect("required native tool unavailable");
    let label = root.join(format!(
        "command-{}.log",
        std::fs::read_dir(root).unwrap().count()
    ));
    std::fs::write(
        &label,
        format!(
            "{command:?}\nstatus: {}\n{}{}",
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        ),
    )
    .unwrap();
    assert!(
        output.status.success(),
        "native artifacts: {}\nlog: {}\n{}{}",
        root.display(),
        label.display(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    println!("{}", String::from_utf8_lossy(&output.stdout));
    output
}
pub fn directory(root: &Path, name: &str) -> Option<PathBuf> {
    for entry in std::fs::read_dir(root).ok()? {
        let path = entry.ok()?.path();
        if path.is_dir() {
            if path.file_name()?.to_str()? == name {
                return Some(path);
            }
            if let Some(found) = directory(&path, name) {
                return Some(found);
            }
        }
    }
    None
}
pub fn docs(root: &Path, module: &str) {
    checked(
        swift("package")
            .arg("--package-path")
            .arg(root.join("sdk"))
            .arg("--scratch-path")
            .arg(root.join("build/sdk"))
            .args(["dump-symbol-graph", "--minimum-access-level", "public"]),
        root,
    );
    let mut command = Command::new(
        std::env::var_os("SUSPECT_SWIFT_DOCC_BIN")
            .map(PathBuf::from)
            .unwrap_or_else(|| find_tool("docc")),
    );
    checked(
        command
            .arg("convert")
            .arg(root.join(format!("sdk/Sources/{module}/{module}.docc")))
            .arg("--additional-symbol-graph-dir")
            .arg(directory(&root.join("build/sdk"), "symbolgraph").unwrap())
            .arg("--output-path")
            .arg(root.join(format!("{module}.doccarchive")))
            .arg("--warnings-as-errors"),
        root,
    );
    assert!(
        root.join(format!("{module}.doccarchive/index.html"))
            .is_file()
    );
}
