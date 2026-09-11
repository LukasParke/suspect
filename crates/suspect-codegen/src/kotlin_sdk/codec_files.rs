//! Bound JVM constant pools while retaining the public Codecs.member surface.
use super::{Plan, emit};
use crate::OutFile;
use std::collections::BTreeSet;

const MODELS_PER_CLASS: usize = 128;

pub(super) fn files(plan: &Plan) -> Vec<OutFile> {
    let prefix = format!("kotlin/src/main/kotlin/{}", plan.config.package_name.replace('.', "/"));
    let symbols = plan.models.symbols();
    if symbols.len() <= MODELS_PER_CLASS {
        return vec![OutFile { path: format!("{prefix}/Codecs.kt"), content: emit::codecs(plan) }];
    }
    let mut used = symbols.iter().map(|symbol| symbol.name.clone()).collect::<BTreeSet<_>>();
    let mut next = 0;
    let mut parent = None;
    let mut files = Vec::new();
    for group in symbols.chunks(MODELS_PER_CLASS) {
        let name = allocate(&mut used, &mut next);
        let base = parent.as_ref().map_or_else(String::new, |parent| format!(" : {parent}()"));
        files.push(OutFile {
            path: format!("{prefix}/{name}.kt"),
            content: emit::codec_group(plan, group, &format!("public sealed class {name} protected constructor(){base}")),
        });
        parent = Some(name);
    }
    files.push(OutFile {
        path: format!("{prefix}/Codecs.kt"),
        content: format!("{}/** Source-validating codecs; inherited groups bound JVM class size. */\npublic object Codecs : {}()\n", emit::header(plan), parent.expect("nonempty groups")),
    });
    files
}

fn allocate(used: &mut BTreeSet<String>, next: &mut usize) -> String {
    loop {
        let name = format!("CodecGroup{next}");
        *next += 1;
        let file_class = format!("{name}Kt");
        if used.contains(&name) || used.contains(&file_class) {
            continue;
        }
        used.insert(name.clone());
        used.insert(file_class);
        return name;
    }
}
