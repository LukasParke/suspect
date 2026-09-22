//! Content-addressed canonical SDK sessions sharing one immutable input graph.
use crate::{
    OutFile,
    backend::{self, BackendDiagnostic, GenerationOptions, TargetConfig},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    io::Read,
    path::{Path, PathBuf},
    sync::Arc,
};
use suspect_ir::contract::Contract;
use suspect_ref::{
    Workspace, WorkspaceBuilder,
    acquire::{AcquireOptions, AcquiredClosure, acquire},
};
use suspect_source::Uri;

/// Physical input configuration, separate from the logical document identities
/// carried by the compiled Contract. Pinned inputs are always cache-only here.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum Input {
    File {
        path: PathBuf,
    },
    Pinned {
        manifest: PathBuf,
        cache_dir: PathBuf,
        #[serde(default)]
        insecure_test_origins: Vec<String>,
    },
}

impl Input {
    /// The configured input file used by watch/editor identity: entry or manifest.
    pub fn path(&self) -> &Path {
        match self {
            Self::File { path } => path,
            Self::Pinned { manifest, .. } => manifest,
        }
    }

    /// Resolve lexical paths without following symlinks or changing retrieval bases.
    pub fn normalized(self) -> Result<Self, SessionError> {
        let path = |path: PathBuf| {
            Uri::from_path(&path)
                .map_err(|error| SessionError::Input(error.to_string()))?
                .as_path()
                .ok_or_else(|| {
                    SessionError::Input("input configuration requires file paths".into())
                })
        };
        Ok(match self {
            Self::File { path: value } => Self::File { path: path(value)? },
            Self::Pinned {
                manifest,
                cache_dir,
                insecure_test_origins,
            } => Self::Pinned {
                manifest: path(manifest)?,
                cache_dir: path(cache_dir)?,
                insecure_test_origins,
            },
        })
    }

    fn pins(&self) -> Result<Option<AcquiredClosure>, SessionError> {
        match self {
            Self::File { .. } => Ok(None),
            Self::Pinned {
                manifest,
                cache_dir,
                insecure_test_origins,
            } => acquire(
                manifest,
                AcquireOptions {
                    cache_dir: cache_dir.clone(),
                    offline: true,
                    insecure_test_origins: insecure_test_origins.clone(),
                    ..Default::default()
                },
            )
            .map(Some)
            .map_err(SessionError::Acquisition),
        }
    }

    fn open_verified(
        &self,
        pins: Option<&AcquiredClosure>,
    ) -> Result<(Arc<Workspace>, Uri), SessionError> {
        let (builder, entry) = match self {
            Self::File { path } => (
                WorkspaceBuilder::new().root(path.parent().unwrap_or(Path::new("."))),
                Uri::from_path(path).map_err(|error| SessionError::Input(error.to_string()))?,
            ),
            Self::Pinned { .. } => {
                let pins = pins.expect("pinned bytes verified before workspace construction");
                (pins.workspace_builder(), pins.entry().clone())
            }
        };
        let workspace = Arc::new(
            builder
                .build()
                .map_err(|error| SessionError::Input(error.to_string()))?,
        );
        Ok((workspace, entry))
    }

    /// Open local sources or reverify the complete pinned cache. This interface
    /// never acquires missing documents or performs network requests.
    pub fn open(&self) -> Result<(Arc<Workspace>, Uri), SessionError> {
        self.open_verified(self.pins()?.as_ref())
    }
}

