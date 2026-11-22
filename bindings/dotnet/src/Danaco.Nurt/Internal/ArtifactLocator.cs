// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl

using System;
using System.Collections.Generic;
using System.IO;

namespace Danaco.Nurt;

/// <summary>
/// Finds the <c>nurt</c> executable and the <c>libnurt</c> library. A path given in the
/// client options or in the variables <c>NURT_EXECUTABLE</c> and <c>NURT_LIBRARY</c> is used
/// as given; otherwise the file bundled with the package for the current platform is used,
/// and for the executable the directories of <c>PATH</c> are searched last.
/// </summary>
internal sealed class ArtifactLocator
{
    public const string ExecutableVariable = "NURT_EXECUTABLE";
    public const string LibraryVariable = "NURT_LIBRARY";
    private const string PathVariable = "PATH";

    private readonly Func<string, string?> _environment;
    private readonly string _baseDirectory;
    private readonly RuntimePlatform _platform;

    public ArtifactLocator()
        : this(Environment.GetEnvironmentVariable, AppContext.BaseDirectory, RuntimePlatform.Current)
    {
    }

    public ArtifactLocator(Func<string, string?> environment, string baseDirectory, RuntimePlatform platform)
    {
        _environment = environment;
        _baseDirectory = baseDirectory;
        _platform = platform;
    }

    public string FindExecutable(string? explicitPath)
    {
        var checkedPaths = new List<string>();
        return Find(explicitPath, ExecutableVariable, _platform.ExecutableNames, searchPath: true, checkedPaths)
            ?? throw NotFound("nurt executable", checkedPaths);
    }

    public string FindLibrary(string? explicitPath)
    {
        var checkedPaths = new List<string>();
        return Find(explicitPath, LibraryVariable, _platform.LibraryNames, searchPath: false, checkedPaths)
            ?? throw NotFound("libnurt library", checkedPaths);
    }

    public string? TryFindLibrary(string? explicitPath, List<string> checkedPaths) =>
        Find(explicitPath, LibraryVariable, _platform.LibraryNames, searchPath: false, checkedPaths);

    internal static NurtCallException NotFound(string what, List<string> checkedPaths) =>
        new(CallStatus.Io, what + " not found; checked: " + (checkedPaths.Count == 0 ? "no location" : string.Join(", ", checkedPaths)));

    private string? Find(string? explicitPath, string variable, IReadOnlyList<string> names, bool searchPath, List<string> checkedPaths)
    {
        string? given = !string.IsNullOrEmpty(explicitPath) ? explicitPath : _environment(variable);
        if (!string.IsNullOrEmpty(given))
        {
            string full = Path.GetFullPath(given);
            checkedPaths.Add(full);
            return File.Exists(full) ? full : null;
        }

        if (_platform.RuntimeIdentifier is { } rid)
        {
            string bundled = Path.Combine(_baseDirectory, "runtimes", rid, "native");
            foreach (string name in names)
            {
                if (Probe(Path.Combine(bundled, name), checkedPaths) is { } found)
                {
                    return found;
                }
            }
        }

        foreach (string name in names)
        {
            if (Probe(Path.Combine(_baseDirectory, name), checkedPaths) is { } found)
            {
                return found;
            }
        }

        if (searchPath && _environment(PathVariable) is { Length: > 0 } pathList)
        {
            foreach (string directory in pathList.Split(Path.PathSeparator))
            {
                if (directory.Length == 0)
                {
                    continue;
                }

                foreach (string name in names)
                {
                    if (Probe(Path.Combine(directory, name), checkedPaths) is { } found)
                    {
                        return found;
                    }
                }
            }
        }

        return null;
    }

    private static string? Probe(string candidate, List<string> checkedPaths)
    {
        checkedPaths.Add(candidate);
        return File.Exists(candidate) ? candidate : null;
    }
}
