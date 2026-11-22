// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! WMI queries through COM.
//!
//! The query runs on the calling thread inside a multithreaded apartment
//! that is entered and left by [`Apartment`]. Results are read one object at
//! a time with a bounded wait, so cancellation and the deadline of the
//! program are observed while WMI is still producing results. All interface
//! pointers are released before the apartment is left.

use std::ffi::c_void;

use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::record::CimObject;
use nurt_contracts::stage::ExecCtx;
use windows::Win32::Foundation::{RPC_E_CHANGED_MODE, RPC_E_TOO_LATE, VARIANT_BOOL};
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx,
    CoInitializeSecurity, CoSetProxyBlanket, CoUninitialize, EOAC_NONE, RPC_C_AUTHN_LEVEL_CALL,
    RPC_C_AUTHN_LEVEL_DEFAULT, RPC_C_IMP_LEVEL_IMPERSONATE, SAFEARRAY,
};
use windows::Win32::System::Ole::{
    SafeArrayGetDim, SafeArrayGetElement, SafeArrayGetLBound, SafeArrayGetUBound,
};
use windows::Win32::System::Variant::{
    VARENUM, VARIANT, VT_ARRAY, VT_BOOL, VT_BSTR, VT_DISPATCH, VT_I1, VT_I2, VT_I4, VT_I8, VT_INT,
    VT_R4, VT_R8, VT_UI1, VT_UI2, VT_UI4, VT_UI8, VT_UINT, VT_UNKNOWN, VT_VARIANT, VariantClear,
};
use windows::Win32::System::Wmi::{
    IEnumWbemClassObject, IWbemClassObject, IWbemLocator, IWbemServices, WBEM_FLAG_FORWARD_ONLY,
    WBEM_FLAG_NONSYSTEM_ONLY, WBEM_FLAG_RETURN_IMMEDIATELY, WBEM_GENERIC_FLAG_TYPE,
    WBEM_S_TIMEDOUT, WbemLocator,
};
use windows::core::{BSTR, HRESULT, HSTRING, IUnknown, Interface, PCWSTR};

use super::convert::{self, MAX_DEPTH, RawObject, RawProperty, RawValue};
use super::wql::CimQuery;
use super::{status_error, too_many};

/// Longest wait for one result object, in milliseconds, before the
/// cancellation token and the deadline are checked again.
const NEXT_TIMEOUT_MS: i32 = 250;

/// `RPC_C_AUTHN_WINNT` (`rpcdce.h`): NTLM authentication service.
const RPC_C_AUTHN_WINNT: u32 = 10;

/// `RPC_C_AUTHZ_NONE` (`rpcdce.h`): no authorization service.
const RPC_C_AUTHZ_NONE: u32 = 0;

/// Query language name passed to `IWbemServices::ExecQuery`.
const QUERY_LANGUAGE: &str = "WQL";

/// System property with the class of an object.
const CLASS_PROPERTY: &str = "__CLASS";

/// System property with the full path of an object.
const PATH_PROPERTY: &str = "__PATH";

/// Membership of the calling thread in a COM apartment.
///
/// The apartment is left on drop when this value entered it. A thread that
/// is already in a single-threaded apartment (`RPC_E_CHANGED_MODE`) keeps
/// it; WMI proxies work from either kind of apartment.
struct Apartment {
    entered: bool,
}

impl Apartment {
    fn enter() -> Result<Self> {
        // SAFETY: the reserved argument is `None` as required; the call has
        // no other preconditions.
        let status = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
        if status == RPC_E_CHANGED_MODE {
            return Ok(Self { entered: false });
        }
        status
            .ok()
            .map_err(|error| com_error("CoInitializeEx", &error))?;
        Ok(Self { entered: true })
    }
}

impl Drop for Apartment {
    fn drop(&mut self) {
        if self.entered {
            // SAFETY: balances the successful `CoInitializeEx` of `enter`
            // on the same thread; every interface pointer obtained inside
            // the apartment is dropped before this value.
            unsafe { CoUninitialize() };
        }
    }
}

