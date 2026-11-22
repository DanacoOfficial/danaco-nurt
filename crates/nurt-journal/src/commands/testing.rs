// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Journal on in-memory volumes, execution contexts and catalog signatures
//! for the tests of the journal stages.
//!
//! The file system has the volumes `/` and `/home`; only `/home` has a
//! journal, and the account state directory lies on it. Every context has the
//! session [`SESSION`] and a bound program.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use nurt_contracts::bound::{BoundDocument, BoundProgram};
use nurt_contracts::config::{ConfigLayer, ConfigLevel};
use nurt_contracts::env::{Host, HostInfo, Os, PathStyle};
use nurt_contracts::envelope::RequestOptions;
use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::fs::{AbsPath, VolumeInfo};
use nurt_contracts::journal::{
    BeginRequest, FileOp, HistoryQuery, InactiveReason, JournalStore, MutationTx, OpId, OpKind,
    SessionId, VolumeSpace, apply_ops,
};
use nurt_contracts::record::Change;
use nurt_contracts::report::IrreversibleEntry;
use nurt_contracts::spec::{
    CatalogFile, DefaultKind, DefaultValue, ParamType, ScalarType, StageSpec,
};
use nurt_contracts::stage::{
    ArgValue, BoundArgs, EffectPlan, ExecCtx, Extensions, StageData, StageFrame, StageImpl,
    StageInput, StageOutput,
};
use nurt_contracts::value::{self, Record, Size, Value};
use nurt_fs::clock::VirtualClock;
use nurt_fs::mem::{MemFs, VolumeSpec};

use crate::layout::{self, JournalLayout};
use crate::store::test_support::TestAdmission;
use crate::store::{JournalLocation, SpaceReporter, VolumeJournals};
use crate::{ConfiguredLimits, Journal, JournalBuilder};

const HOME: &str = "/home";
const DOCS: &str = "/home/user/docs";
const STATE_DIR: &str = "/home/user/.local/state/nurt";
/// Session of every context and of the operations written by [`Env::write`].
pub(super) const SESSION: &str = "s1";
/// Short lock wait, so that a held lock fails a second taker at once.
const LOCK_TIMEOUT: Duration = Duration::from_millis(20);
/// Mount point of the second volume reported by [`FixedSpace`].
const DATA: &str = "/mnt/data";
const GIB: u64 = 1 << 30;

pub(super) fn path(text: &str) -> AbsPath {
    AbsPath::new(text).unwrap()
}

/// Path value used where a parameter needs any path.
pub(super) fn sample_path() -> Value {
    Value::Path(format!("{DOCS}/notes.txt"))
}

fn journal_root() -> AbsPath {
    path(HOME).join_name(layout::default_dir_name()).unwrap()
}

/// Journal directory of the volume `/home` only.
struct HomeJournal;

impl VolumeJournals for HomeJournal {
    fn locate(&self, volume: &VolumeInfo) -> JournalLocation {
        if volume.mount == path(HOME) {
            JournalLocation::Active(journal_root())
        } else {
            JournalLocation::Inactive(InactiveReason::NoAnchor)
        }
    }

    fn active(&self) -> Vec<AbsPath> {
        vec![journal_root()]
    }
}

/// Space service with the volumes `/home` (journal active, 100 GiB) and
/// `/mnt/data` (no journal).
struct FixedSpace;

impl FixedSpace {
    fn volume(mount: &str, inactive: Option<InactiveReason>) -> VolumeSpace {
        VolumeSpace {
            mount: path(mount),
            filesystem: "ext4".to_owned(),
            total: 100 * GIB,
            free: 40 * GIB,
            free_without_journal: 42 * GIB,
            reclaimable: 2 * GIB,
            reserve: 15 * GIB,
            soft: 20 * GIB,
            journal_bytes: 2 * GIB,
            inactive,
        }
    }
}

impl SpaceReporter for FixedSpace {
    fn space(&self, volume: Option<&AbsPath>) -> Result<Vec<VolumeSpace>> {
        let all = vec![
            Self::volume(HOME, None),
            Self::volume(DATA, Some(InactiveReason::NoAnchor)),
        ];
        let Some(volume) = volume else {
            return Ok(all);
        };
        let found: Vec<VolumeSpace> = all
            .into_iter()
            .filter(|space| volume.starts_with(&space.mount))
            .collect();
        if found.is_empty() {
            return Err(Error::new(
                ErrorCode::NotFound,
                format!("no volume contains {volume}"),
            ));
        }
        Ok(found)
    }
}

