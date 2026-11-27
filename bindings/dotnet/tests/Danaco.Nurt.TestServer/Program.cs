// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           bindings/dotnet/tests/Danaco.Nurt.TestServer/Program.cs
// Responsibility:   Source file bindings/dotnet/tests/Danaco.Nurt.TestServer/Program.cs.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

using System;
using System.Threading.Tasks;

namespace Danaco.Nurt.TestServer;

/// <summary>
/// Scripted implementation of <c>nurt serve --stdio</c> for the tests of the process backend.
/// The program text of a request selects the behaviour; see <see cref="ScriptedServer"/>.
/// </summary>
internal static class Program
{
    private const int UsageExitCode = 2;

    private static async Task<int> Main(string[] args)
    {
        if (args.Length < 2 || args[0] != "serve" || args[1] != "--stdio")
        {
            await Console.Error.WriteLineAsync("usage: serve --stdio [--protocol NAME] [--max-inflight N]").ConfigureAwait(false);
            return UsageExitCode;
        }

        ServerOptions options = ServerOptions.Parse(args);
        using var server = new ScriptedServer(options, Console.OpenStandardInput(), Console.OpenStandardOutput());
        return await server.RunAsync().ConfigureAwait(false);
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