/// `VARIANT` cleared on drop.
struct OwnedVariant(VARIANT);

impl OwnedVariant {
    fn new() -> Self {
        Self(VARIANT::default())
    }

    fn vt(&self) -> VARENUM {
        // SAFETY: every `VARIANT` starts with the initialized `vt` tag; the
        // default value is `VT_EMPTY`.
        unsafe { self.0.Anonymous.Anonymous.vt }
    }
}

impl Drop for OwnedVariant {
    fn drop(&mut self) {
        // SAFETY: the variant is either `VT_EMPTY` or was filled by a COM
        // call that transferred ownership of its content to the caller.
        // A failure leaves nothing that could be released here.
        let _cleared = unsafe { VariantClear(&raw mut self.0) };
    }
}

fn com_error(call: &str, error: &windows::core::Error) -> Error {
    let code = error.code().0;
    Error::new(
        ErrorCode::Io,
        format!(
            "{call} failed with 0x{:08X}: {}",
            code.cast_unsigned(),
            error.message()
        ),
    )
    .with_detail(
        "os_error",
        serde_json::Value::from(format!("0x{:08X}", code.cast_unsigned())),
    )
}

fn wmi_error(error: &windows::core::Error, query: &CimQuery) -> Error {
    status_error(error.code().0, &error.message(), query)
}

/// Sets the default security of the process unless the embedding
/// application already did (`RPC_E_TOO_LATE`).
fn initialize_security() -> Result<()> {
    // SAFETY: all pointer arguments are absent; the levels are valid
    // constants of the RPC run time.
    let result = unsafe {
        CoInitializeSecurity(
            None,
            -1,
            None,
            None,
            RPC_C_AUTHN_LEVEL_DEFAULT,
            RPC_C_IMP_LEVEL_IMPERSONATE,
            None,
            EOAC_NONE,
            None,
        )
    };
    match result {
        Err(error) if error.code() != RPC_E_TOO_LATE => {
            Err(com_error("CoInitializeSecurity", &error))
        }
        _ => Ok(()),
    }
}

fn connect(query: &CimQuery) -> Result<IWbemServices> {
    // SAFETY: `WbemLocator` is the class identifier of the WMI locator and
    // the requested interface matches the type parameter.
    let locator: IWbemLocator =
        unsafe { CoCreateInstance(&WbemLocator, None, CLSCTX_INPROC_SERVER) }
            .map_err(|error| com_error("CoCreateInstance", &error))?;
    let namespace = BSTR::from(query.wmi_namespace());
    let empty = BSTR::new();
    // SAFETY: every string argument is a valid `BSTR` that outlives the
    // call; empty strings select the current user and locale.
    let services =
        unsafe { locator.ConnectServer(&namespace, &empty, &empty, &empty, 0, &empty, None) }
            .map_err(|error| wmi_error(&error, query))?;
    // SAFETY: `services` is a valid proxy returned by `ConnectServer`; the
    // principal name and identity are absent, so the defaults apply.
    unsafe {
        CoSetProxyBlanket(
            &services,
            RPC_C_AUTHN_WINNT,
            RPC_C_AUTHZ_NONE,
            PCWSTR::null(),
            RPC_C_AUTHN_LEVEL_CALL,
            RPC_C_IMP_LEVEL_IMPERSONATE,
            None,
            EOAC_NONE,
        )
    }
    .map_err(|error| com_error("CoSetProxyBlanket", &error))?;
    Ok(services)
}

fn execute(services: &IWbemServices, query: &CimQuery) -> Result<IEnumWbemClassObject> {
    let language = BSTR::from(QUERY_LANGUAGE);
    let statement = BSTR::from(query.wql());
    let flags = WBEM_GENERIC_FLAG_TYPE(WBEM_FLAG_FORWARD_ONLY.0 | WBEM_FLAG_RETURN_IMMEDIATELY.0);
    // SAFETY: both strings are valid `BSTR` values that outlive the call and
    // no context object is passed.
    unsafe { services.ExecQuery(&language, &statement, flags, None) }
        .map_err(|error| wmi_error(&error, query))
}

