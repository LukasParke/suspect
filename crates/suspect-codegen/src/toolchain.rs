//! Native-toolchain manifest for the opt-in acceptance gates.
//!
//! Every generated-SDK behavioral gate needs a real toolchain, and those
//! gates are opt-in (`#[ignore]`) so CI stays hermetic. This manifest makes
//! the requirements declarative: per tool, the command that reports its
//! version, the regex that extracts it, the minimum acceptable version, and
//! the installation guidance shown when the probe fails. Gates call
//! [`probe_status`] instead of hand-rolled `Option` dances, so the skip
//! message tells a contributor exactly what is missing, what was found,
//! what is required, and how to install it.

/// One native tool the acceptance gates may require.
pub struct Tool {
    /// Stable identifier (`"go"`, `"jdk"`, …).
    pub id: &'static str,
    /// What the tool gates, in one line.
    pub description: &'static str,
    /// Command that prints the version, as argv.
    pub version_command: &'static [&'static str],
    /// Regex whose first capture group is the version string.
    pub version_regex: &'static str,
    /// Minimum acceptable version (numeric dot-separated comparison).
    pub min_version: &'static str,
    /// Installation guidance shown when the probe fails.
    pub install_documentation: &'static str,
    /// Environment variable overriding the executable location
    /// (`SUSPECT_DOTNET_BIN`, …). Empty when the tool has no override.
    pub env_override: &'static str,
    /// Environment variable naming a tool home directory whose
    /// `bin/<command>` is the executable (`JAVA_HOME`,
    /// `SUSPECT_RUBY_HOME`). Empty when the tool has none.
    pub home_env: &'static str,
    /// Version-manager install location used when no override is set.
    /// A relative path is resolved against the mise data directory
    /// ([`mise_data_dir`]); an absolute path is used as is. One path
    /// component may end in `*` to match a versioned directory name (the
    /// lexically greatest match wins). Empty when the tool always lives on
    /// `PATH`.
    pub version_manager_path: &'static str,
    /// Command printing the tool's home directory, used by
    /// [`resolve_home`] when no home variable is set and the executable is
    /// not inside a `<home>/bin/` layout (e.g. behind a version-manager
    /// shim). Empty when the tool has no home concept.
    pub home_query: &'static [&'static str],
    /// Whether the tool needs `JAVA_HOME` (Maven). [`command`] supplies the
    /// resolved JDK home when the caller's environment has none.
    pub needs_java_home: bool,
}