/// Host facts of a Linux account `user`.
pub(super) struct TestHost {
    info: HostInfo,
}

impl TestHost {
    pub(super) fn new() -> Self {
        Self {
            info: HostInfo {
                os: Os::Linux,
                os_version: "6.8.0".to_owned(),
                architecture: "x86_64".to_owned(),
                hostname: "example".to_owned(),
                user: "user".to_owned(),
                home: PathBuf::from("/home/user"),
                temp: PathBuf::from("/tmp"),
                path_style: PathStyle::Posix,
                timezone: "UTC".to_owned(),
                locale: None,
                cpus: 1,
            },
        }
    }
}

impl Host for TestHost {
    fn info(&self) -> &HostInfo {
        &self.info
    }

    fn env_var(&self, _name: &str) -> Option<OsString> {
        None
    }

    fn env_vars(&self) -> Vec<(OsString, OsString)> {
        Vec::new()
    }
}

fn program() -> BoundDocument {
    BoundDocument::new(
        "catalog",
        "corrections",
        BoundProgram {
            statements: Vec::new(),
        },
    )
}

fn begin(command: &str) -> BeginRequest {
    BeginRequest {
        kind: OpKind::command(command),
        session: Some(SessionId(SESSION.to_owned())),
        program: program(),
        corrections: Vec::new(),
        bulk: false,
        counts_as_step: true,
        preserve_budget: None,
    }
}

/// Context of `command` without a journal store and a bound program.
pub(super) fn bare_ctx(command: &str) -> ExecCtx {
    ExecCtx::builder(Arc::new(VirtualClock::default()), Arc::new(TestHost::new()))
        .frame(StageFrame {
            command: command.to_owned(),
            ..StageFrame::default()
        })
        .build()
}

/// In-memory volumes, a virtual clock and a journal with built-in limits.
pub(super) struct Env {
    fs: Arc<MemFs>,
    clock: Arc<VirtualClock>,
    journal: Arc<Journal>,
}

impl Env {
    /// Environment without a space service.
    pub(super) fn new() -> Self {
        Self::build(None)
    }

    /// Environment whose space service reports `/home` and `/mnt/data`.
    pub(super) fn with_space() -> Self {
        Self::build(Some(Arc::new(FixedSpace)))
    }

    fn build(space: Option<Arc<dyn SpaceReporter>>) -> Self {
        let clock = Arc::new(VirtualClock::default());
        let fs = MemFs::builder()
            .clock(clock.clone())
            .volume(VolumeSpec::posix(path("/")))
            .volume(VolumeSpec::posix(path(HOME)))
            .build()
            .unwrap();
        for dir in JournalLayout::new(journal_root()).structure().unwrap() {
            fs.seed_dir(&dir).unwrap();
        }
        fs.seed_dir(&path(STATE_DIR)).unwrap();
        fs.seed_dir(&path(DOCS)).unwrap();
        let fs = Arc::new(fs);
        let layers = vec![ConfigLayer::new(
            ConfigLevel::User,
            "user.toml",
            serde_json::Map::new(),
        )];
        let limits = ConfiguredLimits::new(&layers, false, fs.clone(), &path(STATE_DIR)).unwrap();
        let mut builder = JournalBuilder::new(
            fs.clone(),
            Arc::new(TestAdmission::default()),
            clock.clone(),
            Arc::new(HomeJournal),
            Arc::new(limits),
            path(STATE_DIR),
        )
        .lock_timeout(LOCK_TIMEOUT);
        if let Some(space) = space {
            builder = builder.space_reporter(space);
        }
        Self {
            fs,
            clock,
            journal: Arc::new(builder.build().unwrap()),
        }
    }

    /// Creates the file `name` in the documents directory.
    pub(super) fn seed(&self, name: &str, content: &[u8]) -> AbsPath {
        let target = path(&format!("{DOCS}/{name}"));
        self.fs.seed_file(&target, content).unwrap();
        target
    }

    pub(super) fn read(&self, target: &AbsPath) -> Vec<u8> {
        crate::manifest::read_bytes(self.fs.as_ref(), target).unwrap()
    }

    /// Replaces the content of `target` as one journal operation of the
    /// session.
    pub(super) fn write(&self, target: &AbsPath, content: &[u8]) -> OpId {
        let mut tx = self.journal.begin(begin("write"));
        apply_ops(
            tx.as_mut(),
            vec![FileOp::write(target.clone(), content.to_vec())],
        )
        .unwrap();
        tx.commit().unwrap().op.unwrap()
    }