#[derive(Debug, Clone)]
pub struct SessionConfig {
    pub targets: Vec<TargetConfig>,
    pub operation_ids: Vec<String>,
    /// Versioned interpretation shared by selected targets; native capability
    /// fences remain per adapter. Included in complete and target cache keys.
    pub generation: GenerationOptions,
    pub cache_entries: usize,
    /// Retained input/artifact bytes; graph overhead is additionally bounded by entries.
    pub cache_bytes: usize,
    pub owner: String,
}
impl Default for SessionConfig {
    fn default() -> Self {
        Self {
            targets: Vec::new(),
            operation_ids: Vec::new(),
            generation: GenerationOptions::default(),
            cache_entries: 4,
            cache_bytes: 128 * 1024 * 1024,
            owner: "suspect-sdk:session".into(),
        }
    }
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Stats {
    pub compiles: usize,
    pub renders: usize,
    pub cache_hits: usize,
}
#[derive(Debug, Clone)]
pub struct SessionOutput {
    /// Content identity of the loaded source closure and generation configuration.
    pub revision: String,
    pub contract: Arc<Contract>,
    pub files: Arc<Vec<OutFile>>,
    pub changed_paths: Vec<String>,
    pub new_documents: Vec<String>,
    pub delta: Stats,
    pub stats: Stats,
}
#[derive(Debug)]
pub enum SessionError {
    Input(String),
    Acquisition(suspect_ref::acquire::AcquireError),
    Configuration(String),
    Backend(Vec<BackendDiagnostic>),
    Write(String),
}
impl std::fmt::Display for SessionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Input(s) | Self::Configuration(s) | Self::Write(s) => f.write_str(s),
            Self::Acquisition(error) => std::fmt::Display::fmt(error, f),
            Self::Backend(findings) => write!(f, "backend refused generation: {findings:?}"),
        }
    }
}
impl std::error::Error for SessionError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Acquisition(error) => Some(error),
            _ => None,
        }
    }
}
type DigestMap = BTreeMap<PathBuf, String>;
struct Snapshot {
    contract: Arc<Contract>,
    files: Arc<Vec<OutFile>>,
    closure: DigestMap,
    missing: BTreeSet<PathBuf>,
    configuration: String,
    bytes: usize,
    source_bytes: usize,
    targets: BTreeMap<String, Vec<String>>,
    pinned_identity: Option<String>,
    documents: BTreeSet<String>,
}
pub struct Session {
    entry: PathBuf,
    input: Input,
    config: SessionConfig,
    cache: VecDeque<Snapshot>,
    previous: BTreeMap<String, String>,
    previous_documents: BTreeSet<String>,
    stats: Stats,
}

fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn read(path: &Path) -> Result<Vec<u8>, SessionError> {
    const MAXIMUM: u64 = 64 << 20;
    let io = |error| SessionError::Input(format!("{}: {error}", path.display()));
    let metadata = std::fs::metadata(path).map_err(io)?;
    if !metadata.is_file() || metadata.len() > MAXIMUM {
        return Err(SessionError::Input(format!(
            "{}: expected a regular source file of at most {MAXIMUM} bytes",
            path.display()
        )));
    }
    let file = std::fs::File::open(path).map_err(io)?;
    if !file.metadata().map_err(io)?.is_file() {
        return Err(SessionError::Input("source is not a regular file".into()));
    }
    let mut bytes = Vec::new();
    file.take(MAXIMUM + 1).read_to_end(&mut bytes).map_err(io)?;
    if bytes.len() as u64 > MAXIMUM {
        return Err(SessionError::Input(
            "source exceeds the 64 MiB document ceiling".into(),
        ));
    }
    Ok(bytes)
}
fn configuration(config: &SessionConfig) -> String {
    let value = serde_json::json!({"format":"suspect.sdk.session.v1","generator":env!("CARGO_PKG_VERSION"),"targets":config.targets,"operationIds":config.operation_ids,"generation":config.generation});
    hash(&serde_json::to_vec(&value).expect("session identity"))
}
impl Session {
    pub fn new(entry: impl AsRef<Path>, config: SessionConfig) -> Result<Self, SessionError> {
        Self::with_input(
            Input::File {
                path: entry.as_ref().to_owned(),
            },
            config,
        )
    }