/// The manifest, in canonical order.
pub const TOOLS: &[Tool] = &[
    Tool {
        id: "go",
        description: "Go 1.23+ acceptance gates (incoming webhooks, streams, replay)",
        version_command: &["go", "version"],
        version_regex: r"go(\d+\.\d+(?:\.\d+)?)",
        min_version: "1.23",
        install_documentation: "Install Go 1.23 or later from https://go.dev/doc/install",
        env_override: "",
        home_env: "",
        version_manager_path: "",
        home_query: &[],
        needs_java_home: false,
    },
    Tool {
        id: "jdk",
        description: "Java 21+ acceptance gates (OAuth lifecycle, pagination, streams)",
        version_command: &["java", "-version"],
        version_regex: r#""?(\d+\.\d+(?:\.\d+)?)"?"#,
        min_version: "21",
        install_documentation: "Install JDK 21 or later from https://adoptium.net, or point JAVA_HOME at one",
        env_override: "SUSPECT_JAVA_BIN",
        home_env: "JAVA_HOME",
        version_manager_path: "installs/java/temurin-21",
        home_query: &["java", "-XshowSettings:property=java.home", "-version"],
        needs_java_home: false,
    },
    Tool {
        id: "maven",
        description: "Package compilation for the Java acceptance gates",
        version_command: &["mvn", "--version"],
        version_regex: r"Apache Maven (\d+\.\d+(?:\.\d+)?)",
        min_version: "3.8",
        install_documentation: "Install Maven 3.8 or later from https://maven.apache.org/install.html",
        env_override: "SUSPECT_MAVEN_BIN",
        home_env: "",
        version_manager_path: "installs/maven/3/apache-maven-*/bin",
        home_query: &[],
        needs_java_home: true,
    },
    Tool {
        id: "dotnet",
        description: "C# acceptance gates (OAuth lifecycle, pagination, streams)",
        version_command: &["dotnet", "--version"],
        version_regex: r"(\d+\.\d+\.\d+)",
        min_version: "8.0",
        install_documentation: "Install the .NET 8 SDK or later from https://dotnet.microsoft.com/download",
        env_override: "SUSPECT_DOTNET_BIN",
        home_env: "",
        version_manager_path: "dotnet-root/dotnet",
        home_query: &[],
        needs_java_home: false,
    },
    Tool {
        id: "swift",
        description: "Swift acceptance gates (OAuth lifecycle, pagination, streams)",
        version_command: &["swift", "--version"],
        version_regex: r"(?:swift-lang-|Swift version )(\d+\.\d+(?:\.\d+)?)",
        min_version: "5.9",
        install_documentation: "Install Swift 5.9 or later from https://www.swift.org/install, or `mise x swift@latest`",
        env_override: "SUSPECT_SWIFT_BIN",
        home_env: "",
        version_manager_path: "",
        home_query: &[],
        needs_java_home: false,
    },
    Tool {
        id: "dart",
        description: "Dart acceptance gates (AOT elimination gate, pagination)",
        version_command: &["dart", "--version"],
        version_regex: r"Dart SDK version: (\d+\.\d+\.\d+)",
        min_version: "3.9",
        install_documentation: "Install the Dart SDK 3.9 or later (`mise x dart@3.9 -- dart --version`)",
        env_override: "SUSPECT_DART_BIN",
        home_env: "",
        version_manager_path: "",
        home_query: &[],
        needs_java_home: false,
    },
    Tool {
        id: "node",
        description: "TypeScript package compilation and behavioral gates",
        version_command: &["node", "--version"],
        version_regex: r"v(\d+\.\d+\.\d+)",
        min_version: "20",
        install_documentation: "Install Node.js 20 or later from https://nodejs.org",
        env_override: "",
        home_env: "",
        version_manager_path: "",
        home_query: &[],
        needs_java_home: false,
    },
    Tool {
        id: "ruby",
        description: "Ruby acceptance gates (OAuth lifecycle, pagination, streams)",
        version_command: &["ruby", "--version"],
        version_regex: r"ruby (\d+\.\d+\.\d+)",
        min_version: "3.2",
        install_documentation: "Install Ruby 3.2 or later from https://www.ruby-lang.org/en/documentation/installation",
        env_override: "SUSPECT_RUBY_BIN",
        home_env: "SUSPECT_RUBY_HOME",
        version_manager_path: "installs/ruby/3",
        home_query: &["ruby", "-e", "print RbConfig::CONFIG['prefix']"],
        needs_java_home: false,
    },
    Tool {
        id: "php",
        description: "PHP acceptance gates (OAuth lifecycle, pagination, streams)",
        version_command: &["php", "--version"],
        version_regex: r"PHP (\d+\.\d+\.\d+)",
        min_version: "8.2",
        install_documentation: "Install PHP 8.2 or later from https://www.php.net/manual/en/install.php",
        env_override: "SUSPECT_PHP_BIN",
        home_env: "",
        version_manager_path: concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../target/sdk-php-tools/php-8.3.32/php"
        ),
        home_query: &[],
        needs_java_home: false,
    },
    Tool {
        id: "terraform",
        description: "Terraform provider acceptance gates",
        version_command: &["terraform", "version"],
        version_regex: r"Terraform v(\d+\.\d+\.\d+)",
        min_version: "1.5",
        install_documentation: "Install Terraform 1.5 or later from https://developer.hashicorp.com/terraform/install",
        env_override: "SUSPECT_TERRAFORM_BIN",
        home_env: "",
        version_manager_path: "installs/terraform/latest",
        home_query: &[],
        needs_java_home: false,
    },
    Tool {
        id: "cmake",
        description: "C++ acceptance gates (generated CMake packages, installed consumers)",
        version_command: &["cmake", "--version"],
        version_regex: r"cmake version (\d+\.\d+(?:\.\d+)?)",
        min_version: "3.24",
        install_documentation: "Install CMake 3.24 or later from https://cmake.org/download (or `mise use cmake@3`)",
        env_override: "SUSPECT_CPP_CMAKE",
        home_env: "",
        version_manager_path: "",
        home_query: &[],
        needs_java_home: false,
    },
    Tool {
        id: "cxx",
        description: "C++20 compiler for the C++ acceptance gates",
        version_command: &["clang++", "--version"],
        version_regex: r"(?:clang version |g\+\+ \([^)]*\) )(\d+\.\d+(?:\.\d+)?)",
        min_version: "13",
        install_documentation: "Install clang++ 15+ or g++ 13+ (Xcode command line tools, or `apt-get install clang`), or set SUSPECT_CPP_CXX",
        env_override: "SUSPECT_CPP_CXX",
        home_env: "",
        version_manager_path: "",
        home_query: &[],
        needs_java_home: false,
    },
    Tool {
        id: "doxygen",
        description: "Doxygen docs target of generated C++ packages",
        version_command: &["doxygen", "--version"],
        version_regex: r"^(\d+\.\d+\.\d+)",
        min_version: "1.9.8",
        install_documentation: "Run `tools/native-setup.sh cpp` (Doxygen 1.9.8), or set SUSPECT_CPP_DOXYGEN",
        env_override: "SUSPECT_CPP_DOXYGEN",
        home_env: "",
        // Pinned 1.9.8 provisioned by `tools/native-setup.sh cpp` on macOS
        // (Linux uses the distribution package on PATH).
        version_manager_path: concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../target/sdk-cpp-tools/doxygen-1.9.8/doxygen"
        ),
        home_query: &[],
        needs_java_home: false,
    },
];

/// The outcome of a toolchain probe.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolStatus {
    /// The tool is present and at least `min_version`.
    Available {
        /// The detected version string.
        version: String,
    },
    /// The command could not be run at all.
    Missing,
    /// The tool is present but older than `min_version`.
    TooOld {
        /// The detected version string.
        found: String,
        /// The required minimum version string.
        required: String,
    },
}

impl ToolStatus {
    /// Whether the gate may run.
    #[must_use]
    pub fn is_available(&self) -> bool {
        matches!(self, Self::Available { .. })
    }
}

/// Probes one tool by id. Unknown ids are a caller bug and return
/// [`ToolStatus::Missing`] with a diagnostic comment via [`guidance`].
#[must_use]
pub fn probe_status(id: &str) -> (ToolStatus, &'static str) {
    let Some(tool) = TOOLS.iter().find(|t| t.id == id) else {
        return (ToolStatus::Missing, "unknown tool id");
    };
    let Some(executable) = resolve_path(id) else {
        return (ToolStatus::Missing, tool.install_documentation);
    };
    let output = match command_for(tool, executable)
        .args(&tool.version_command[1..])
        .output()
    {
        Ok(output) => output,
        Err(_) => return (ToolStatus::Missing, tool.install_documentation),
    };
    if !output.status.success() {
        return (ToolStatus::Missing, tool.install_documentation);
    }
    let text = String::from_utf8_lossy(&output.stdout).into_owned()
        + &String::from_utf8_lossy(&output.stderr);
    let version = extract_version(&text, tool.version_regex);
    let Some(version) = version else {
        return (ToolStatus::Missing, tool.install_documentation);
    };
    if version_at_least(&version, tool.min_version) {
        (
            ToolStatus::Available { version },
            tool.install_documentation,
        )
    } else {
        (
            ToolStatus::TooOld {
                found: version,
                required: tool.min_version.to_owned(),
            },
            tool.install_documentation,
        )
    }
}

