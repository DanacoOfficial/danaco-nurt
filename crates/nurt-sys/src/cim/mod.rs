// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! The `windows.cim` command: instances of CIM classes read through WMI.
//!
//! The command builds a WQL data query from its parameters, runs it against
//! the local WMI service through COM and returns one `cim_object` record per
//! instance: the class, the object path and the requested properties
//! converted to language values. On other operating systems the command
//! fails with `E_OS_ONLY`.
//!
//! When `properties` leaves out the key properties of the class, WMI does
//! not report the object path and the `path` field is empty.

#[cfg(any(windows, test))]
mod convert;
#[cfg(any(windows, test))]
mod datetime;
mod wql;

#[cfg(windows)]
mod com;

use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::stage::{
    BoundArgs, CheckCtx, ExecCtx, InputKinds, ReadStage, StageDescriptor, StageImpl, StageInput,
    StageOutput, StaticInfo,
};
use nurt_contracts::value::Value;

use self::wql::CimQuery;

/// Name of the command in the catalog.
pub const COMMAND: &str = "windows.cim";

const PARAM_CLASS: &str = "class";
const PARAM_WHERE: &str = "where";
const PARAM_NAMESPACE: &str = "namespace";
const PARAM_PROPERTIES: &str = "properties";

/// Namespace used when the parameter is absent, as in the catalog.
const DEFAULT_NAMESPACE: &str = "root/cimv2";

/// Implementation of the `windows.cim` command.
///
/// The static check validates the class, the property names, the namespace
/// and the condition on every system, so a malformed query is reported
/// before execution; the query itself runs only on Windows.
///
/// ```
/// use nurt_contracts::stage::StageImpl;
/// use nurt_sys::cim::WindowsCim;
///
/// assert_eq!(WindowsCim.descriptor().command, "windows.cim");
/// ```
#[derive(Debug, Clone, Copy, Default)]
pub struct WindowsCim;

impl WindowsCim {
    /// Creates the command implementation.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

fn query_from_args(args: &BoundArgs) -> Result<CimQuery> {
    args.require(PARAM_CLASS)?;
    let class = args.text(PARAM_CLASS)?.unwrap_or_default();
    let condition = args.text(PARAM_WHERE)?;
    let namespace = args.text(PARAM_NAMESPACE)?.unwrap_or(DEFAULT_NAMESPACE);
    let properties = match args.value(PARAM_PROPERTIES) {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::List(items)) => items
            .iter()
            .map(|item| match item {
                Value::Text(name) => Ok(name.clone()),
                other => Err(Error::new(
                    ErrorCode::Type,
                    format!(
                        "`windows.cim` expects text items in `properties`, not {}",
                        other.type_name()
                    ),
                )),
            })
            .collect::<Result<Vec<_>>>()?,
        Some(other) => {
            return Err(Error::new(
                ErrorCode::Type,
                format!(
                    "`windows.cim` expects list<text> for `properties`, not {}",
                    other.type_name()
                ),
            ));
        }
    };
    CimQuery::new(class, condition, namespace, &properties)
}

/// `E_OS_ONLY` for a run outside Windows.
#[cfg(not(windows))]
fn os_only() -> Error {
    Error::new(
        ErrorCode::OsOnly,
        "`windows.cim` is available only on Windows; use `host` or `processes` on this system",
    )
    .with_detail("os", serde_json::json!(["windows"]))
}

/// Number of records used by the rest of the pipeline, when limited, and the
/// number of records a stage may hold.
#[cfg(any(windows, test))]
fn read_budget(ctx: &ExecCtx) -> (Option<u64>, u64) {
    (ctx.demand().limit, ctx.limits().max_records)
}

/// `E_LIMIT` when the query returns more instances than a stage may hold.
#[cfg(any(windows, test))]
fn too_many(max: u64) -> Error {
    Error::new(
        ErrorCode::Limit,
        format!("the query returns more than {max} instances; narrow it with `where`"),
    )
}

