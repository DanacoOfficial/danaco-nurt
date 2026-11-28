// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           bindings/dotnet/src/Danaco.Nurt/BackendKind.cs
// Responsibility:   Source file bindings/dotnet/src/Danaco.Nurt/BackendKind.cs.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

namespace Danaco.Nurt;

/// <summary>Mechanism through which a <see cref="NurtClient"/> reaches Danaco Nurt.</summary>
public enum BackendKind
{
    /// <summary>
    /// The library when it is found and implements a compatible interface version;
    /// otherwise the process.
    /// </summary>
    Auto = 0,

    /// <summary>The shared library <c>libnurt</c>, loaded into the current process.</summary>
    Library = 1,

    /// <summary>A child process <c>nurt serve --stdio</c>, reached through JSON-RPC 2.0.</summary>
    Process = 2,
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