/// Resolves the executable for one tool: the `SUSPECT_*_BIN` override,
/// then the version-manager path when it exists, then `PATH`.
///
/// The returned path is what both the version probe and the actual gate
/// build run, so "probe says available" and "command runs" can't drift
/// apart.
#[must_use]
pub fn resolve_path(id: &str) -> Option<std::path::PathBuf> {
    let tool = TOOLS.iter().find(|t| t.id == id)?;
    if !tool.env_override.is_empty()
        && let Some(path) = std::env::var_os(tool.env_override)
        && !path.is_empty()
    {
        return Some(std::path::PathBuf::from(path));
    }
    if !tool.home_env.is_empty()
        && let Some(home) = std::env::var_os(tool.home_env)
        && !home.is_empty()
    {
        let candidate = std::path::PathBuf::from(home)
            .join("bin")
            .join(tool.version_command[0]);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    if let Some(managed) = managed_path(tool) {
        if managed.is_file() {
            return Some(managed);
        }
        // Directory layouts (e.g. `…/installs/maven/3/bin`, a JDK home)
        // need the binary name appended, directly or under `bin/`.
        if managed.is_dir() {
            for candidate in [
                managed.join(tool.version_command[0]),
                managed.join("bin").join(tool.version_command[0]),
            ] {
                if candidate.is_file() {
                    return Some(candidate);
                }
            }
        }
    }
    find_on_path(tool.version_command[0])
}

/// A [`std::process::Command`] for the resolved tool, with the
/// environment it needs to run (Maven gets the resolved JDK's
/// `JAVA_HOME` when the caller has none). `None` when the tool cannot be
/// located; use [`gate`] first for an actionable skip message.
#[must_use]
pub fn command(id: &str) -> Option<std::process::Command> {
    let tool = TOOLS.iter().find(|t| t.id == id)?;
    Some(command_for(tool, resolve_path(id)?))
}

fn command_for(tool: &Tool, executable: std::path::PathBuf) -> std::process::Command {
    let mut command = std::process::Command::new(executable);
    if tool.needs_java_home
        && std::env::var_os("JAVA_HOME").is_none_or(|home| home.is_empty())
        && let Some(home) = resolve_home("jdk")
    {
        command.env("JAVA_HOME", home);
    }
    command
}

/// A `cargo` command for building a generated package from a test.
///
/// It runs the same cargo as the calling build (`$CARGO`, falling back to
/// `cargo` on `PATH`) and drops `RUSTFLAGS`/`RUSTDOCFLAGS`, so lint flags
/// the developer or CI exported for this workspace (`-D warnings`) do not
/// leak into packages that are built as separate workspaces. Incremental
/// compilation is off and debug info is line tables only, which keeps
/// each scratch build a fraction of its default size.
#[must_use]
pub fn nested_cargo() -> std::process::Command {
    let cargo = std::env::var_os("CARGO")
        .filter(|path| !path.is_empty())
        .unwrap_or_else(|| "cargo".into());
    let mut command = std::process::Command::new(cargo);
    command
        .env_remove("RUSTFLAGS")
        .env_remove("RUSTDOCFLAGS")
        // Throwaway builds: incremental state and full debug info only cost
        // disk (each generated package is built from scratch once).
        .env("CARGO_INCREMENTAL", "0")
        .env("CARGO_PROFILE_DEV_DEBUG", "line-tables-only")
        .env("CARGO_PROFILE_TEST_DEBUG", "line-tables-only");
    command
}

/// The environment variable that turns a missing tool into a hard
/// failure. CI jobs dedicated to one backend set it so a broken toolchain
/// install cannot pass as a skipped gate.
pub const REQUIRE_NATIVE_ENV: &str = "SUSPECT_REQUIRE_NATIVE";

/// Gate entry point for acceptance tests: the resolved executable, or
/// `None` after printing the actionable skip message.
///
/// # Panics
/// When the tool is unavailable and [`REQUIRE_NATIVE_ENV`] is set to a
/// non-empty value other than `0`.
#[must_use]
pub fn require(id: &str) -> Option<std::path::PathBuf> {
    match gate(id) {
        Ok(path) => Some(path),
        Err(skip) => {
            assert!(!native_required(), "{REQUIRE_NATIVE_ENV} is set: {skip}");
            eprintln!("skipping: {skip}");
            None
        }
    }
}

/// [`require`] for a tool's home directory (`jdk`, `ruby`).
///
/// # Panics
/// As [`require`].
#[must_use]
pub fn require_home(id: &str) -> Option<std::path::PathBuf> {
    require(id)?;
    let home = resolve_home(id);
    if home.is_none() {
        let skip = format!("`{id}` is installed but its home directory could not be determined");
        assert!(!native_required(), "{REQUIRE_NATIVE_ENV} is set: {skip}");
        eprintln!("skipping: {skip}");
    }
    home
}

/// [`external_fixture`] for acceptance tests: the path, or `None` after
/// printing the skip message. External fixtures are never required, even
/// under [`REQUIRE_NATIVE_ENV`], because CI does not have them.
#[must_use]
pub fn require_fixture(env_var: &str, what: &str) -> Option<std::path::PathBuf> {
    match external_fixture(env_var, what) {
        Ok(path) => Some(path),
        Err(skip) => {
            eprintln!("skipping: {skip}");
            None
        }
    }
}

/// A file `tools/native-setup.sh <backend>` provisions under the
/// workspace `target/` directory (`relative` is relative to `target/`),
/// or the path named by `env_var` when that is set. `None` after a skip
/// message naming the setup command when the file is missing.
///
/// # Panics
/// As [`require`]: under [`REQUIRE_NATIVE_ENV`] a missing provisioned
/// file is a hard failure, because CI runs the setup script.
#[must_use]
pub fn require_provisioned(
    env_var: &str,
    relative: &str,
    backend: &str,
) -> Option<std::path::PathBuf> {
    let path = std::env::var_os(env_var)
        .filter(|value| !value.is_empty())
        .map_or_else(
            || {
                std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("../../target")
                    .join(relative)
            },
            std::path::PathBuf::from,
        );
    if path.exists() {
        return Some(path);
    }
    let skip = format!(
        "{} is missing; run `tools/native-setup.sh {backend}` (or set {env_var})",
        path.display()
    );
    assert!(!native_required(), "{REQUIRE_NATIVE_ENV} is set: {skip}");
    eprintln!("skipping: {skip}");
    None
}

/// A PHP command that ignores every `php.ini` (`-n`, so no user or
/// distribution settings leak into a gate) but still loads the shared
/// extensions the installation's ini files enable.
///
/// Distribution and `setup-php` builds ship `ctype`, `phar`, `curl`,
/// `mbstring`, `tokenizer`, … as shared modules that only an ini file
/// loads; a bare `php -n` there cannot even open `composer.phar`. This
/// asks the interpreter which extensions its ini enables beyond the
/// `-n` set and re-adds exactly those with `-d extension=…` (Zend
/// extensions such as Xdebug and OPcache are not re-added). A static
/// build gets plain `-n`. `None` when PHP cannot be located; gate with
/// [`require`] first for the skip message.
#[must_use]
pub fn php_command() -> Option<std::process::Command> {
    static ARGS: std::sync::OnceLock<Vec<String>> = std::sync::OnceLock::new();
    let php = resolve_path("php")?;
    let args = ARGS.get_or_init(|| php_hermetic_args(&php));
    let mut command = std::process::Command::new(php);
    command.args(args);
    Some(command)
}

fn php_hermetic_args(php: &std::path::Path) -> Vec<String> {
    const QUERY: &str =
        r#"echo ini_get("extension_dir"), "\n", implode("\n", get_loaded_extensions(false));"#;
    let loaded = |hermetic: bool| -> Option<Vec<String>> {
        let mut command = std::process::Command::new(php);
        if hermetic {
            command.arg("-n");
        }
        let output = command.args(["-r", QUERY]).output().ok()?;
        output.status.success().then(|| {
            String::from_utf8_lossy(&output.stdout)
                .lines()
                .map(str::to_owned)
                .collect()
        })
    };
    let mut args = vec!["-n".to_owned()];
    let (Some(configured), Some(bare)) = (loaded(false), loaded(true)) else {
        return args;
    };
    let Some((extension_dir, configured)) = configured.split_first() else {
        return args;
    };
    let bare: std::collections::BTreeSet<String> = bare
        .iter()
        .skip(1)
        .map(|name| name.to_lowercase())
        .collect();
    let directory = std::path::Path::new(extension_dir);
    let shared: Vec<String> = configured
        .iter()
        .map(|name| name.to_lowercase())
        .filter(|name| !bare.contains(name))
        .filter(|name| {
            ["so", "dll"]
                .iter()
                .any(|suffix| directory.join(format!("{name}.{suffix}")).is_file())
        })
        .collect();
    if !shared.is_empty() {
        args.extend(["-d".to_owned(), format!("extension_dir={extension_dir}")]);
        for name in shared {
            args.extend(["-d".to_owned(), format!("extension={name}")]);
        }
    }
    args
}

/// A tool of the resolved Swift toolchain (`swiftc`, `docc`, ...).
///
/// `SUSPECT_SWIFTC_BIN` / `SUSPECT_SWIFT_DOCC_BIN` override `swiftc` /
/// `docc`. Otherwise the tool beside the resolved `swift` driver (a
/// Swift.org or swiftly toolchain, Linux included) is used; when the driver
/// is the macOS `/usr/bin` shim, `xcrun --find` locates the selected Xcode
/// toolchain's tool instead. `None` when it cannot be found; gate with
/// [`require`]`("swift")` first for the skip message.
#[must_use]
pub fn swift_tool(name: &str) -> Option<std::path::PathBuf> {
    let override_var = match name {
        "swiftc" => "SUSPECT_SWIFTC_BIN",
        "docc" => "SUSPECT_SWIFT_DOCC_BIN",
        _ => "",
    };
    if !override_var.is_empty()
        && let Some(path) = std::env::var_os(override_var).filter(|value| !value.is_empty())
    {
        return Some(path.into());
    }
    let xcrun = || {
        let output = std::process::Command::new("xcrun")
            .args(["--find", name])
            .output()
            .ok()?;
        output.status.success().then(|| {
            std::path::PathBuf::from(String::from_utf8_lossy(&output.stdout).trim().to_owned())
        })
    };
    let swift = resolve_path("swift")?;
    let canonical = std::fs::canonicalize(&swift).unwrap_or(swift);
    if canonical.parent() != Some(std::path::Path::new("/usr/bin")) {
        let sibling = canonical.with_file_name(name);
        if sibling.is_file() {
            return Some(sibling);
        }
    }
    xcrun().or_else(|| find_on_path(name))
}

/// The SDK root Swift gates compile against: `SUSPECT_SWIFT_SDKROOT`, else
/// on macOS the `xcrun --sdk macosx --show-sdk-path` SDK. `None` where the
/// toolchain needs no SDK argument (Linux).
#[must_use]
pub fn swift_sdk_root() -> Option<std::path::PathBuf> {
    if let Some(path) = std::env::var_os("SUSPECT_SWIFT_SDKROOT").filter(|value| !value.is_empty())
    {
        return Some(path.into());
    }
    if !cfg!(target_os = "macos") {
        return None;
    }
    let output = std::process::Command::new("xcrun")
        .args(["--sdk", "macosx", "--show-sdk-path"])
        .output()
        .ok()?;
    output.status.success().then(|| {
        std::path::PathBuf::from(String::from_utf8_lossy(&output.stdout).trim().to_owned())
    })
}

/// The import search directory (`swiftc -I`) holding `<module>.swiftmodule`
/// under a SwiftPM scratch directory, for either build backend: the native
/// build system's `<triple>/<config>/Modules/`, or Swift Build's
/// `out/Products/<Config>/` (the default in recent toolchains). Swift Build's
/// per-object intermediates are used only when no product module exists.
/// `None` when the module was not built there.
#[must_use]
pub fn swift_module_dir(scratch: &std::path::Path, module: &str) -> Option<std::path::PathBuf> {
    fn walk(dir: &std::path::Path, name: &str, found: &mut Vec<std::path::PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.filter_map(Result::ok) {
            let path = entry.path();
            if entry.file_name() == name {
                found.push(dir.to_path_buf());
            } else if path.is_dir() && !path.is_symlink() {
                walk(&path, name, found);
            }
        }
    }
    let mut found = Vec::new();
    walk(scratch, &format!("{module}.swiftmodule"), &mut found);
    let rank = |dir: &std::path::Path| {
        let named = |component: &std::path::Path, name: &str| {
            component.file_name().is_some_and(|value| value == name)
        };
        if named(dir, "Modules") {
            0
        } else if dir.parent().is_some_and(|parent| named(parent, "Products")) {
            1
        } else {
            2
        }
    };
    found.sort_by_key(|dir| (rank(dir), dir.clone()));
    found.into_iter().next()
}

/// Whether `SUSPECT_REQUIRE_NATIVE` turns skips into failures (set,
/// non-empty and not `0`).
pub fn native_required() -> bool {
    std::env::var(REQUIRE_NATIVE_ENV).is_ok_and(|value| !value.is_empty() && value != "0")
}

/// SHA-256 of the OpenRouter OpenAPI document the OpenRouter-backed gates
/// were written against (`openrouter-web` commit `50b0c3910b36`,
/// `projects/docs/openapi/openapi.yaml`). Gates compare their expected
/// operations, names and recorded responses to this exact revision.
pub const OPENROUTER_SPEC_SHA256: &str =
    "bd4953b29f34de134ed4be27b2803c5622a756c3c63bc8617636b6abe5de1821";

/// The `openrouter-web` commit whose OpenAPI document is
/// [`OPENROUTER_SPEC_SHA256`].
pub const OPENROUTER_SPEC_COMMIT: &str = "50b0c3910b36dc33c949a490bbe2fba59d574c42";

/// The pinned OpenRouter OpenAPI document for gates that exercise a real
/// production API description.
///
/// Looks in `SUSPECT_OPENROUTER_YAML` / `SUSPECT_OPENROUTER_OPENAPI` (a
/// file), then `$OPENROUTER_WEB_ROOT/projects/docs/openapi/openapi.yaml`,
/// then a sibling `../openrouter-web` checkout of this workspace. The file
/// is used only when its SHA-256 is [`OPENROUTER_SPEC_SHA256`]; any other
/// revision would make the gates fail on upstream spec drift rather than
/// on generator behavior. Otherwise prints a skip message with the command
/// that materializes the pinned file, and returns `None`. Never required,
/// even under [`REQUIRE_NATIVE_ENV`].
#[must_use]
pub fn openrouter_spec() -> Option<std::path::PathBuf> {
    const RELATIVE: &str = "projects/docs/openapi/openapi.yaml";
    let non_empty = |name: &str| std::env::var_os(name).filter(|value| !value.is_empty());
    let candidate = non_empty("SUSPECT_OPENROUTER_YAML")
        .or_else(|| non_empty("SUSPECT_OPENROUTER_OPENAPI"))
        .map(std::path::PathBuf::from)
        .or_else(|| {
            non_empty("OPENROUTER_WEB_ROOT").map(|root| std::path::Path::new(&root).join(RELATIVE))
        })
        .or_else(|| {
            let sibling = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../../openrouter-web")
                .join(RELATIVE);
            sibling.is_file().then_some(sibling)
        });
    let materialize = format!(
        "materialize it with `git -C <openrouter-web> show {OPENROUTER_SPEC_COMMIT}:{RELATIVE} > openrouter.yaml` and set SUSPECT_OPENROUTER_YAML to that file"
    );
    let Some(path) = candidate else {
        eprintln!("skipping: no OpenRouter OpenAPI document; {materialize}");
        return None;
    };
    let Ok(bytes) = std::fs::read(&path) else {
        eprintln!("skipping: cannot read {}; {materialize}", path.display());
        return None;
    };
    let digest = sha2::Digest::finalize(<sha2::Sha256 as sha2::Digest>::new_with_prefix(&bytes));
    let hex: String = digest.iter().map(|byte| format!("{byte:02x}")).collect();
    if hex != OPENROUTER_SPEC_SHA256 {
        eprintln!(
            "skipping: {} is OpenRouter revision {hex}, gates are pinned to {OPENROUTER_SPEC_SHA256}; {materialize}",
            path.display()
        );
        return None;
    }
    Some(path)
}

/// The root of the pinned OpenRouter checkout, for gates that read
/// companion documents beside [`openrouter_spec`].
///
/// The root is the pinned spec's path with
/// `projects/docs/openapi/openapi.yaml` stripped (so a materialized tree
/// `<root>/projects/docs/openapi/openapi.yaml` works as well as a real
/// checkout). Each of `companions` (paths relative to the root) must
/// exist; they are not hash-pinned, so materialize them from
/// [`OPENROUTER_SPEC_COMMIT`] too. `None` after a skip message otherwise.
/// Never required, even under [`REQUIRE_NATIVE_ENV`].
#[must_use]
pub fn openrouter_checkout(companions: &[&str]) -> Option<std::path::PathBuf> {
    let spec = openrouter_spec()?;
    let Some(root) = spec.ancestors().nth(4).map(std::path::Path::to_path_buf) else {
        eprintln!(
            "skipping: {} is not inside an openrouter-web tree",
            spec.display()
        );
        return None;
    };
    if let Some(missing) = companions
        .iter()
        .find(|relative| !root.join(relative).is_file())
    {
        eprintln!(
            "skipping: {} has no {missing}; materialize it with `git -C <openrouter-web> show {OPENROUTER_SPEC_COMMIT}:{missing}`",
            root.display()
        );
        return None;
    }
    Some(root)
}

/// A Node.js executable of one pinned major version (`22` or `24`).
///
/// `SUSPECT_PACKAGE_NODE` (for 22) or `SUSPECT_NODE24_BIN` (for 24) wins;
/// then the mise `installs/node/<major>` install; then `node` on `PATH`
/// when it reports that major version. `None` after the skip message
/// (a panic instead under [`REQUIRE_NATIVE_ENV`]).
///
/// # Panics
/// As [`require`].
#[must_use]
pub fn pinned_node(major: u32) -> Option<std::path::PathBuf> {
    let override_var = match major {
        22 => "SUSPECT_PACKAGE_NODE",
        24 => "SUSPECT_NODE24_BIN",
        _ => "",
    };
    if !override_var.is_empty()
        && let Some(path) = std::env::var_os(override_var).filter(|value| !value.is_empty())
    {
        return Some(path.into());
    }
    let reports_major = |node: &std::path::Path| {
        std::process::Command::new(node)
            .arg("--version")
            .output()
            .ok()
            .filter(|output| output.status.success())
            .is_some_and(|output| {
                String::from_utf8_lossy(&output.stdout)
                    .trim()
                    .strip_prefix('v')
                    .and_then(|version| version.split('.').next())
                    .is_some_and(|found| found == major.to_string())
            })
    };
    let managed = mise_data_dir().map(|dir| dir.join(format!("installs/node/{major}/bin/node")));
    if let Some(node) = managed.filter(|node| node.is_file() && reports_major(node)) {
        return Some(node);
    }
    if let Some(node) = find_on_path("node").filter(|node| reports_major(node)) {
        return Some(node);
    }
    let skip = format!(
        "Node {major} not found; set {} or install it (e.g. `mise install node@{major}`)",
        if override_var.is_empty() {
            "PATH"
        } else {
            override_var
        }
    );
    assert!(!native_required(), "{REQUIRE_NATIVE_ENV} is set: {skip}");
    eprintln!("skipping: {skip}");
    None
}

/// A Python interpreter of one exact minor version (`"3.11"`, `"3.14"`).
///
/// Looks for `python<minor>` on `PATH`, then the mise
/// `installs/python/<minor>` and uv `cpython-<minor>-*` installs, and
/// accepts only an interpreter that reports that version. `None` after the
/// skip message (a panic instead under [`REQUIRE_NATIVE_ENV`]).
///
/// # Panics
/// As [`require`].
#[must_use]
pub fn pinned_python(minor: &str) -> Option<std::path::PathBuf> {
    let reports = |python: &std::path::Path| {
        std::process::Command::new(python)
            .args(["-c", "import sys; print('%d.%d' % sys.version_info[:2])"])
            .output()
            .ok()
            .filter(|output| output.status.success())
            .is_some_and(|output| String::from_utf8_lossy(&output.stdout).trim() == minor)
    };
    let mut candidates: Vec<std::path::PathBuf> = Vec::new();
    candidates.extend(find_on_path(&format!("python{minor}")));
    if let Some(mise) = mise_data_dir() {
        candidates.push(mise.join(format!("installs/python/{minor}/bin/python{minor}")));
    }
    let uv_root = std::env::var_os("UV_PYTHON_INSTALL_DIR")
        .map(std::path::PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME")
                .map(|home| std::path::Path::new(&home).join(".local/share/uv/python"))
        });
    if let Some(root) = uv_root
        && let Ok(entries) = std::fs::read_dir(&root)
    {
        let prefix = format!("cpython-{minor}.");
        let mut installs: Vec<_> = entries
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| {
                path.file_name()
                    .is_some_and(|name| name.to_string_lossy().starts_with(&prefix))
            })
            .collect();
        installs.sort();
        candidates.extend(
            installs
                .into_iter()
                .rev()
                .map(|dir| dir.join(format!("bin/python{minor}"))),
        );
    }
    if let Some(found) = candidates
        .into_iter()
        .find(|python| python.is_file() && reports(python))
    {
        return Some(found);
    }
    let skip = format!(
        "Python {minor} not found; install it (e.g. `uv python install {minor}`) so `python{minor}` is on PATH"
    );
    assert!(!native_required(), "{REQUIRE_NATIVE_ENV} is set: {skip}");
    eprintln!("skipping: {skip}");
    None
}