    /// Operations 1, 2 and 3, each writing its own file.
    pub(super) fn three_operations(&self) {
        for name in ["a.txt", "b.txt", "c.txt"] {
            let target = self.seed(name, name.as_bytes());
            self.write(&target, b"new");
        }
    }

    /// Number of operations that can still be undone.
    pub(super) fn available(&self) -> u64 {
        self.journal.status(None).unwrap()[0].entries_available
    }

    pub(super) fn pinned(&self, op: u64) -> bool {
        let query = HistoryQuery {
            id: Some(OpId(op)),
            ..HistoryQuery::default()
        };
        self.journal.history(&query).unwrap()[0].pinned
    }

    /// Context of `command` with the journal, the session and the program.
    pub(super) fn ctx(&self, command: &str) -> ExecCtx {
        self.ctx_with(command, |_| {})
    }

    /// As [`Env::ctx`], with request options adjusted by `adjust`.
    pub(super) fn ctx_with(
        &self,
        command: &str,
        adjust: impl FnOnce(&mut RequestOptions),
    ) -> ExecCtx {
        let mut options = RequestOptions {
            session: Some(SESSION.to_owned()),
            ..RequestOptions::default()
        };
        adjust(&mut options);
        let mut extensions = Extensions::new();
        extensions.insert::<BoundDocument>(Arc::new(program()));
        ExecCtx::builder(self.clock.clone(), Arc::new(TestHost::new()))
            .journal(self.journal.clone())
            .options(Arc::new(options))
            .extensions(extensions)
            .frame(StageFrame {
                command: command.to_owned(),
                ..StageFrame::default()
            })
            .build()
    }
}

fn collect(output: StageOutput<'_>) -> Result<Vec<Record>> {
    match output.data {
        StageData::Records(stream) => stream.collect(),
        StageData::None => Ok(Vec::new()),
        other => panic!("the stage produced {other:?} instead of records"),
    }
}

/// Records of a read stage.
pub(super) fn read(stage: &dyn StageImpl, args: &BoundArgs, ctx: &ExecCtx) -> Result<Vec<Record>> {
    let output = stage
        .as_read()
        .unwrap()
        .execute(args, StageInput::none(), ctx)?;
    collect(output)
}

/// Plan of an effect stage.
pub(super) fn plan(stage: &dyn StageImpl, args: &BoundArgs, ctx: &ExecCtx) -> Result<EffectPlan> {
    stage
        .as_effect()
        .unwrap()
        .plan(args, StageInput::none(), ctx)
}

/// Result of an applied effect stage.
#[derive(Debug)]
pub(super) struct Done {
    pub(super) records: Vec<Record>,
    pub(super) changes: Vec<Change>,
    pub(super) irreversible: Vec<IrreversibleEntry>,
}

/// Plans and applies an effect stage in a transaction of the executor, then
/// commits the transaction and checks that it recorded no entry.
pub(super) fn apply(
    env: &Env,
    stage: &dyn StageImpl,
    args: &BoundArgs,
    ctx: &ExecCtx,
) -> Result<Done> {
    let effect = stage.as_effect().unwrap();
    let planned = effect.plan(args, StageInput::none(), ctx)?;
    let mut tx: Box<dyn MutationTx + '_> = env.journal.begin(begin(&ctx.frame().command));
    let outcome = effect.apply(args, planned, tx.as_mut(), ctx)?;
    let done = Done {
        records: collect(outcome.output)?,
        changes: outcome.changes,
        irreversible: outcome.irreversible,
    };
    assert_eq!(tx.commit().unwrap().op, None);
    Ok(done)
}

fn table_json(table: &toml_edit::Table) -> serde_json::Value {
    serde_json::Value::Object(
        table
            .iter()
            .map(|(key, item)| (key.to_owned(), item_json(item)))
            .collect(),
    )
}

fn item_json(item: &toml_edit::Item) -> serde_json::Value {
    match item {
        toml_edit::Item::Value(value) => value_json(value),
        toml_edit::Item::Table(table) => table_json(table),
        toml_edit::Item::ArrayOfTables(tables) => {
            serde_json::Value::Array(tables.iter().map(table_json).collect())
        }
        toml_edit::Item::None => serde_json::Value::Null,
    }
}

