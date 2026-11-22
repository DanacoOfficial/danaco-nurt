using System.Text.Json;
using System.Threading;
using System.Threading.Tasks;

namespace Danaco.Nurt;

/// <summary>Execution started with the method <c>run</c> of <c>nurt serve</c>.</summary>
internal sealed class ProcessExecution : IExecutionHandle
{
    private readonly RpcConnection _connection;
    private readonly PendingCall _call;
    private readonly EventQueue _events;

    public ProcessExecution(RpcConnection connection, PendingCall call, EventQueue events)
    {
        _connection = connection;
        _call = call;
        _events = events;
        Completion = ConvertAsync(call.Source.Task);
    }

    public Task<Response> Completion { get; }

    public Task CancelAsync(CancellationToken cancellationToken) =>
        _connection.CancelRequestAsync(_call.Id, cancellationToken);

    public void Dispose()
    {
        if (!_call.Source.Task.IsCompleted)
        {
            _connection.CancelInBackground(_call.Id);
        }

        _connection.RemoveRoute(_call.Id);
        _events.Complete();
    }

    private static async Task<Response> ConvertAsync(Task<JsonElement> result) =>
        Response.FromElement(await result.ConfigureAwait(false));
}
