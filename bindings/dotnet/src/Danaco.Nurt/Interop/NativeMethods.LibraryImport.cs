// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl

using System;
using System.Runtime.CompilerServices;
using System.Runtime.InteropServices;

namespace Danaco.Nurt;

internal static unsafe partial class NativeMethods
{
    [LibraryImport(LibraryName, EntryPoint = "nurt_abi_version")]
    [UnmanagedCallConv(CallConvs = new[] { typeof(CallConvCdecl) })]
    internal static partial uint AbiVersion();

    [LibraryImport(LibraryName, EntryPoint = "nurt_abi_check")]
    [UnmanagedCallConv(CallConvs = new[] { typeof(CallConvCdecl) })]
    internal static partial CallStatus AbiCheck(uint headerVersion);

    [LibraryImport(LibraryName, EntryPoint = "nurt_engine_config_get")]
    [UnmanagedCallConv(CallConvs = new[] { typeof(CallConvCdecl) })]
    internal static partial CallStatus EngineConfigGet(EngineHandle engine, NativeBuffer* outJson);

    [LibraryImport(LibraryName, EntryPoint = "nurt_session_end")]
    [UnmanagedCallConv(CallConvs = new[] { typeof(CallConvCdecl) })]
    internal static partial CallStatus SessionEnd(EngineHandle engine, byte* session);

    [LibraryImport(LibraryName, EntryPoint = "nurt_ext_load")]
    [UnmanagedCallConv(CallConvs = new[] { typeof(CallConvCdecl) })]
    internal static partial CallStatus ExtLoad(EngineHandle engine, byte* manifestPath);

    [LibraryImport(LibraryName, EntryPoint = "nurt_ext_list")]
    [UnmanagedCallConv(CallConvs = new[] { typeof(CallConvCdecl) })]
    internal static partial CallStatus ExtList(EngineHandle engine, NativeBuffer* outJson);

    [LibraryImport(LibraryName, EntryPoint = "nurt_info")]
    [UnmanagedCallConv(CallConvs = new[] { typeof(CallConvCdecl) })]
    internal static partial CallStatus Info(NativeBuffer* outJson);

    [LibraryImport(LibraryName, EntryPoint = "nurt_engine_open")]
    [UnmanagedCallConv(CallConvs = new[] { typeof(CallConvCdecl) })]
    internal static partial CallStatus EngineOpen(byte* configJson, out EngineHandle engine);

    [LibraryImport(LibraryName, EntryPoint = "nurt_engine_close")]
    [UnmanagedCallConv(CallConvs = new[] { typeof(CallConvCdecl) })]
    internal static partial void EngineClose(IntPtr engine);

    [LibraryImport(LibraryName, EntryPoint = "nurt_parse")]
    [UnmanagedCallConv(CallConvs = new[] { typeof(CallConvCdecl) })]
    internal static partial CallStatus Parse(EngineHandle engine, byte* program, int variant, NativeBuffer* outAst, NativeBuffer* outDiagnostics);

    [LibraryImport(LibraryName, EntryPoint = "nurt_print")]
    [UnmanagedCallConv(CallConvs = new[] { typeof(CallConvCdecl) })]
    internal static partial CallStatus Print(EngineHandle engine, byte* ast, int variant, NativeBuffer* outText);

    [LibraryImport(LibraryName, EntryPoint = "nurt_correct")]
    [UnmanagedCallConv(CallConvs = new[] { typeof(CallConvCdecl) })]
    internal static partial CallStatus Correct(EngineHandle engine, byte* program, int variant, NativeBuffer* outResult);

    [LibraryImport(LibraryName, EntryPoint = "nurt_check")]
    [UnmanagedCallConv(CallConvs = new[] { typeof(CallConvCdecl) })]
    internal static partial CallStatus Check(EngineHandle engine, byte* request, NativeBuffer* outBound, NativeBuffer* outDiagnostics);

