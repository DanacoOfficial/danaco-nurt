// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           bindings/dotnet/src/Danaco.Nurt/Rpc/ProcessExecution.cs
// Responsibility:   Source file bindings/dotnet/src/Danaco.Nurt/Rpc/ProcessExecution.cs.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

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

    public Task<NurtResponse> Completion { get; }

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

    private static async Task<NurtResponse> ConvertAsync(Task<JsonElement> result) =>
        NurtResponse.FromElement(await result.ConfigureAwait(false));
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