/// One step of the result enumeration.
enum Step {
    Object(IWbemClassObject),
    Pending,
    Done,
}

fn next(enumerator: &IEnumWbemClassObject, query: &CimQuery) -> Result<Step> {
    let mut slot: [Option<IWbemClassObject>; 1] = [None];
    let mut returned = 0_u32;
    // SAFETY: `slot` has room for the one object requested and `returned`
    // is valid for writes; a returned object is owned by `slot`.
    let status = unsafe { enumerator.Next(NEXT_TIMEOUT_MS, &mut slot, &raw mut returned) };
    if status.is_err() {
        return Err(wmi_error(
            &windows::core::Error::from_hresult(status),
            query,
        ));
    }
    match slot[0].take() {
        Some(object) if returned > 0 => Ok(Step::Object(object)),
        _ if status == HRESULT(WBEM_S_TIMEDOUT.0) => Ok(Step::Pending),
        _ => Ok(Step::Done),
    }
}

fn get(object: &IWbemClassObject, name: &str, depth: usize) -> Result<(RawValue, i32)> {
    let wide = HSTRING::from(name);
    let mut variant = OwnedVariant::new();
    let mut cim_type = 0_i32;
    // SAFETY: `wide` is a null-terminated UTF-16 string that outlives the
    // call; `variant` and `cim_type` are valid for writes and the variant
    // content is released by `OwnedVariant`.
    unsafe {
        object.Get(
            PCWSTR(wide.as_ptr()),
            0,
            &raw mut variant.0,
            Some(&raw mut cim_type),
            None,
        )
    }
    .map_err(|error| com_error("IWbemClassObject::Get", &error))?;
    Ok((read_variant(&variant, depth)?, cim_type))
}

fn text_property(object: &IWbemClassObject, name: &str) -> Result<Option<String>> {
    match get(object, name, MAX_DEPTH)?.0 {
        RawValue::Text(text) => Ok(Some(text)),
        _ => Ok(None),
    }
}

fn all_properties(object: &IWbemClassObject, depth: usize) -> Result<Vec<RawProperty>> {
    // SAFETY: `object` is a valid class object; the enumeration is ended
    // below on every path that started it.
    unsafe { object.BeginEnumeration(WBEM_FLAG_NONSYSTEM_ONLY.0) }
        .map_err(|error| com_error("IWbemClassObject::BeginEnumeration", &error))?;
    let properties = enumerate(object, depth);
    // SAFETY: ends the enumeration started above on the same object.
    let ended = unsafe { object.EndEnumeration() };
    let properties = properties?;
    ended.map_err(|error| com_error("IWbemClassObject::EndEnumeration", &error))?;
    Ok(properties)
}

fn enumerate(object: &IWbemClassObject, depth: usize) -> Result<Vec<RawProperty>> {
    let mut properties = Vec::new();
    loop {
        let mut name = BSTR::new();
        let mut variant = OwnedVariant::new();
        let mut cim_type = 0_i32;
        let mut flavor = 0_i32;
        // SAFETY: all output arguments are valid for writes; the name and
        // the variant are owned by `BSTR` and `OwnedVariant`. At the end of
        // the enumeration the call succeeds with `WBEM_S_NO_MORE_DATA` and
        // leaves the name empty.
        unsafe {
            object.Next(
                0,
                &raw mut name,
                &raw mut variant.0,
                &raw mut cim_type,
                &raw mut flavor,
            )
        }
        .map_err(|error| com_error("IWbemClassObject::Next", &error))?;
        if name.is_empty() {
            return Ok(properties);
        }
        properties.push(RawProperty {
            name: name.to_string(),
            cim_type,
            value: read_variant(&variant, depth)?,
        });
    }
}

