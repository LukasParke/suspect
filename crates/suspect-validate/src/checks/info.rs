//! Info-object checks: license identification.

use suspect_oas::{OasVersion, OpenApi};

use super::diag;
use crate::diagnostic::{Diagnostic, Severity};

/// `oas-license-missing-url` (Warning): a license must carry `url` on 3.0,
/// and either `url` or `identifier` on 3.1+.
pub(crate) fn check_license(api: &OpenApi<'_>, out: &mut Vec<Diagnostic>) {
    let Some(info) = api.info() else { return };
    let Some(license) = info.license() else { return };
    let has_url = license.url().is_some();
    let has_id = license.identifier().is_some();
    let missing = match api.version() {
        OasVersion::V30 => !has_url,
        OasVersion::V31 | OasVersion::V32 => !has_url && !has_id,
    };
    if missing {
        let name = license.name().unwrap_or("<unnamed>");
        out.push(diag(
            api,
            "oas-license-missing-url",
            Severity::Warning,
            license.node().byte_range(),
            format!("license `{name}` has neither `url` nor `identifier`"),
        ));
    }
}
