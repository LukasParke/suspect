//! Server-variable and tag checks.

use rustc_hash::FxHashSet;
use suspect_oas::{OpenApi, Server};

use super::{diag, template_vars};
use crate::diagnostic::{Diagnostic, Severity};

/// `oas-server-variable-unknown` (Error): every `{var}` in a server URL must
/// be declared in that server's `variables` map. Covers root, path-item, and
/// operation servers.
pub(crate) fn check_server_variables(api: &OpenApi<'_>, out: &mut Vec<Diagnostic>) {
    let check_servers = |servers: Vec<Server<'_>>, out: &mut Vec<Diagnostic>| {
        for server in servers {
            let Some(url) = server.url() else { continue };
            let declared: FxHashSet<&str> = server
                .variables()
                .into_iter()
                .map(|(name, _)| name)
                .collect();
            for var in template_vars(url) {
                if !declared.contains(var) {
                    out.push(diag(
                        api,
                        "oas-server-variable-unknown",
                        Severity::Error,
                        server.node().byte_range(),
                        format!("server URL `{url}` uses variable `{{{var}}}` that is not declared in `variables`"),
                    ));
                }
            }
        }
    };

    check_servers(api.servers(), out);
    if let Some(paths) = api.paths() {
        for (_, item) in paths.iter() {
            let r = item.resolved();
            check_servers(r.servers(), out);
            for op in r.operations() {
                check_servers(op.servers(), out);
            }
        }
    }
    if let Some(webhooks) = api.webhooks() {
        for (_, item) in webhooks.iter() {
            let r = item.resolved();
            check_servers(r.servers(), out);
            for op in r.operations() {
                check_servers(op.servers(), out);
            }
        }
    }
}