/// `HRESULT` values of WMI (`WBEMSTATUS`, `wbemcli.h`) and COM that map to
/// specific error codes.
#[cfg(any(windows, test))]
mod status {
    /// `WBEM_E_ACCESS_DENIED`.
    pub(super) const WBEM_E_ACCESS_DENIED: i32 = 0x8004_1003_u32.cast_signed();
    /// `WBEM_E_INVALID_NAMESPACE`.
    pub(super) const WBEM_E_INVALID_NAMESPACE: i32 = 0x8004_100E_u32.cast_signed();
    /// `WBEM_E_INVALID_CLASS`.
    pub(super) const WBEM_E_INVALID_CLASS: i32 = 0x8004_1010_u32.cast_signed();
    /// `WBEM_E_NOT_FOUND`.
    pub(super) const WBEM_E_NOT_FOUND: i32 = 0x8004_1002_u32.cast_signed();
    /// `WBEM_E_INVALID_QUERY`.
    pub(super) const WBEM_E_INVALID_QUERY: i32 = 0x8004_1017_u32.cast_signed();
    /// `WBEM_E_INVALID_PROPERTY`.
    pub(super) const WBEM_E_INVALID_PROPERTY: i32 = 0x8004_1031_u32.cast_signed();
    /// `E_ACCESSDENIED`.
    pub(super) const E_ACCESSDENIED: i32 = 0x8007_0005_u32.cast_signed();
}

/// Maps a failed `HRESULT` of WMI to an error of the language.
#[cfg(any(windows, test))]
fn status_error(code: i32, os_message: &str, query: &CimQuery) -> Error {
    let os_error = format!("0x{:08X}", code.cast_unsigned());
    let error = match code {
        status::WBEM_E_INVALID_CLASS | status::WBEM_E_NOT_FOUND => Error::new(
            ErrorCode::NotFound,
            format!(
                "no CIM class `{}` in namespace `{}`",
                query.class, query.namespace
            ),
        )
        .with_found(query.class.clone()),
        status::WBEM_E_INVALID_NAMESPACE => Error::new(
            ErrorCode::NotFound,
            format!("no CIM namespace `{}`", query.namespace),
        )
        .with_found(query.namespace.clone()),
        status::WBEM_E_INVALID_QUERY | status::WBEM_E_INVALID_PROPERTY => Error::new(
            ErrorCode::InvalidValue,
            format!(
                "WMI rejected the query `{}`; check the property names and the `where` condition",
                query.wql()
            ),
        )
        .with_found(query.wql()),
        status::WBEM_E_ACCESS_DENIED | status::E_ACCESSDENIED => Error::new(
            ErrorCode::Privilege,
            format!(
                "reading namespace `{}` requires the `administrator` privilege",
                query.namespace
            ),
        )
        .with_detail("required", serde_json::Value::from("administrator")),
        _ => Error::new(
            ErrorCode::Io,
            format!("WMI query failed with {os_error}: {}", os_message.trim()),
        ),
    };
    error
        .with_detail("os_error", serde_json::Value::from(os_error))
        .with_detail("os_message", serde_json::Value::from(os_message.trim()))
}

#[cfg(windows)]
fn run(query: &CimQuery, ctx: &ExecCtx) -> Result<Vec<nurt_contracts::value::Record>> {
    let (take, max) = read_budget(ctx);
    let objects = com::query(query, ctx, take, max)?;
    Ok(objects
        .into_iter()
        .map(nurt_contracts::value::Record::from)
        .collect())
}

#[cfg(not(windows))]
fn run(_query: &CimQuery, _ctx: &ExecCtx) -> Result<Vec<nurt_contracts::value::Record>> {
    Err(os_only())
}

impl StageImpl for WindowsCim {
    fn descriptor(&self) -> StageDescriptor {
        StageDescriptor {
            command: COMMAND.to_owned(),
            source: true,
            input: InputKinds::None,
        }
    }

    fn check(&self, args: &BoundArgs, _ctx: &CheckCtx<'_>) -> Result<StaticInfo> {
        query_from_args(args)?;
        Ok(StaticInfo::default())
    }

    fn as_read(&self) -> Option<&dyn ReadStage> {
        Some(self)
    }
}