fn read_object(object: &IWbemClassObject, query: &CimQuery) -> Result<CimObject> {
    let class = text_property(object, CLASS_PROPERTY)?.unwrap_or_default();
    let path = text_property(object, PATH_PROPERTY)?.unwrap_or_default();
    let properties = if query.properties.is_empty() {
        all_properties(object, 0)?
    } else {
        query
            .properties
            .iter()
            .map(|name| {
                get(object, name, 0).map(|(value, cim_type)| RawProperty {
                    name: name.clone(),
                    cim_type,
                    value,
                })
            })
            .collect::<Result<Vec<_>>>()?
    };
    Ok(CimObject {
        class,
        path,
        properties: convert::fields(properties, 0),
    })
}

fn embedded(unknown: Option<&IUnknown>, depth: usize) -> Result<RawValue> {
    let Some(unknown) = unknown else {
        return Ok(RawValue::Null);
    };
    if depth >= MAX_DEPTH {
        return Ok(RawValue::Null);
    }
    let Ok(object) = unknown.cast::<IWbemClassObject>() else {
        return Ok(RawValue::Null);
    };
    Ok(RawValue::Object(RawObject {
        class: text_property(&object, CLASS_PROPERTY)?,
        properties: all_properties(&object, depth + 1)?,
    }))
}

fn read_variant(variant: &OwnedVariant, depth: usize) -> Result<RawValue> {
    let vt = variant.vt();
    // SAFETY: the union member read in each arm is the one selected by the
    // `vt` tag; pointers and interfaces are borrowed, not taken. Empty,
    // null and unsupported values are read as null.
    let value = unsafe {
        let data = &variant.0.Anonymous.Anonymous.Anonymous;
        if vt.0 & VT_ARRAY.0 != 0 {
            return read_array(data.parray, VARENUM(vt.0 & !VT_ARRAY.0), depth);
        }
        match vt {
            VT_BOOL => RawValue::Bool(data.boolVal.0 != 0),
            VT_I1 => RawValue::Signed(i64::from(data.cVal)),
            VT_I2 => RawValue::Signed(i64::from(data.iVal)),
            VT_I4 => RawValue::Signed(i64::from(data.lVal)),
            VT_INT => RawValue::Signed(i64::from(data.intVal)),
            VT_I8 => RawValue::Signed(data.llVal),
            VT_UI1 => RawValue::Unsigned(u64::from(data.bVal)),
            VT_UI2 => RawValue::Unsigned(u64::from(data.uiVal)),
            VT_UI4 => RawValue::Unsigned(u64::from(data.ulVal)),
            VT_UINT => RawValue::Unsigned(u64::from(data.uintVal)),
            VT_UI8 => RawValue::Unsigned(data.ullVal),
            VT_R4 => RawValue::Real(f64::from(data.fltVal)),
            VT_R8 => RawValue::Real(data.dblVal),
            VT_BSTR => RawValue::Text(data.bstrVal.to_string()),
            VT_UNKNOWN => return embedded(data.punkVal.as_ref(), depth),
            VT_DISPATCH => {
                let unknown = data
                    .pdispVal
                    .as_ref()
                    .and_then(|dispatch| dispatch.cast::<IUnknown>().ok());
                return embedded(unknown.as_ref(), depth);
            }
            _ => RawValue::Null,
        }
    };
    Ok(value)
}

/// Reads element `index` of a one-dimensional array into `T`.
///
/// # Safety
///
/// `array` must be a valid array whose elements have the layout of `T`,
/// and `T` must take ownership of the copy made by `SafeArrayGetElement`
/// (`BSTR`, interfaces and `VARIANT` release it on drop).
unsafe fn element<T: Default>(array: *const SAFEARRAY, index: i32) -> Result<T> {
    let mut value = T::default();
    // SAFETY: guaranteed by the caller; `value` is valid for writes of `T`.
    unsafe { SafeArrayGetElement(array, &raw const index, (&raw mut value).cast::<c_void>()) }
        .map_err(|error| com_error("SafeArrayGetElement", &error))?;
    Ok(value)
}

