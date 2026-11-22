using System;
using System.Runtime.InteropServices;

namespace Danaco.Nurt;

internal static unsafe partial class NativeMethods
{
    /// <summary>Flag of <c>LoadLibraryExW</c> that resolves dependencies from the directory of the loaded file.</summary>
    internal const uint LoadWithAlteredSearchPath = 0x00000008;

    [DllImport(LibraryName, EntryPoint = "nurt_abi_version", CallingConvention = CallingConvention.Cdecl, ExactSpelling = true)]
    internal static extern uint AbiVersion();

    [DllImport(LibraryName, EntryPoint = "nurt_info", CallingConvention = CallingConvention.Cdecl, ExactSpelling = true)]
    internal static extern CallStatus Info(NativeBuffer* outJson);

    [DllImport(LibraryName, EntryPoint = "nurt_engine_open", CallingConvention = CallingConvention.Cdecl, ExactSpelling = true)]
    internal static extern CallStatus EngineOpen(byte* configJson, out EngineHandle engine);

    [DllImport(LibraryName, EntryPoint = "nurt_engine_close", CallingConvention = CallingConvention.Cdecl, ExactSpelling = true)]
    internal static extern void EngineClose(IntPtr engine);

    [DllImport(LibraryName, EntryPoint = "nurt_parse", CallingConvention = CallingConvention.Cdecl, ExactSpelling = true)]
    internal static extern CallStatus Parse(EngineHandle engine, byte* program, int variant, NativeBuffer* outAst, NativeBuffer* outDiagnostics);

    [DllImport(LibraryName, EntryPoint = "nurt_print", CallingConvention = CallingConvention.Cdecl, ExactSpelling = true)]
    internal static extern CallStatus Print(EngineHandle engine, byte* ast, int variant, NativeBuffer* outText);

    [DllImport(LibraryName, EntryPoint = "nurt_correct", CallingConvention = CallingConvention.Cdecl, ExactSpelling = true)]
    internal static extern CallStatus Correct(EngineHandle engine, byte* program, int variant, NativeBuffer* outResult);

    [DllImport(LibraryName, EntryPoint = "nurt_check", CallingConvention = CallingConvention.Cdecl, ExactSpelling = true)]
    internal static extern CallStatus Check(EngineHandle engine, byte* request, NativeBuffer* outBound, NativeBuffer* outDiagnostics);

    [DllImport(LibraryName, EntryPoint = "nurt_run_json", CallingConvention = CallingConvention.Cdecl, ExactSpelling = true)]
    internal static extern CallStatus RunJson(EngineHandle engine, byte* request, NativeBuffer* outResponse);

    [DllImport(LibraryName, EntryPoint = "nurt_exec_start", CallingConvention = CallingConvention.Cdecl, ExactSpelling = true)]
    internal static extern CallStatus ExecStart(EngineHandle engine, byte* request, IntPtr callback, IntPtr user, out ExecHandle execution);

    [DllImport(LibraryName, EntryPoint = "nurt_exec_wait", CallingConvention = CallingConvention.Cdecl, ExactSpelling = true)]
    internal static extern CallStatus ExecWait(ExecHandle execution, uint timeoutMilliseconds, NativeBuffer* outResponse);

    [DllImport(LibraryName, EntryPoint = "nurt_exec_cancel", CallingConvention = CallingConvention.Cdecl, ExactSpelling = true)]
    internal static extern CallStatus ExecCancel(ExecHandle execution);

    [DllImport(LibraryName, EntryPoint = "nurt_exec_free", CallingConvention = CallingConvention.Cdecl, ExactSpelling = true)]
    internal static extern void ExecFree(IntPtr execution);

    [DllImport(LibraryName, EntryPoint = "nurt_page", CallingConvention = CallingConvention.Cdecl, ExactSpelling = true)]
    internal static extern CallStatus Page(EngineHandle engine, byte* cursor, uint limit, NativeBuffer* outResponse);

    [DllImport(LibraryName, EntryPoint = "nurt_catalog", CallingConvention = CallingConvention.Cdecl, ExactSpelling = true)]
    internal static extern CallStatus Catalog(EngineHandle engine, byte* what, NativeBuffer* outJson);

    [DllImport(LibraryName, EntryPoint = "nurt_describe", CallingConvention = CallingConvention.Cdecl, ExactSpelling = true)]
    internal static extern CallStatus Describe(EngineHandle engine, byte* what, NativeBuffer* outText);

    [DllImport(LibraryName, EntryPoint = "nurt_help", CallingConvention = CallingConvention.Cdecl, ExactSpelling = true)]
    internal static extern CallStatus Help(EngineHandle engine, byte* topic, NativeBuffer* outText);

    [DllImport(LibraryName, EntryPoint = "nurt_schema", CallingConvention = CallingConvention.Cdecl, ExactSpelling = true)]
    internal static extern CallStatus Schema(byte* which, NativeBuffer* outJson);

    [DllImport(LibraryName, EntryPoint = "nurt_history", CallingConvention = CallingConvention.Cdecl, ExactSpelling = true)]
    internal static extern CallStatus History(EngineHandle engine, byte* query, NativeBuffer* outJson);

    [DllImport(LibraryName, EntryPoint = "nurt_space", CallingConvention = CallingConvention.Cdecl, ExactSpelling = true)]
    internal static extern CallStatus Space(EngineHandle engine, byte* volume, NativeBuffer* outJson);

    [DllImport(LibraryName, EntryPoint = "nurt_buf_free", CallingConvention = CallingConvention.Cdecl, ExactSpelling = true)]
    internal static extern void BufferFree(NativeBuffer* buffer);

    [DllImport(LibraryName, EntryPoint = "nurt_last_error", CallingConvention = CallingConvention.Cdecl, ExactSpelling = true)]
    internal static extern CallStatus LastError(IntPtr engine, NativeBuffer* outJson);

    [DllImport("kernel32", EntryPoint = "LoadLibraryExW", CharSet = CharSet.Unicode, SetLastError = true, ExactSpelling = true)]
    internal static extern IntPtr LoadLibraryEx(string path, IntPtr file, uint flags);
}
