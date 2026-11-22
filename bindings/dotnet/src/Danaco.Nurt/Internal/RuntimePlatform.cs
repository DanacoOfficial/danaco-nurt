// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl

using System;
using System.Collections.Generic;
using System.IO;
using System.Runtime.InteropServices;

namespace Danaco.Nurt;

/// <summary>
/// Runtime identifier of the current platform in the form used by the package layout
/// <c>runtimes/&lt;rid&gt;/native/</c>, and the file names of the artifacts.
/// </summary>
internal sealed class RuntimePlatform
{
    private const string MuslLoaderDirectory = "/lib";
    private const string MuslLoaderPattern = "ld-musl-*.so.1";

    public RuntimePlatform(OSFamily family, string? runtimeIdentifier)
    {
        Family = family;
        RuntimeIdentifier = runtimeIdentifier;
    }

    public enum OSFamily
    {
        Other = 0,
        Linux = 1,
        MacOS = 2,
        Windows = 3,
    }

    public static RuntimePlatform Current { get; } = Detect();

    public OSFamily Family { get; }

    /// <summary>Identifier such as <c>linux-musl-x64</c>; <see langword="null"/> when no package asset exists for the platform.</summary>
    public string? RuntimeIdentifier { get; }

    public IReadOnlyList<string> ExecutableNames => Family == OSFamily.Windows
        ? new[] { "nurt.exe" }
        : new[] { "nurt" };

    public IReadOnlyList<string> LibraryNames => Family switch
    {
        OSFamily.Windows => new[] { "nurt.dll" },
        OSFamily.MacOS => new[] { "libnurt.dylib", "libnurt.1.dylib" },
        _ => new[] { "libnurt.so", "libnurt.so.1" },
    };

    private static RuntimePlatform Detect()
    {
        OSFamily family;
        string os;
        if (RuntimeInformation.IsOSPlatform(OSPlatform.Windows))
        {
            family = OSFamily.Windows;
            os = "win";
        }
        else if (RuntimeInformation.IsOSPlatform(OSPlatform.OSX))
        {
            family = OSFamily.MacOS;
            os = "osx";
        }
        else if (RuntimeInformation.IsOSPlatform(OSPlatform.Linux))
        {
            family = OSFamily.Linux;
            os = IsMusl() ? "linux-musl" : "linux";
        }
        else
        {
            return new RuntimePlatform(OSFamily.Other, null);
        }

        string? architecture = RuntimeInformation.OSArchitecture switch
        {
            Architecture.X64 => "x64",
            Architecture.Arm64 => "arm64",
            _ => null,
        };
        return new RuntimePlatform(family, architecture is null ? null : os + "-" + architecture);
    }

    private static bool IsMusl()
    {
        try
        {
            return Directory.Exists(MuslLoaderDirectory)
                && Directory.GetFiles(MuslLoaderDirectory, MuslLoaderPattern).Length > 0;
        }
        catch (IOException)
        {
            return false;
        }
        catch (UnauthorizedAccessException)
        {
            return false;
        }
    }
}
