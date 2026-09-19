//! Pinned Go package metadata, including the actual generated SDK's module sum.
//! The SDK h1 algorithm is Go's sumdb/dirhash.Hash1; ZIP timestamps are irrelevant.
use std::collections::BTreeMap;

use sha2::{Digest, Sha256};

use super::*;

pub(super) fn artifacts(plan: &ProviderPlan) -> [OutFile; 2] {
    let config = &plan.config;
    let dependencies = BTreeMap::from([
        (config.sdk.module_path.as_str(), config.sdk.version.as_str()),
        (
            "github.com/hashicorp/terraform-plugin-framework",
            config.framework_version.as_str(),
        ),
    ]);
    let mut module = format!(
        "module {}\n\ngo {}\n\ntoolchain {}\n\nrequire (\n",
        config.module_path, config.go_version, config.go_toolchain
    );
    for (name, version) in dependencies {
        module.push_str(&format!("\t{name} v{version}\n"));
    }
    module.push_str(")\n\n");
    module.push_str(include_str!("dependencies.mod"));
    let prefix = format!("{}@v{}/", config.sdk.module_path, config.sdk.version);
    let files: BTreeMap<_, _> = plan
        .sdk_files
        .iter()
        .map(|f| {
            (
                format!(
                    "{prefix}{}",
                    f.path.strip_prefix("go/").expect("canonical Go artifact")
                ),
                f.content.as_bytes(),
            )
        })
        .collect();
    let sum = hash(&files);
    let go_mod = plan
        .sdk_files
        .iter()
        .find(|f| f.path == "go/go.mod")
        .expect("canonical SDK module");
    let mod_sum = hash(&BTreeMap::from([(
        "go.mod".to_owned(),
        go_mod.content.as_bytes(),
    )]));
    let mut sums = include_str!("dependencies.sum")
        .lines()
        .map(str::to_owned)
        .collect::<Vec<_>>();
    // The retained Go lock sorts versions semantically within each module;
    // insert this distinct module without re-sorting those version rows as text.
    let index = sums
        .iter()
        .position(|line| {
            line.split_whitespace()
                .next()
                .is_some_and(|name| name > config.sdk.module_path.as_str())
        })
        .unwrap_or(sums.len());
    sums.splice(
        index..index,
        [
            format!("{} v{} {sum}", config.sdk.module_path, config.sdk.version),
            format!(
                "{} v{}/go.mod {mod_sum}",
                config.sdk.module_path, config.sdk.version
            ),
        ],
    );
    [
        OutFile {
            path: "terraform/go.mod".into(),
            content: module,
        },
        OutFile {
            path: "terraform/go.sum".into(),
            content: format!("{}\n", sums.join("\n")),
        },
    ]
}

fn hash(files: &BTreeMap<String, &[u8]>) -> String {
    let mut summary = Sha256::new();
    for (name, content) in files {
        summary.update(format!("{:x}  {name}\n", Sha256::digest(content)).as_bytes());
    }
    let bytes = summary.finalize();
    // SHA-256 has a fixed 32-byte digest; only base64 presentation is implemented
    // locally. Digest computation stays with the existing audited sha2 dependency.
    const DIGITS: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut base64 = String::from("h1:");
    for block in bytes.chunks(3) {
        let bits = (u32::from(block[0]) << 16)
            | (u32::from(*block.get(1).unwrap_or(&0)) << 8)
            | u32::from(*block.get(2).unwrap_or(&0));
        for shift in [18, 12, 6, 0] {
            if shift == 0 && block.len() < 3 || shift == 6 && block.len() < 2 {
                base64.push('=');
            } else {
                base64.push(char::from(DIGITS[((bits >> shift) & 63) as usize]));
            }
        }
    }
    base64
}

pub(super) fn reserved_module(path: &str) -> bool {
    std::iter::once("github.com/hashicorp/terraform-plugin-framework")
        .chain(
            include_str!("dependencies.mod")
                .lines()
                .filter_map(|line| line.split_whitespace().next())
                .filter(|name| name.contains('.')),
        )
        .any(|name| {
            path == name
                || path
                    .strip_prefix(name)
                    .is_some_and(|suffix| suffix.starts_with('/'))
        })
}
