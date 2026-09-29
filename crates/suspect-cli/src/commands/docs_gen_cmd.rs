//! `suspect docs` — API reference generation in three styles.

use std::path::PathBuf;

use suspect_lsp::docs_gen;

/// Output style for the generated reference.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum DocsStyle {
    /// One self-contained HTML file.
    Html,
    /// Structured Markdown (frontmatter pages per tag/schema) for
    /// MDX/MDsvex pipelines.
    Markdown,
    /// A runnable SvelteKit site: generated data + routes layer with a
    /// scaffolded-once maintained layer for the rest of the site.
    Sveltekit,
}

/// One docs generation.
#[derive(Debug, clap::Args)]
pub struct DocsGenArgs {
    /// The OpenAPI document to document.
    #[arg(required = true)]
    pub input: PathBuf,
    /// Output style (default: a single HTML file, or `docs.style` from
    /// `.suspect.yaml`).
    #[arg(long, value_enum)]
    pub style: Option<DocsStyle>,
    /// Output path: a file for `html`, a directory for `markdown` and
    /// `sveltekit`.
    #[arg(long)]
    pub output: Option<PathBuf>,
    /// Override the document title (defaults to `info.title`).
    #[arg(long)]
    pub title: Option<String>,
}

/// Renders the reference in the requested style.
///
/// # Errors
/// Propagates IO and parsing failures.
pub fn docs_gen(args: &DocsGenArgs) -> anyhow::Result<i32> {
    let absolute = args.input.canonicalize()?;
    let text = std::fs::read_to_string(&absolute)?;
    let uri =
        suspect_source::Uri::from_path(&absolute).map_err(|e| anyhow::anyhow!(e.to_string()))?;
    let low = suspect_low::LowDoc::parse(
        uri,
        suspect_source::Source::from_vec(text.as_bytes().to_vec()),
    );
    let model = docs_gen::extract(&low);
    let model = if let Some(title) = &args.title {
        let mut titled = model.clone();
        titled.title = title.clone();
        titled
    } else {
        model
    };

    let style = args.style.unwrap_or(DocsStyle::Html);
    match style {
        DocsStyle::Html => {
            let html = docs_gen::render_html(&model);
            match &args.output {
                Some(path) => {
                    std::fs::write(path, &html)?;
                    eprintln!("docs → {} ({} bytes)", path.display(), html.len());
                }
                None => print!("{html}"),
            }
        }
        DocsStyle::Markdown => {
            let out = args.output.clone().ok_or_else(|| {
                anyhow::anyhow!("--output <dir> is required for --style markdown")
            })?;
            let report = docs_gen::render_markdown(&model, &out)?;
            eprintln!(
                "docs → {} ({} files: index, toc.json, {} operation pages, {} schema pages)",
                out.display(),
                report.files.len(),
                model.groups.len(),
                model.schemas.len(),
            );
        }
        DocsStyle::Sveltekit => {
            let out = args.output.clone().ok_or_else(|| {
                anyhow::anyhow!("--output <dir> is required for --style sveltekit")
            })?;
            let report = docs_gen::render_sveltekit(&model, &out)?;
            eprintln!(
                "docs → {}: {} generated files refreshed, {} maintained files scaffolded, {} maintained files left untouched",
                out.display(),
                report.generated.len(),
                report.scaffolded.len(),
                report.skipped.len(),
            );
            if !report.scaffolded.is_empty() {
                eprintln!("next: cd {}/ && npm install && npm run dev", out.display());
            }
        }
    }
    Ok(0)
}
