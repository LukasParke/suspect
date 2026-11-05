//! `codegen.compatibility_profiles` in the setting table accepts exactly the
//! profiles `suspect gen sdk --compatibility-profile` does. suspect-config
//! does not depend on suspect-codegen, so the list is checked here.

use suspect_codegen::http_protocol::CompatibilityProfile;

#[test]
fn compatibility_profiles_match_codegen() {
    let codegen: Vec<&str> = CompatibilityProfile::ALL.iter().map(|p| p.name()).collect();
    assert_eq!(suspect_config::schema::COMPATIBILITY_PROFILES, codegen);
    let row = suspect_config::schema::SETTINGS
        .iter()
        .find(|s| s.path == "codegen.compatibility_profiles")
        .unwrap();
    assert_eq!(row.values(), codegen);
}