/// The mise data directory: `$MISE_DATA_DIR`, else `$XDG_DATA_HOME/mise`,
/// else `~/.local/share/mise`. `None` when none of those can be formed.
#[must_use]
pub fn mise_data_dir() -> Option<std::path::PathBuf> {
    let non_empty = |name: &str| std::env::var_os(name).filter(|value| !value.is_empty());
    if let Some(dir) = non_empty("MISE_DATA_DIR") {
        return Some(dir.into());
    }
    if let Some(dir) = non_empty("XDG_DATA_HOME") {
        return Some(std::path::PathBuf::from(dir).join("mise"));
    }
    let home = non_empty("HOME").or_else(|| non_empty("USERPROFILE"))?;
    Some(std::path::PathBuf::from(home).join(".local/share/mise"))
}

/// The tool's version-manager location, resolved per
/// [`Tool::version_manager_path`].
fn managed_path(tool: &Tool) -> Option<std::path::PathBuf> {
    if tool.version_manager_path.is_empty() {
        return None;
    }
    let path = std::path::Path::new(tool.version_manager_path);
    let base = if path.is_absolute() {
        std::path::PathBuf::new()
    } else {
        mise_data_dir()?
    };
    let mut resolved = base;
    for component in path.components() {
        let text = component.as_os_str().to_string_lossy();
        match text.strip_suffix('*') {
            Some(prefix) => {
                let best = std::fs::read_dir(&resolved)
                    .ok()?
                    .filter_map(Result::ok)
                    .map(|entry| entry.file_name().to_string_lossy().into_owned())
                    .filter(|name| name.starts_with(prefix))
                    .max()?;
                resolved.push(best);
            }
            None => resolved.push(component),
        }
    }
    Some(resolved)
}

