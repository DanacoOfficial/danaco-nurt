// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           bindings/dotnet/src/Danaco.Nurt/Interop/EventCallback.cs
// Responsibility:   Source file bindings/dotnet/src/Danaco.Nurt/Interop/EventCallback.cs.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

using System;
using System.Runtime.CompilerServices;
using System.Runtime.InteropServices;

namespace Danaco.Nurt;

/// <summary>
/// Receiver of <c>nurt_event_cb</c>. The callback argument is a <see cref="GCHandle"/> of the
/// <see cref="EventQueue"/> of the execution; events are queued without blocking the
/// library thread.
/// </summary>
internal static unsafe class EventCallback
{
#if NET8_0_OR_GREATER
    public static IntPtr Pointer => (IntPtr)(delegate* unmanaged[Cdecl]<IntPtr, byte*, void>)&OnEvent;

    [UnmanagedCallersOnly(CallConvs = new[] { typeof(CallConvCdecl) })]
    private static void OnEvent(IntPtr user, byte* eventJson) => Deliver(user, eventJson);
#else
    // The delegate is held for the life of the process, so the pointer given to the
    // library never refers to a collected delegate.
    private static readonly NativeEventCallback Instance = Deliver;

    public static IntPtr Pointer { get; } = Marshal.GetFunctionPointerForDelegate(Instance);

    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate void NativeEventCallback(IntPtr user, byte* eventJson);
#endif

    private static void Deliver(IntPtr user, byte* eventJson)
    {
        // No exception may cross the native boundary: an exception that leaves a function of
        // UnmanagedCallersOnly ends the process. An event that cannot be decoded or queued is
        // dropped; the complete output stays available through the cursor of the response.
#pragma warning disable CA1031 // Every exception is contained here, whatever its type.
        try
        {
            if (GCHandle.FromIntPtr(user).Target is EventQueue queue)
            {
                queue.Add(NurtExecutionEvent.Parse(Utf8.FromNative(eventJson)));
            }
        }
        catch (Exception)
        {
            return;
        }
#pragma warning restore CA1031
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