fn value_json(value: &toml_edit::Value) -> serde_json::Value {
    match value {
        toml_edit::Value::String(text) => serde_json::Value::from(text.value().as_str()),
        toml_edit::Value::Integer(number) => serde_json::Value::from(*number.value()),
        toml_edit::Value::Float(number) => serde_json::Value::from(*number.value()),
        toml_edit::Value::Boolean(flag) => serde_json::Value::from(*flag.value()),
        toml_edit::Value::Datetime(time) => serde_json::Value::from(time.value().to_string()),
        toml_edit::Value::Array(items) => {
            serde_json::Value::Array(items.iter().map(value_json).collect())
        }
        toml_edit::Value::InlineTable(table) => serde_json::Value::Object(
            table
                .iter()
                .map(|(key, value)| (key.to_owned(), value_json(value)))
                .collect(),
        ),
    }
}

/// Commands of the catalog files `journal.toml` and `core.toml`, by name.
fn catalog() -> &'static BTreeMap<String, StageSpec> {
    static CATALOG: OnceLock<BTreeMap<String, StageSpec>> = OnceLock::new();
    CATALOG.get_or_init(|| {
        let mut commands = BTreeMap::new();
        for name in ["journal.toml", "core.toml"] {
            let file = format!("{}/../../catalog/{name}", env!("CARGO_MANIFEST_DIR"));
            let text = std::fs::read_to_string(file).unwrap();
            let document: toml_edit::DocumentMut = text.parse().unwrap();
            let mut catalog: CatalogFile =
                serde_json::from_value(table_json(document.as_table())).unwrap();
            catalog.resolve_groups();
            for command in catalog.commands {
                commands.insert(command.name.clone(), command);
            }
        }
        commands
    })
}

/// Catalog specification of `command`.
pub(super) fn spec(command: &str) -> StageSpec {
    catalog()
        .get(command)
        .cloned()
        .unwrap_or_else(|| panic!("{command} is not in the catalog"))
}

/// Value of a catalog default literal.
fn default_value(default: &DefaultValue) -> Value {
    let text = default.as_str().trim();
    match default.kind() {
        DefaultKind::Int => Value::Int(text.replace('_', "").parse().unwrap()),
        DefaultKind::Bool => Value::Bool(text == "true"),
        DefaultKind::Word => Value::Enum(text.to_owned()),
        other => panic!("default {text} of kind {other:?} has no test value"),
    }
}

/// Bound arguments as the binder passes them: `given` values, catalog
/// defaults for the other parameters, and absent values without default.
pub(super) fn args(spec: &StageSpec, given: &[(&str, Value)]) -> BoundArgs {
    let mut args = BoundArgs::new(spec.name.clone());
    for param in &spec.params {
        let value = given.iter().find(|(name, _)| *name == param.name);
        match (value, &param.default) {
            (Some((_, value)), _) => {
                args.insert(param.name.clone(), ArgValue::Value(value.clone()), false);
            }
            (None, Some(default)) => {
                args.insert(
                    param.name.clone(),
                    ArgValue::Value(default_value(default)),
                    true,
                );
            }
            (None, None) => args.insert(param.name.clone(), ArgValue::Absent, false),
        }
    }
    args
}

/// Bound arguments of `command` with `given` values and catalog defaults.
pub(super) fn call(command: &str, given: &[(&str, Value)]) -> BoundArgs {
    args(&spec(command), given)
}

/// Bound arguments without any value, not even the catalog defaults.
pub(super) fn absent(spec: &StageSpec) -> BoundArgs {
    let mut args = BoundArgs::new(spec.name.clone());
    for param in &spec.params {
        args.insert(param.name.clone(), ArgValue::Absent, false);
    }
    args
}

fn scalar_sample(scalar: ScalarType) -> Value {
    match scalar {
        ScalarType::Int => Value::Int(1),
        ScalarType::Bool => Value::Bool(true),
        ScalarType::Path => sample_path(),
        ScalarType::Duration => Value::Duration(value::Duration::from_millis(3_600_000)),
        ScalarType::Instant => {
            Value::Instant(value::Instant::from_unix_nanos(1_700_000_000_000_000_000))
        }
        ScalarType::Size => Value::Size(Size::from_bytes(GIB)),
        ScalarType::Percent => Value::Text("10%".to_owned()),
        other => panic!("no sample value of type {other}"),
    }
}

/// One valid value of every alternative of a parameter type.
pub(super) fn samples(ty: &ParamType) -> Vec<Value> {
    ty.alternatives()
        .iter()
        .flat_map(|alternative| match alternative {
            ParamType::Scalar(scalar) => vec![scalar_sample(*scalar)],
            ParamType::Enum(values) => values.iter().cloned().map(Value::Enum).collect(),
            ParamType::List(inner) => samples(inner)
                .into_iter()
                .map(|value| Value::List(vec![value]))
                .collect(),
            other => panic!("no sample value of type {other}"),
        })
        .collect()
}
