//! `suspect docs` — static API reference HTML generation.

use std::path::PathBuf;

/// One docs generation.
#[derive(Debug, clap::Args)]
pub struct DocsGenArgs {
    /// The OpenAPI document to document.
    #[arg(required = true)]
    pub input: PathBuf,
    /// Output HTML path (stdout when omitted).
    #[arg(long)]
    pub output: Option<PathBuf>,
    /// Override the document title (defaults to `info.title`).
    #[arg(long)]
    pub title: Option<String>,
}

/// Renders the reference HTML; writes to a file or stdout.
///
/// # Errors
/// Propagates IO and parsing failures.
pub fn docs_gen(args: &DocsGenArgs) -> anyhow::Result<i32> {
    let absolute = args.input.canonicalize()?;
    let text = std::fs::read_to_string(&absolute)?;
    let uri =
        suspect_source::Uri::from_path(&absolute).map_err(|e| anyhow::Error::msg(e.to_string()))?;
    let low = suspect_low::LowDoc::parse(
        uri,
        suspect_source::Source::from_vec(text.as_bytes().to_vec()),
    );
    let title = args.title.clone().unwrap_or_else(|| {
        low.root()
            .get("info")
            .and_then(|i| i.get("title"))
            .and_then(|t| t.as_str())
            .unwrap_or("API Reference")
            .to_owned()
    });
    let html = suspect_lsp::docs_gen::render(&low, &title);
    match &args.output {
        Some(path) => {
            std::fs::write(path, &html)?;
            eprintln!("docs → {} ({} bytes)", path.display(), html.len());
        }
        None => print!("{html}"),
    }
    Ok(0)
}
