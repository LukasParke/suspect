//! `suspect auth check`: verify the credential configuration without
//! running a single workflow.
//!
//! The credentials file maps security scheme names to strategies; a typo
//! in a scheme name or an expired client secret otherwise surfaces only
//! as a failing workflow deep in a run. This command resolves every
//! configured scheme the way a run would — OAuth schemes acquire a real
//! token through their token endpoint — and reports the outcome, printing
//! each scheme's wire placement and never a secret value.

use std::path::{Path, PathBuf};

use suspect_test::auth::AuthConfig;

/// Arguments for `suspect auth`.
#[derive(Debug, clap::Args)]
pub struct AuthArgs {
    /// The auth subcommand to run.
    #[command(subcommand)]
    pub cmd: AuthCmd,
}

/// `suspect auth` subcommands.
#[derive(Debug, clap::Subcommand)]
pub enum AuthCmd {
    /// Verify every configured credential without running a workflow.
    Check {
        /// Credentials file (default: discovered `.suspect/credentials.json`,
        /// or `SUSPECT_CREDENTIALS`).
        #[arg(long, value_name = "FILE")]
        credentials: Option<PathBuf>,
    },
}

/// `suspect auth check`: returns 0 when every scheme resolves.
pub fn check(credentials: Option<&Path>) -> anyhow::Result<i32> {
    let path = credentials
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("SUSPECT_CREDENTIALS").map(PathBuf::from))
        .or_else(|| {
            // From the current directory, like a run from the project root.
            suspect_test::auth::discover_credentials(Path::new("."))
        });
    let Some(path) = path else {
        eprintln!("no credentials file found (looked for .suspect/credentials.json upward)");
        return Ok(0);
    };
    let config: AuthConfig =
        suspect_test::auth::load_credentials_file(&path).map_err(|e| anyhow::anyhow!("{e}"))?;
    if config.schemes.is_empty() {
        println!("{}: no schemes configured", path.display());
        return Ok(0);
    }

    println!("{}", path.display());
    let rt = tokio::runtime::Runtime::new()?;
    let mut failed = 0usize;
    rt.block_on(async {
        let http = super::http::LiveTransport::new(std::time::Duration::from_secs(15))?;
        let state = suspect_test::auth::AuthState::default();
        for (scheme, credential) in &config.schemes {
            let outcome = state
                .resolve(&http, scheme, credential)
                .await
                .and_then(|placed| placed.ok_or_else(|| "resolved to no placement".to_owned()));
            match outcome {
                Ok(placement) => {
                    let (where_, name) = match &placement {
                        suspect_test::auth::Injected::Header(name, _) => ("header", name.clone()),
                        suspect_test::auth::Injected::Query(name, _) => ("query", name.clone()),
                    };
                    let kind = match credential {
                        suspect_test::auth::Credential::Bearer { .. } => "static bearer",
                        suspect_test::auth::Credential::ApiKey { .. } => "static api key",
                        suspect_test::auth::Credential::ClientCredentials { .. } => {
                            "oauth client-credentials (token acquired)"
                        }
                        suspect_test::auth::Credential::RefreshToken { .. } => {
                            "oauth refresh-token (token acquired)"
                        }
                    };
                    println!("  ok       {scheme}: {where_} `{name}` (value not shown)");
                    println!("           kind: {kind}");
                }
                Err(message) => {
                    failed += 1;
                    println!("  FAIL     {scheme}: {message}");
                }
            }
        }
        Ok::<_, anyhow::Error>(())
    })?;
    if failed > 0 {
        eprintln!("{failed} scheme(s) failed");
        Ok(1)
    } else {
        println!("all schemes resolve");
        Ok(0)
    }
}