impl ReadStage for WindowsCim {
    fn execute<'a>(
        &'a self,
        args: &'a BoundArgs,
        _input: StageInput<'a>,
        ctx: &'a ExecCtx,
    ) -> Result<StageOutput<'a>> {
        let query = query_from_args(args)?;
        let records = run(&query, ctx)?;
        Ok(StageOutput::records(
            nurt_contracts::stage::RecordStream::from_records(records),
        ))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use nurt_contracts::config::Config;
    use nurt_contracts::env::{BootId, Clock, ClockStamp, Host, HostInfo, Os, PathStyle, WallTime};
    use nurt_contracts::spec::{Os as SpecOs, StageSpec};
    use nurt_contracts::stage::{ArgValue, CancelReason, CancelToken, Demand, Limits, StageFrame};

    use super::*;

    struct FixedClock;

    impl Clock for FixedClock {
        fn now(&self) -> WallTime {
            WallTime::UNIX_EPOCH
        }

        fn monotonic(&self) -> Duration {
            Duration::ZERO
        }

        fn stamp(&self) -> ClockStamp {
            ClockStamp {
                wall: WallTime::UNIX_EPOCH,
                boot: BootId("boot".to_owned()),
                since_boot_with_suspend: Duration::ZERO,
                since_boot_active: Duration::ZERO,
            }
        }

        fn sleep(&self, _duration: Duration, cancel: &CancelToken) -> Result<(), CancelReason> {
            cancel.reason().map_or(Ok(()), Err)
        }
    }

    struct TestHost(HostInfo);

    impl Host for TestHost {
        fn info(&self) -> &HostInfo {
            &self.0
        }

        fn env_var(&self, _name: &str) -> Option<std::ffi::OsString> {
            None
        }

        fn env_vars(&self) -> Vec<(std::ffi::OsString, std::ffi::OsString)> {
            Vec::new()
        }
    }

    fn host() -> Arc<TestHost> {
        Arc::new(TestHost(HostInfo {
            os: Os::Linux,
            os_version: "6.8.0".to_owned(),
            architecture: "x86_64".to_owned(),
            hostname: "host".to_owned(),
            user: "user".to_owned(),
            home: "/home/user".into(),
            temp: "/tmp".into(),
            path_style: PathStyle::Posix,
            timezone: "UTC".to_owned(),
            locale: None,
            cpus: 1,
        }))
    }

    fn ctx() -> ExecCtx {
        ExecCtx::builder(Arc::new(FixedClock), host()).build()
    }

    fn text(value: &str) -> ArgValue {
        ArgValue::Value(Value::Text(value.to_owned()))
    }

    fn args(class: &str) -> BoundArgs {
        BoundArgs::new(COMMAND)
            .with(PARAM_CLASS, text(class), false)
            .with(PARAM_WHERE, ArgValue::Absent, false)
            .with(PARAM_NAMESPACE, text(DEFAULT_NAMESPACE), true)
            .with(PARAM_PROPERTIES, ArgValue::Absent, false)
    }

    fn frozen_entry() -> toml::Value {
        let path = format!(
            "{}/../../catalog/admin.toml",
            std::env::var("CARGO_MANIFEST_DIR")
                .as_deref()
                .unwrap_or(env!("CARGO_MANIFEST_DIR"))
        );
        let text = std::fs::read_to_string(path).unwrap();
        let catalog: toml::Table = text.parse().unwrap();
        catalog["command"]
            .as_array()
            .unwrap()
            .iter()
            .find(|command| command["name"].as_str() == Some(COMMAND))
            .unwrap()
            .clone()
    }

    fn frozen_spec() -> StageSpec {
        frozen_entry().try_into().unwrap()
    }

    #[test]
    fn catalog_signatures_match_frozen() {
        let spec = frozen_spec();
        assert_eq!(spec.os, [SpecOs::Windows]);
        assert!(spec.external.is_none());
        let names: Vec<&str> = spec
            .params
            .iter()
            .map(|param| param.name.as_str())
            .collect();
        assert_eq!(
            names,
            [PARAM_CLASS, PARAM_WHERE, PARAM_NAMESPACE, PARAM_PROPERTIES]
        );
        let entry = frozen_entry();
        let params = entry["param"].as_array().unwrap();
        assert_eq!(params[0]["type"].as_str(), Some("text"));
        assert_eq!(params[0]["position"].as_integer(), Some(1));
        assert_eq!(params[0]["required"].as_bool(), Some(true));
        assert_eq!(params[1]["type"].as_str(), Some("text"));
        assert_eq!(params[2]["type"].as_str(), Some("text"));
        assert_eq!(
            params[2]["default"].as_str(),
            Some(format!("\"{DEFAULT_NAMESPACE}\"").as_str())
        );
        assert_eq!(params[3]["type"].as_str(), Some("list<text>"));
        assert_eq!(
            entry["output"].as_array().unwrap()[0].as_str(),
            Some("cim_object")
        );
        assert_eq!(entry["input"].as_array().unwrap()[0].as_str(), Some("none"));
        let descriptor = WindowsCim::new().descriptor();
        assert!(descriptor.source);
        assert_eq!(descriptor.input, InputKinds::None);
    }

    #[test]
    fn cim_check_validates_query_on_every_system() {
        let host = host();
        let config = Config::default();
        let spec = frozen_spec();
        let check = CheckCtx::new(&spec, host.as_ref(), &config);
        assert!(WindowsCim.check(&args("Win32_Service"), &check).is_ok());
        let error = WindowsCim
            .check(&args("Win32_Service WHERE 1=1"), &check)
            .unwrap_err();
        assert_eq!(error.code(), ErrorCode::InvalidValue);

        let listed = args("Win32_Service").with(
            PARAM_PROPERTIES,
            ArgValue::Value(Value::List(vec![
                Value::Text("Name".to_owned()),
                Value::Text("State".to_owned()),
            ])),
            false,
        );
        let query = query_from_args(&listed).unwrap();
        assert_eq!(query.wql(), "SELECT Name, State FROM Win32_Service");

        let wrong_item = args("Win32_Service").with(
            PARAM_PROPERTIES,
            ArgValue::Value(Value::List(vec![Value::Int(1)])),
            false,
        );
        assert_eq!(
            query_from_args(&wrong_item).unwrap_err().code(),
            ErrorCode::Type
        );
        let wrong_list = args("Win32_Service").with(PARAM_PROPERTIES, text("Name"), false);
        assert_eq!(
            query_from_args(&wrong_list).unwrap_err().code(),
            ErrorCode::Type
        );
        let missing = BoundArgs::new(COMMAND).with(PARAM_CLASS, ArgValue::Absent, false);
        assert_eq!(
            query_from_args(&missing).unwrap_err().code(),
            ErrorCode::MissingParam
        );
        let defaulted = BoundArgs::new(COMMAND).with(PARAM_CLASS, text("Win32_Bios"), false);
        assert_eq!(
            query_from_args(&defaulted).unwrap().namespace,
            DEFAULT_NAMESPACE
        );
    }

    #[cfg(not(windows))]
    #[test]
    fn cim_non_windows_os_only() {
        let ctx = ctx();
        let error = WindowsCim
            .execute(&args("Win32_OperatingSystem"), StageInput::none(), &ctx)
            .unwrap_err();
        assert_eq!(error.code(), ErrorCode::OsOnly);
        assert_eq!(error.details["os"], serde_json::json!(["windows"]));
        let invalid = WindowsCim
            .execute(&args("Win32 OperatingSystem"), StageInput::none(), &ctx)
            .unwrap_err();
        assert_eq!(invalid.code(), ErrorCode::InvalidValue);
    }

    #[test]
    fn cim_wmi_status_maps_to_error_codes() {
        let query = CimQuery::new("Win32_Nothing", Some("Name = 'x'"), "root/cimv2", &[]).unwrap();
        let cases = [
            (status::WBEM_E_INVALID_CLASS, ErrorCode::NotFound),
            (status::WBEM_E_NOT_FOUND, ErrorCode::NotFound),
            (status::WBEM_E_INVALID_NAMESPACE, ErrorCode::NotFound),
            (status::WBEM_E_INVALID_QUERY, ErrorCode::InvalidValue),
            (status::WBEM_E_INVALID_PROPERTY, ErrorCode::InvalidValue),
            (status::WBEM_E_ACCESS_DENIED, ErrorCode::Privilege),
            (status::E_ACCESSDENIED, ErrorCode::Privilege),
            (0x8004_1001_u32.cast_signed(), ErrorCode::Io),
        ];
        for (code, expected) in cases {
            let error = status_error(code, " message \r\n", &query);
            assert_eq!(error.code(), expected, "{code:#X}");
            assert_eq!(error.details["os_message"], "message");
        }
        let denied = status_error(status::WBEM_E_ACCESS_DENIED, "", &query);
        assert_eq!(denied.details["required"], "administrator");
        assert_eq!(
            status_error(0x8004_1001_u32.cast_signed(), "", &query).details["os_error"],
            "0x80041001"
        );
    }

    #[test]
    fn cim_read_budget_follows_demand_and_limits() {
        let limited = ExecCtx::builder(Arc::new(FixedClock), host())
            .limits(Limits {
                max_records: 10,
                ..Limits::default()
            })
            .frame(StageFrame {
                demand: Demand {
                    limit: Some(3),
                    ..Demand::default()
                },
                ..StageFrame::default()
            })
            .build();
        assert_eq!(read_budget(&limited), (Some(3), 10));
        assert_eq!(read_budget(&ctx()).0, None);
        assert_eq!(too_many(10).code(), ErrorCode::Limit);
    }

    #[cfg(windows)]
    fn objects(args: &BoundArgs) -> Result<Vec<nurt_contracts::record::CimObject>> {
        let ctx = ctx();
        let output = WindowsCim.execute(args, StageInput::none(), &ctx)?;
        let records = match output.data {
            nurt_contracts::stage::StageData::Records(stream) => {
                stream.collect::<Result<Vec<_>>>()?
            }
            _ => Vec::new(),
        };
        records
            .into_iter()
            .map(|record| {
                nurt_contracts::record::CimObject::try_from(record)
                    .map_err(|error| Error::new(ErrorCode::Type, format!("{error:?}")))
            })
            .collect()
    }

    #[cfg(windows)]
    #[test]
    #[ignore = "requires Windows Management Instrumentation"]
    fn it_cim_win32_operating_system() {
        let listed = args("Win32_OperatingSystem").with(
            PARAM_PROPERTIES,
            ArgValue::Value(Value::List(vec![
                Value::Text("Caption".to_owned()),
                Value::Text("LastBootUpTime".to_owned()),
            ])),
            false,
        );
        let found = objects(&listed).unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].class, "Win32_OperatingSystem");
        assert!(matches!(found[0].properties["Caption"], Value::Text(_)));
        assert!(matches!(
            found[0].properties["LastBootUpTime"],
            Value::Instant(_)
        ));
        let all = objects(&args("Win32_OperatingSystem")).unwrap();
        assert!(all[0].path.contains("Win32_OperatingSystem"));
    }

    #[cfg(windows)]
    #[test]
    #[ignore = "requires Windows Management Instrumentation"]
    fn it_cim_win32_service_where_name() {
        let filtered = args("Win32_Service").with(PARAM_WHERE, text("Name = 'Winmgmt'"), false);
        let found = objects(&filtered).unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(
            found[0].properties["Name"],
            Value::Text("Winmgmt".to_owned())
        );
        assert!(matches!(found[0].properties["ProcessId"], Value::Int(_)));
    }

    #[cfg(windows)]
    #[test]
    #[ignore = "requires Windows Management Instrumentation"]
    fn it_cim_invalid_class_not_found() {
        let error = objects(&args("Nurt_NoSuchClass")).unwrap_err();
        assert_eq!(error.code(), ErrorCode::NotFound);
        let namespace = args("Win32_Bios").with(PARAM_NAMESPACE, text("root/nurtmissing"), false);
        assert_eq!(objects(&namespace).unwrap_err().code(), ErrorCode::NotFound);
    }
}
