using System;
using System.Runtime.InteropServices;

namespace Danaco.Nurt;

/// <summary>Owned handle of <c>nurt_engine</c>, closed with <c>nurt_engine_close</c>.</summary>
internal sealed class EngineHandle : SafeHandle
{
    public EngineHandle()
        : base(IntPtr.Zero, ownsHandle: true)
    {
    }

    public override bool IsInvalid => handle == IntPtr.Zero;

    protected override bool ReleaseHandle()
    {
        NativeMethods.EngineClose(handle);
        return true;
    }
}
