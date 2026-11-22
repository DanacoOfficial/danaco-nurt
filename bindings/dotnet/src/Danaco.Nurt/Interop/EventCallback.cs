using System;
using System.Runtime.CompilerServices;
using System.Runtime.InteropServices;
using System.Text;

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
        // No exception may cross the native boundary. An event that cannot be decoded is
        // dropped; the complete output stays available through the cursor of the response.
        try
        {
            if (GCHandle.FromIntPtr(user).Target is EventQueue queue)
            {
                queue.Add(ExecutionEvent.Parse(Utf8.FromNative(eventJson)));
            }
        }
        catch (CallException)
        {
            return;
        }
        catch (DecoderFallbackException)
        {
            return;
        }
        catch (InvalidOperationException)
        {
            return;
        }
    }
}
