// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl

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
    Task<NurtResponse> Completion { get; }

    Task CancelAsync(CancellationToken cancellationToken);
}