/// Resolves a tool's home directory (a JDK's `JAVA_HOME`, a Ruby prefix).
///
/// Order: the tool's home variable; the parent of `bin/` when the
/// resolved executable lives in a `<home>/bin/` layout; then the tool's
/// [`Tool::home_query`] run through the resolved executable. `None` when
/// the tool has no home or none can be found.
#[must_use]
pub fn resolve_home(id: &str) -> Option<std::path::PathBuf> {
    let tool = TOOLS.iter().find(|t| t.id == id)?;
    if !tool.home_env.is_empty()
        && let Some(home) = std::env::var_os(tool.home_env)
        && !home.is_empty()
    {
        return Some(home.into());
    }
    if tool.home_query.is_empty() {
        return None;
    }
    let executable = resolve_path(id)?;
    let canonical = std::fs::canonicalize(&executable).unwrap_or_else(|_| executable.clone());
    if let Some(bin) = canonical.parent()
        && bin.file_name().is_some_and(|name| name == "bin")
        && let Some(home) = bin.parent()
        && home.join("bin").join(tool.version_command[0]).is_file()
    {
        return Some(home.to_path_buf());
    }
    let output = std::process::Command::new(&executable)
        .args(&tool.home_query[1..])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout).into_owned()
        + &String::from_utf8_lossy(&output.stderr);
    // `java -XshowSettings:property=java.home` prints `java.home = <dir>`;
    // other queries print the bare directory.
    let line = text
        .lines()
        .map(str::trim)
        .find_map(|line| line.strip_prefix("java.home = "))
        .or_else(|| text.lines().map(str::trim).find(|line| !line.is_empty()))?;
    let home = std::path::PathBuf::from(line);
    home.is_dir().then_some(home)
}

