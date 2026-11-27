// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           bindings/dotnet/src/Danaco.Nurt/Interop/ExecHandle.cs
// Responsibility:   Source file bindings/dotnet/src/Danaco.Nurt/Interop/ExecHandle.cs.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

using System;
using System.Runtime.InteropServices;
using System.Threading;

namespace Danaco.Nurt;

/// <summary>
/// Owned handle of <c>nurt_exec</c>, released with <c>nurt_exec_free</c>.
/// </summary>
/// <remarks>
/// The handle keeps the event queue of the execution reachable through a
/// <see cref="GCHandle"/> passed to the library as the callback argument. The library calls
/// the callback neither after <c>exec.finished</c> nor after <c>nurt_exec_free</c>, so the
/// <see cref="GCHandle"/> is freed right after <c>nurt_exec_free</c>. The handle also holds a
/// reference on the engine, so that <c>nurt_engine_close</c> runs only after every
/// execution of the engine is released.
/// </remarks>
internal sealed class ExecHandle : SafeHandle
{
    private static int s_live;
    private GCHandle _callbackState;
    private EngineHandle? _engine;

    public ExecHandle()
        : base(IntPtr.Zero, ownsHandle: true)
    {
    }

    /// <summary>Number of executions started and not yet released, in the whole process.</summary>
    internal static int LiveCount => Volatile.Read(ref s_live);

    public override bool IsInvalid => handle == IntPtr.Zero;

    /// <summary>
    /// Takes over the callback state and one reference on <paramref name="engine"/> that the caller
    /// added before it started the execution; the reference is released with the handle.
    /// </summary>
    public void Attach(GCHandle callbackState, EngineHandle engine)
    {
        _callbackState = callbackState;
        _engine = engine;
        Interlocked.Increment(ref s_live);
    }

    protected override bool ReleaseHandle()
    {
        NativeMethods.ExecFree(handle);
        if (_callbackState.IsAllocated)
        {
            _callbackState.Free();
        }

        _engine?.DangerousRelease();
        Interlocked.Decrement(ref s_live);
        return true;
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