fn read_array(array: *mut SAFEARRAY, element_type: VARENUM, depth: usize) -> Result<RawValue> {
    if array.is_null() {
        return Ok(RawValue::Null);
    }
    // SAFETY: `array` is the non-null array of a `VT_ARRAY` variant that
    // stays alive while the variant is borrowed.
    let (dimensions, bounds) = unsafe {
        let dimensions = SafeArrayGetDim(array);
        let lower = SafeArrayGetLBound(array, 1);
        let upper = SafeArrayGetUBound(array, 1);
        (
            dimensions,
            lower.and_then(|lower| upper.map(|upper| (lower, upper))),
        )
    };
    if dimensions != 1 {
        return Ok(RawValue::Null);
    }
    let (lower, upper) = bounds.map_err(|error| com_error("SafeArrayGetLBound", &error))?;
    let mut items = Vec::new();
    for index in lower..=upper {
        // SAFETY: the element type of each arm is the layout of the array
        // elements given by the variant tag; owned copies are released by
        // the Rust types.
        let item = unsafe {
            match element_type {
                VT_BSTR => RawValue::Text(element::<BSTR>(array, index)?.to_string()),
                VT_BOOL => RawValue::Bool(element::<VARIANT_BOOL>(array, index)?.0 != 0),
                VT_I1 => RawValue::Signed(i64::from(element::<i8>(array, index)?)),
                VT_I2 => RawValue::Signed(i64::from(element::<i16>(array, index)?)),
                VT_I4 | VT_INT => RawValue::Signed(i64::from(element::<i32>(array, index)?)),
                VT_I8 => RawValue::Signed(element::<i64>(array, index)?),
                VT_UI1 => RawValue::Unsigned(u64::from(element::<u8>(array, index)?)),
                VT_UI2 => RawValue::Unsigned(u64::from(element::<u16>(array, index)?)),
                VT_UI4 | VT_UINT => RawValue::Unsigned(u64::from(element::<u32>(array, index)?)),
                VT_UI8 => RawValue::Unsigned(element::<u64>(array, index)?),
                VT_R4 => RawValue::Real(f64::from(element::<f32>(array, index)?)),
                VT_R8 => RawValue::Real(element::<f64>(array, index)?),
                VT_UNKNOWN => embedded(element::<Option<IUnknown>>(array, index)?.as_ref(), depth)?,
                VT_VARIANT => {
                    let variant = OwnedVariant(element::<VARIANT>(array, index)?);
                    read_variant(&variant, depth)?
                }
                _ => RawValue::Null,
            }
        };
        items.push(item);
    }
    Ok(RawValue::Array(items))
}

/// Runs `query` and reads at most `take` objects.
///
/// # Errors
///
/// Returns `E_NOT_FOUND` for an unknown class or namespace, `E_INVALID_VALUE` for a
/// query rejected by WMI, `E_PRIVILEGE` when access is denied, `E_LIMIT`
/// when more than `max` objects are returned, `E_CANCELLED` and `E_TIMEOUT`
/// from the execution context, and `E_IO` for other COM failures.
pub(super) fn query(
    query: &CimQuery,
    ctx: &ExecCtx,
    take: Option<u64>,
    max: u64,
) -> Result<Vec<CimObject>> {
    let _apartment = Apartment::enter()?;
    initialize_security()?;
    let services = connect(query)?;
    let enumerator = execute(&services, query)?;
    let mut objects: Vec<CimObject> = Vec::new();
    loop {
        let read = u64::try_from(objects.len()).unwrap_or(u64::MAX);
        if take.is_some_and(|take| read >= take) {
            break;
        }
        ctx.checkpoint()?;
        match next(&enumerator, query)? {
            Step::Object(object) => {
                if read >= max {
                    return Err(too_many(max));
                }
                objects.push(read_object(&object, query)?);
            }
            Step::Pending => {}
            Step::Done => break,
        }
    }
    Ok(objects)
}