    /// Create a session over an explicit local or cache-only pinned input.
    pub fn with_input(input: Input, config: SessionConfig) -> Result<Self, SessionError> {
        let input = input.normalized()?;
        let entry = input.path().to_owned();
        Self::valid(&config)?;
        Ok(Self {
            entry,
            input,
            config,
            cache: VecDeque::new(),
            previous: BTreeMap::new(),
            previous_documents: BTreeSet::new(),
            stats: Stats::default(),
        })
    }
    fn valid(config: &SessionConfig) -> Result<(), SessionError> {
        if config.targets.is_empty()
            || config.cache_entries == 0
            || config.cache_bytes == 0
            || config.owner.is_empty()
        {
            return Err(SessionError::Configuration(
                "targets, cache budgets and owner must be nonempty".into(),
            ));
        }
        let unique = config
            .targets
            .iter()
            .map(|target| target.backend)
            .collect::<BTreeSet<_>>();
        if unique.len() != config.targets.len() {
            return Err(SessionError::Configuration(
                "select each backend only once per artifact root".into(),
            ));
        }
        Ok(())
    }
    pub fn set_config(&mut self, config: SessionConfig) -> Result<(), SessionError> {
        Self::valid(&config)?;
        self.config = config;
        self.evict();
        Ok(())
    }
    pub fn stats(&self) -> Stats {
        self.stats
    }
    fn current(
        snapshot: &Snapshot,
        entry: &Path,
        entry_hash: &str,
        pinned_identity: Option<&str>,
    ) -> bool {
        if pinned_identity.is_some() || snapshot.pinned_identity.is_some() {
            // pins() has already verified every manifest/cache byte before this
            // decision, including files not in the selected schema closure.
            return pinned_identity == snapshot.pinned_identity.as_deref();
        }
        snapshot
            .closure
            .get(entry)
            .is_some_and(|digest| digest == entry_hash)
            && snapshot.closure.iter().all(|(path, digest)| {
                path == entry || read(path).is_ok_and(|bytes| hash(&bytes) == *digest)
            })
            && snapshot
                .missing
                .iter()
                .all(|path| path.try_exists().is_ok_and(|exists| !exists))
    }
    pub fn generate(&mut self) -> Result<SessionOutput, SessionError> {
        let pins = self.input.pins()?;
        let pinned_identity=pins.as_ref().map(|pins|hash(&serde_json::to_vec(&serde_json::json!({
            "manifest":pins.fingerprint(),"provider":pins.provider().fingerprint(),"entry":pins.entry().as_str(),
        })).expect("pinned input identity")));
        let entry_bytes = read(&self.entry)?;
        let entry_hash = hash(&entry_bytes);
        if let Some(pins) = &pins
            && pins.fingerprint() != format!("sha256-{entry_hash}")
        {
            return Err(SessionError::Input(
                "pin manifest changed during verification".into(),
            ));
        }
        let config_hash = configuration(&self.config);
        let mut delta = Stats::default();
        let matching = self.cache.iter().position(|snapshot| {
            snapshot.configuration == config_hash
                && Self::current(
                    snapshot,
                    &self.entry,
                    &entry_hash,
                    pinned_identity.as_deref(),
                )
        });
        let (contract, files, documents, revision) = if let Some(index) = matching {
            let snapshot = self.cache.remove(index).expect("cache index");
            let contract = snapshot.contract.clone();
            let files = snapshot.files.clone();
            let docs = snapshot.documents.clone();
            let revision = revision(
                &snapshot.closure,
                &snapshot.missing,
                &config_hash,
                snapshot.pinned_identity.as_deref(),
            );
            self.cache.push_front(snapshot);
            delta.cache_hits = 1;
            (contract, files, docs, revision)
        } else {
            let reusable = self.cache.iter().find(|snapshot| {
                Self::current(
                    snapshot,
                    &self.entry,
                    &entry_hash,
                    pinned_identity.as_deref(),
                )
            });
            let (contract, closure, missing, source_bytes, cacheable, documents) =
                if let Some(snapshot) = reusable {
                    (
                        snapshot.contract.clone(),
                        snapshot.closure.clone(),
                        snapshot.missing.clone(),
                        snapshot.source_bytes,
                        true,
                        snapshot.documents.clone(),
                    )
                } else {
                    let (workspace, entry) = self.input.open_verified(pins.as_ref())?;
                    let contract = Arc::new(
                        Contract::from_workspace(&workspace, &entry)
                            .map_err(|error| SessionError::Input(error.to_string()))?,
                    );
                    let mut closure = BTreeMap::new();
                    let mut source_bytes = 0;
                    let documents: BTreeSet<String>;
                    // Include documents loaded by failed pointer/anchor resolution,
                    // even when they could not enter the accepted Contract graph.
                    if let Some(pins) = &pins {
                        closure.insert(self.entry.clone(), entry_hash.clone());
                        closure.insert(pins.cache_manifest_path().to_owned(), entry_hash.clone());
                        for document in pins.documents() {
                            let cache = document.cache_path().ok_or_else(|| {
                                SessionError::Input("pinned cache metadata is missing".into())
                            })?;
                            let digest =
                                document.digest().strip_prefix("sha256-").ok_or_else(|| {
                                    SessionError::Input("pinned digest metadata is invalid".into())
                                })?;
                            closure.insert(cache.to_owned(), digest.to_owned());
                            source_bytes += usize::try_from(document.byte_len()).map_err(|_| {
                                SessionError::Input(
                                    "pinned document size exceeds this host's address space".into(),
                                )
                            })?;
                        }
                        source_bytes += entry_bytes.len();
                        documents = workspace.uris().iter().map(ToString::to_string).collect();
                    } else {
                        for uri in workspace.uris() {
                            let path = uri.as_path().ok_or_else(|| {
                                SessionError::Input(format!("non-file input {uri}"))
                            })?;
                            let document = workspace.get(&uri).ok_or_else(|| {
                                SessionError::Input("compiled source document is missing".into())
                            })?;
                            let data = document.doc().inner().bytes();
                            source_bytes += data.len();
                            closure.insert(path, hash(data));
                        }
                        documents = closure
                            .keys()
                            .map(|path| path.display().to_string())
                            .collect();
                    }
                    let mut missing = BTreeSet::new();
                    let mut cacheable = true;
                    for uri in workspace.failed_document_uris() {
                        if pins.is_some() {
                            continue;
                        } // The closed provider/manifest determines misses.
                        // Remote policy is immutable for this file-only session.
                        let Some(path) = uri.as_path() else { continue };
                        if closure.contains_key(&path) {
                            continue;
                        }
                        if path.try_exists().is_ok_and(|exists| !exists) {
                            missing.insert(path);
                        } else {
                            cacheable = false;
                        } // Unreadable/oversized inputs may recover; never guess their bytes.
                    }
                    delta.compiles = 1;
                    (
                        contract,
                        closure,
                        missing,
                        source_bytes,
                        cacheable,
                        documents,
                    )
                };
            // Bytes changed while resolving: don't associate artifacts with a stale identity.
            if pins.is_none() && closure.get(&self.entry) != Some(&entry_hash) {
                return Err(SessionError::Input(
                    "input changed while compiling the snapshot".into(),
                ));
            }
            let mut selected = Vec::new();
            if self.config.operation_ids.is_empty() {
                selected.extend(contract.operations().map(|op| op.source().clone()));
            } else {
                for id in &self.config.operation_ids {
                    // A selector names either the operation's operationId or,
                    // for unnamed operations, the `METHOD /path` identity the
                    // SDK-behavior planners use. Both spellings must reach
                    // exactly one source operation.
                    let found = contract
                        .operations()
                        .filter(|op| op.operation_id() == Some(id.as_str()))
                        .collect::<Vec<_>>();
                    if found.len() != 1 {
                        if found.is_empty() {
                            let method_path = id.split_once(' ');
                            if let Some((method, path)) = method_path {
                                let by_path = contract
                                    .operations()
                                    .filter(|op| {
                                        op.operation_id().is_none()
                                            && op.method().as_str().eq_ignore_ascii_case(method)
                                            && op.path_template() == Some(path)
                                    })
                                    .collect::<Vec<_>>();
                                if by_path.len() == 1 {
                                    selected.push(by_path[0].source().clone());
                                    continue;
                                }
                            }
                        }
                        return Err(SessionError::Configuration(format!(
                            "operationId {id:?} requires exactly one source operation"
                        )));
                    };
                    selected.push(found[0].source().clone());
                }
            }
            selected.sort();
            selected.dedup();
            let mut files = Vec::new();
            let mut targets = BTreeMap::new();
            for target in &self.config.targets {
                let key=hash(&serde_json::to_vec(&serde_json::json!({"target":target,"generation":self.config.generation,"sources":selected.iter().map(|source|serde_json::json!({"document":source.document().as_str(),"pointer":source.pointer()})).collect::<Vec<_>>()})).expect("target identity"));
                let cached = self.cache.iter().find(|snapshot| {
                    Arc::ptr_eq(&snapshot.contract, &contract)
                        && snapshot.targets.contains_key(&key)
                });
                let generated = if let Some(snapshot) = cached {
                    let paths = &snapshot.targets[&key];
                    delta.cache_hits += 1;
                    snapshot
                        .files
                        .iter()
                        .filter(|file| paths.contains(&file.path))
                        .cloned()
                        .collect::<Vec<_>>()
                } else {
                    delta.renders += 1;
                    backend::generate_with_options(
                        contract.clone(),
                        &selected,
                        target,
                        &self.config.generation,
                    )
                    .map_err(SessionError::Backend)?
                };
                targets.insert(
                    key,
                    generated.iter().map(|file| file.path.clone()).collect(),
                );
                files.extend(generated);
            }
            files.sort_by(|a, b| a.path.cmp(&b.path));
            if files.windows(2).any(|pair| pair[0].path == pair[1].path) {
                return Err(SessionError::Configuration(
                    "backend artifact roots collide".into(),
                ));
            }
            if pins.is_some() {
                let after = self.input.pins()?.expect("pinned input");
                let identity=hash(&serde_json::to_vec(&serde_json::json!({
                    "manifest":after.fingerprint(),"provider":after.provider().fingerprint(),"entry":after.entry().as_str(),
                })).expect("pinned input identity"));
                if Some(&identity) != pinned_identity.as_ref() {
                    return Err(SessionError::Input(
                        "pinned input changed during planning".into(),
                    ));
                }
            } else {
                for (path, digest) in &closure {
                    if hash(&read(path)?) != *digest {
                        return Err(SessionError::Input(format!(
                            "input changed during planning: {}",
                            path.display()
                        )));
                    }
                }
            }
            for path in &missing {
                if !path.try_exists().is_ok_and(|exists| !exists) {
                    return Err(SessionError::Input(format!(
                        "missing input changed during planning: {}",
                        path.display()
                    )));
                }
            }
            let files = Arc::new(files);
            let bytes = source_bytes + files.iter().map(|file| file.content.len()).sum::<usize>();
            let revision = revision(&closure, &missing, &config_hash, pinned_identity.as_deref());
            if cacheable && bytes <= self.config.cache_bytes {
                self.cache.push_front(Snapshot {
                    contract: contract.clone(),
                    files: files.clone(),
                    closure,
                    missing,
                    configuration: config_hash,
                    bytes,
                    source_bytes,
                    targets,
                    pinned_identity: pinned_identity.clone(),
                    documents: documents.clone(),
                });
                self.evict();
            }
            (contract, files, documents, revision)
        };
        let documents: BTreeSet<String> = documents;
        let fingerprints = files
            .iter()
            .map(|file| (file.path.clone(), hash(file.content.as_bytes())))
            .collect::<BTreeMap<_, _>>();
        let changed_paths = changed(&self.previous, &fingerprints);
        let new_documents = documents
            .difference(&self.previous_documents)
            .cloned()
            .collect();
        self.stats.compiles += delta.compiles;
        self.stats.renders += delta.renders;
        self.stats.cache_hits += delta.cache_hits;
        self.previous = fingerprints;
        self.previous_documents = documents;
        Ok(SessionOutput {
            revision,
            contract,
            files,
            changed_paths,
            new_documents,
            delta,
            stats: self.stats,
        })
    }
    fn evict(&mut self) {
        while self.cache.len() > self.config.cache_entries
            || self
                .cache
                .iter()
                .map(|snapshot| snapshot.bytes)
                .sum::<usize>()
                > self.config.cache_bytes
        {
            self.cache.pop_back();
        }
    }
    pub fn write(
        &self,
        output: &SessionOutput,
        directory: impl AsRef<Path>,
    ) -> Result<(), SessionError> {
        crate::write_files_with_owner(
            &output.files,
            directory.as_ref(),
            &self.config.owner,
            crate::Adoption::Refuse,
        )
        .map_err(SessionError::Write)
    }
}
fn revision(
    closure: &DigestMap,
    missing: &BTreeSet<PathBuf>,
    configuration: &str,
    pinned_identity: Option<&str>,
) -> String {
    if let Some(input) = pinned_identity {
        return hash(
            &serde_json::to_vec(
                &serde_json::json!({"configuration":configuration,"pinnedInput":input}),
            )
            .expect("pinned revision"),
        );
    }
    hash(
        &serde_json::to_vec(
            &serde_json::json!({"configuration":configuration,"documents":closure,"missing":missing}),
        )
        .expect("snapshot identity"),
    )
}
fn changed(previous: &BTreeMap<String, String>, next: &BTreeMap<String, String>) -> Vec<String> {
    previous
        .keys()
        .chain(next.keys())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .filter(|path| previous.get(*path) != next.get(*path))
        .cloned()
        .collect()
}
