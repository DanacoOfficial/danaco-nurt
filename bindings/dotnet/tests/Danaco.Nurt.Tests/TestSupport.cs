using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.IO;
using System.Text.Json;
using System.Threading.Tasks;
using Microsoft.VisualStudio.TestTools.UnitTesting;

namespace Danaco.Nurt.Tests;

internal static class TestSupport
{
    /// <summary>Variable naming a <c>libnurt</c> build (the test stub or a release) for the library tests.</summary>
    public const string LibraryVariable = "NURT_TEST_LIBRARY";

    /// <summary>Variable naming an executable that implements <c>nurt serve --stdio</c> for the conformance cases.</summary>
    public const string ServeVariable = "NURT_TEST_SERVE";

    /// <summary>Arguments of <see cref="ServeVariable"/>, separated by spaces; default <c>serve --stdio</c>.</summary>
    public const string ServeArgumentsVariable = "NURT_TEST_SERVE_ARGS";

    private static readonly TimeSpan PollInterval = TimeSpan.FromMilliseconds(20);

    public static string TestServerAssembly => Path.Combine(AppContext.BaseDirectory, "Danaco.Nurt.TestServer.dll");

    /// <summary>Path of the <c>dotnet</c> host that runs the scripted server.</summary>
    public static string DotnetHost()
    {
        string? host = Environment.GetEnvironmentVariable("DOTNET_HOST_PATH");
        if (string.IsNullOrEmpty(host)
            && Environment.ProcessPath is { } current
            && string.Equals(Path.GetFileNameWithoutExtension(current), "dotnet", StringComparison.OrdinalIgnoreCase))
        {
            host = current;
        }

        if (string.IsNullOrEmpty(host) || !File.Exists(host))
        {
            Assert.Inconclusive("the dotnet host is not known; run the tests with dotnet test");
        }

        return host!;
    }

    /// <summary>Options of a process client connected to the scripted server.</summary>
    public static ClientOptions ProcessOptions(BackendKind backend, params string[] serverOptions)
    {
        var arguments = new List<string> { TestServerAssembly, "serve", "--stdio" };
        arguments.AddRange(serverOptions);
        return new ClientOptions
        {
            Backend = backend,
            ExecutablePath = DotnetHost(),
            ServeArguments = arguments,
            Session = "tests",
        };
    }

    public static Client ProcessClient(params string[] serverOptions) =>
        new(ProcessOptions(BackendKind.Process, serverOptions));

    /// <summary>Directory of the repository, found from the test output upwards; <see langword="null"/> outside a checkout.</summary>
    public static string? RepositoryRoot()
    {
        for (DirectoryInfo? directory = new(AppContext.BaseDirectory); directory is not null; directory = directory.Parent)
        {
            if (File.Exists(Path.Combine(directory.FullName, "crates", "nurt-ffi", "include", "nurt.h")))
            {
                return directory.FullName;
            }
        }

        return null;
    }

    public static string RequireRepositoryFile(params string[] parts)
    {
        string? root = RepositoryRoot();
        if (root is null)
        {
            Assert.Inconclusive("the tests do not run inside a checkout of the repository");
        }

        var segments = new List<string> { root! };
        segments.AddRange(parts);
        return Path.Combine(segments.ToArray());
    }

    public static string? Variable(string name) =>
        Environment.GetEnvironmentVariable(name) is { Length: > 0 } value ? value : null;

    /// <summary>Awaits an action and returns the expected exception, also when a derived type is thrown.</summary>
    public static async Task<T> ThrowsAsync<T>(Func<Task> action)
        where T : Exception
    {
        try
        {
            await action();
        }
        catch (T expected)
        {
            return expected;
        }

        throw new AssertFailedException("expected " + typeof(T).Name);
    }

    /// <summary>Repeats a check until it holds or the time runs out; used for effects delivered in the background.</summary>
    public static async Task<bool> EventuallyAsync(Func<Task<bool>> condition, TimeSpan limit)
    {
        var clock = Stopwatch.StartNew();
        while (clock.Elapsed < limit)
        {
            if (await condition())
            {
                return true;
            }

            await Task.Delay(PollInterval);
        }

        return await condition();
    }

    public static async Task<int> ServerProcessIdAsync(Client client)
    {
        JsonElement info = await client.InfoAsync();
        return info.GetProperty("pid").GetInt32();
    }

    public static async Task<int> CancelledCountAsync(Client client)
    {
        JsonElement info = await client.InfoAsync();
        return info.GetProperty("cancelled").GetInt32();
    }

    public static bool HasExited(int processId)
    {
        try
        {
            using Process process = Process.GetProcessById(processId);
            return process.HasExited;
        }
        catch (ArgumentException)
        {
            return true;
        }
    }

    public static JsonElement Json(string text)
    {
        using JsonDocument document = JsonDocument.Parse(text);
        return document.RootElement.Clone();
    }
}
