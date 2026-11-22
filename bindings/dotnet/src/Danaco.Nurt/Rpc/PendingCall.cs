using System.Text.Json;
using System.Threading;
using System.Threading.Tasks;

namespace Danaco.Nurt;

/// <summary>Request sent to <c>nurt serve</c> and waiting for its response.</summary>
internal sealed class PendingCall
{
    private SemaphoreSlim? _slot;

    public PendingCall(long id, string method, SemaphoreSlim? slot)
    {
        Id = id;
        Method = method;
        _slot = slot;
    }

    public long Id { get; }

    public string Method { get; }

    public TaskCompletionSource<JsonElement> Source { get; } = new(TaskCreationOptions.RunContinuationsAsynchronously);

    /// <summary>Returns the in-flight slot held by the request; later calls do nothing.</summary>
    public void ReleaseSlot() => Interlocked.Exchange(ref _slot, null)?.Release();
}