    [LibraryImport(LibraryName, EntryPoint = "nurt_run_json")]
    [UnmanagedCallConv(CallConvs = new[] { typeof(CallConvCdecl) })]
    internal static partial CallStatus RunJson(EngineHandle engine, byte* request, NativeBuffer* outResponse);

    [LibraryImport(LibraryName, EntryPoint = "nurt_exec_start")]
    [UnmanagedCallConv(CallConvs = new[] { typeof(CallConvCdecl) })]
    internal static partial CallStatus ExecStart(EngineHandle engine, byte* request, IntPtr callback, IntPtr user, out ExecHandle execution);

    [LibraryImport(LibraryName, EntryPoint = "nurt_exec_wait")]
    [UnmanagedCallConv(CallConvs = new[] { typeof(CallConvCdecl) })]
    internal static partial CallStatus ExecWait(ExecHandle execution, uint timeoutMilliseconds, NativeBuffer* outResponse);

    [LibraryImport(LibraryName, EntryPoint = "nurt_exec_cancel")]
    [UnmanagedCallConv(CallConvs = new[] { typeof(CallConvCdecl) })]
    internal static partial CallStatus ExecCancel(ExecHandle execution);

    [LibraryImport(LibraryName, EntryPoint = "nurt_exec_free")]
    [UnmanagedCallConv(CallConvs = new[] { typeof(CallConvCdecl) })]
    internal static partial void ExecFree(IntPtr execution);

    [LibraryImport(LibraryName, EntryPoint = "nurt_page")]
    [UnmanagedCallConv(CallConvs = new[] { typeof(CallConvCdecl) })]
    internal static partial CallStatus Page(EngineHandle engine, byte* cursor, uint limit, NativeBuffer* outResponse);

    [LibraryImport(LibraryName, EntryPoint = "nurt_catalog")]
    [UnmanagedCallConv(CallConvs = new[] { typeof(CallConvCdecl) })]
    internal static partial CallStatus Catalog(EngineHandle engine, byte* what, NativeBuffer* outJson);

    [LibraryImport(LibraryName, EntryPoint = "nurt_describe")]
    [UnmanagedCallConv(CallConvs = new[] { typeof(CallConvCdecl) })]
    internal static partial CallStatus Describe(EngineHandle engine, byte* what, NativeBuffer* outText);

    [LibraryImport(LibraryName, EntryPoint = "nurt_help")]
    [UnmanagedCallConv(CallConvs = new[] { typeof(CallConvCdecl) })]
    internal static partial CallStatus Help(EngineHandle engine, byte* topic, NativeBuffer* outText);

    [LibraryImport(LibraryName, EntryPoint = "nurt_schema")]
    [UnmanagedCallConv(CallConvs = new[] { typeof(CallConvCdecl) })]
    internal static partial CallStatus Schema(byte* which, NativeBuffer* outJson);

    [LibraryImport(LibraryName, EntryPoint = "nurt_history")]
    [UnmanagedCallConv(CallConvs = new[] { typeof(CallConvCdecl) })]
    internal static partial CallStatus History(EngineHandle engine, byte* query, NativeBuffer* outJson);

    [LibraryImport(LibraryName, EntryPoint = "nurt_space")]
    [UnmanagedCallConv(CallConvs = new[] { typeof(CallConvCdecl) })]
    internal static partial CallStatus Space(EngineHandle engine, byte* volume, NativeBuffer* outJson);

    [LibraryImport(LibraryName, EntryPoint = "nurt_buf_free")]
    [UnmanagedCallConv(CallConvs = new[] { typeof(CallConvCdecl) })]
    internal static partial void BufferFree(NativeBuffer* buffer);

    [LibraryImport(LibraryName, EntryPoint = "nurt_last_error")]
    [UnmanagedCallConv(CallConvs = new[] { typeof(CallConvCdecl) })]
    internal static partial CallStatus LastError(IntPtr engine, NativeBuffer* outJson);
}
