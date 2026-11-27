// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           bindings/dotnet/src/Danaco.Nurt/Internal/IExecutionHandle.cs
// Responsibility:   Source file bindings/dotnet/src/Danaco.Nurt/Internal/IExecutionHandle.cs.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

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

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
