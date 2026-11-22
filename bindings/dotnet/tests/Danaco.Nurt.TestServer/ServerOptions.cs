// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl

using System.Globalization;

namespace Danaco.Nurt.TestServer;

/// <summary>Options of the scripted server, given after <c>serve --stdio</c>.</summary>
internal sealed class ServerOptions
{
    /// <summary>Protocol name reported by <c>initialize</c>.</summary>
    public string Protocol { get; private set; } = "nurt-rpc/1";

    /// <summary>Limit of requests in progress reported by <c>initialize</c>; 0 reports no limit.</summary>
    public int MaxInflight { get; private set; }

    public static ServerOptions Parse(string[] args)
    {
        var options = new ServerOptions();
        for (int index = 2; index + 1 < args.Length; index += 2)
        {
            switch (args[index])
            {
                case "--protocol":
                    options.Protocol = args[index + 1];
                    break;
                case "--max-inflight":
                    options.MaxInflight = int.Parse(args[index + 1], CultureInfo.InvariantCulture);
                    break;
            }
        }

        return options;
    }
}