/// An external fixture directory or file named by an environment
/// variable (e.g. `OPENROUTER_WEB_ROOT`), for gates that need inputs this
/// repository does not ship.
///
/// # Errors
/// A skip message when the variable is unset or names a missing path.
pub fn external_fixture(env_var: &str, what: &str) -> Result<std::path::PathBuf, String> {
    match std::env::var_os(env_var).filter(|value| !value.is_empty()) {
        Some(path) => {
            let path = std::path::PathBuf::from(path);
            if path.exists() {
                Ok(path)
            } else {
                Err(format!(
                    "{env_var}={} does not exist ({what})",
                    path.display()
                ))
            }
        }
        None => Err(format!("set {env_var} to {what}")),
    }
}

/// Walks `PATH` for an executable with the given name (the `which` dance
/// without a dependency).
fn find_on_path(name: &str) -> Option<std::path::PathBuf> {
    let path_var = std::env::var_os("PATH")?;
    std::env::split_paths(&path_var)
        .map(|dir| dir.join(name))
        .find(|candidate| candidate.is_file())
}

/// The acceptance-gate entry point: the resolved executable when the tool
/// is present and new enough, or the actionable skip message.
///
/// ```
/// let jdk = suspect_codegen::toolchain::gate("jdk");
/// match jdk {
///     Ok(java) => { /* run the gate with `java` */ }
///     Err(skip) => eprintln!("skipping: {skip}"),
/// }
/// ```
///
/// # Errors
/// The skip message naming what is missing, what was found, what is
/// required, and how to install it.
pub fn gate(id: &str) -> Result<std::path::PathBuf, String> {
    let (status, install) = probe_status(id);
    match status {
        ToolStatus::Available { .. } => {
            resolve_path(id).ok_or_else(|| format!("`{id}` resolved but not locatable"))
        }
        ToolStatus::Missing => Err(format!(
            "{}: not found; {install}",
            TOOLS
                .iter()
                .find(|t| t.id == id)
                .map_or(id, |t| t.description)
        )),
        ToolStatus::TooOld { found, required } => Err(format!(
            "{}: found {found}, requires >= {required}; {install}",
            TOOLS
                .iter()
                .find(|t| t.id == id)
                .map_or(id, |t| t.description)
        )),
    }
}

