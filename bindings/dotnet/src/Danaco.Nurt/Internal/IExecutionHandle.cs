using System;
using System.Threading;
using System.Threading.Tasks;

namespace Danaco.Nurt;

/// <summary>
/// Running execution of one backend. <see cref="Completion"/> ends with the response,
/// also after cancellation; disposing cancels a running execution and releases it.
/// </summary>
internal interface IExecutionHandle : IDisposable
{
    Task<Response> Completion { get; }

    Task CancelAsync(CancellationToken cancellationToken);
}