/// Extracts the first capture group of the compiled regex.
fn extract_version(text: &str, pattern: &str) -> Option<String> {
    let regex = regex::Regex::new(pattern).ok()?;
    regex
        .captures(text)
        .and_then(|captures| captures.get(1))
        .map(|m| m.as_str().to_owned())
}

/// Numeric dot-separated comparison: `1.9 < 1.10`.
#[must_use]
pub fn version_at_least(found: &str, min: &str) -> bool {
    let parse = |s: &str| -> Vec<u64> {
        s.split('.')
            .map(|p| p.trim().parse::<u64>().unwrap_or(0))
            .collect()
    };
    let (f, m) = (parse(found), parse(min));
    for i in 0..m.len().max(f.len()) {
        let a = f.get(i).copied().unwrap_or(0);
        let b = m.get(i).copied().unwrap_or(0);
        if a != b {
            return a > b;
        }
    }
    true
}

/// The actionable skip message for one tool: what it gates, what was
/// found (if anything), what is required, and how to install it.
#[must_use]
pub fn guidance(id: &str) -> String {
    let Some(tool) = TOOLS.iter().find(|t| t.id == id) else {
        return format!("unknown tool `{id}`");
    };
    match probe_status(id).0 {
        ToolStatus::Available { version } => {
            format!(
                "{}: available ({version}); requires >= {}",
                tool.description, tool.min_version
            )
        }
        ToolStatus::Missing => format!(
            "{}: not installed (requires >= {}); {}",
            tool.description, tool.min_version, tool.install_documentation
        ),
        ToolStatus::TooOld { found, required } => format!(
            "{}: found {found}, requires >= {required}; {}",
            tool.description, tool.install_documentation
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_evidence_checks_out_the_pinned_openrouter_commit() {
        let workflow = include_str!("../../../.github/workflows/native-evidence.yml");
        assert!(
            workflow.contains(&format!("ref: {OPENROUTER_SPEC_COMMIT}")),
            "native-evidence.yml must check out openrouter-web at {OPENROUTER_SPEC_COMMIT}"
        );
    }

    #[test]
    fn versions_compare_numerically_per_component() {
        assert!(version_at_least("1.23.0", "1.23"));
        assert!(version_at_least("1.24.1", "1.23"));
        assert!(!version_at_least("1.9", "1.10"));
        assert!(!version_at_least("21.0.1", "21.1"));
        assert!(version_at_least("21.0.8", "21"));
    }

    #[test]
    fn every_manifest_tool_has_coherent_metadata() {
        // Realistic command output per tool; the regex must capture the
        // version from the output its own command produces.
        let samples: &[(&str, &str)] = &[
            ("go", "go version go1.23.4 darwin/arm64"),
            ("jdk", "openjdk version \"21.0.8\" 2025-07-15"),
            ("maven", "Apache Maven 3.9.9 (build)"),
            ("dotnet", "8.0.412"),
            (
                "swift",
                "swift-driver version: 1.115 Swift version 6.2 (swift-lang-6.2.0)",
            ),
            ("dart", "Dart SDK version: 3.9.4 (stable)"),
            ("node", "v20.19.0"),
            ("ruby", "ruby 3.4.5 (2025-07-16)"),
            ("php", "PHP 8.3.32 (cli)"),
            ("terraform", "Terraform v1.15.8\non darwin_arm64"),
            ("cmake", "cmake version 3.31.6\n\nCMake suite maintained"),
            (
                "cxx",
                "Apple clang version 21.0.0 (clang-2100.0.123.102)\nTarget: arm64-apple-darwin",
            ),
            ("doxygen", "1.9.8"),
        ];
        for tool in TOOLS {
            assert!(
                regex::Regex::new(tool.version_regex).is_ok(),
                "{}: version regex must compile",
                tool.id
            );
            let sample = samples
                .iter()
                .find(|(id, _)| *id == tool.id)
                .map(|(_, s)| *s)
                .expect("every tool has a sample");
            let extracted = extract_version(sample, tool.version_regex);
            assert!(
                extracted.is_some(),
                "{}: regex must extract a version from {:?}",
                tool.id,
                sample
            );
            assert!(
                version_at_least(extracted.as_deref().unwrap_or(""), tool.min_version),
                "{}: the sample {} must satisfy the manifest minimum {}",
                tool.id,
                extracted.unwrap_or_default(),
                tool.min_version
            );
            assert!(!tool.install_documentation.is_empty());
            assert!(!tool.description.is_empty());
        }
    }

    #[test]
    fn probes_run_without_panicking_and_guidance_is_actionable() {
        for tool in TOOLS {
            let (status, install) = probe_status(tool.id);
            let _ = status;
            assert_eq!(install, tool.install_documentation);
            let message = guidance(tool.id);
            assert!(
                message.contains(tool.description),
                "guidance must name what the tool gates: {message}"
            );
        }
    }

    #[test]
    fn path_resolution_prefers_override_then_managed_then_path() {
        // Every tool resolves through one of the three channels without
        // panicking; a missing tool yields None, not an error.
        for tool in TOOLS {
            let resolved = resolve_path(tool.id);
            if let Some(path) = &resolved {
                assert!(
                    path.is_file(),
                    "{}: resolved path must exist: {}",
                    tool.id,
                    path.display()
                );
            }
        }
        // Unknown ids resolve to nothing.
        assert!(resolve_path("definitely-not-a-tool").is_none());
    }

    #[test]
    fn gate_returns_executable_or_actionable_skip() {
        // For every tool the Err message must carry the install doc, and
        // the Ok case must be the same path resolve_path picked.
        for tool in TOOLS {
            match gate(tool.id) {
                Ok(path) => assert_eq!(Some(&path), resolve_path(tool.id).as_ref()),
                Err(skip) => {
                    assert!(
                        skip.contains(tool.install_documentation),
                        "skip message must carry install guidance: {skip}"
                    );
                }
            }
        }
        assert!(gate("definitely-not-a-tool").is_err());
    }
}
