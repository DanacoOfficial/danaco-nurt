using System;
using System.Collections.Generic;
using System.IO;
using System.Threading.Tasks;
using Microsoft.VisualStudio.TestTools.UnitTesting;

namespace Danaco.Nurt.Tests;

[TestClass]
public sealed class ArtifactLocatorTests
{
    private static readonly RuntimePlatform Linux = new(RuntimePlatform.OSFamily.Linux, "linux-x64");
    private readonly Dictionary<string, string> _environment = new(StringComparer.Ordinal);
    private string _root = string.Empty;

    [TestInitialize]
    public void CreateDirectory()
    {
        _root = Path.Combine(Path.GetTempPath(), "nurt-dotnet-" + Guid.NewGuid().ToString("N"));
        Directory.CreateDirectory(_root);
    }

    [TestCleanup]
    public void RemoveDirectory() => Directory.Delete(_root, recursive: true);

    [TestMethod]
    public void ExplicitPathIsUsedAsGiven()
    {
        string file = Touch("custom", "nurt");
        _environment[ArtifactLocator.ExecutableVariable] = Touch("other", "nurt");

        Assert.AreEqual(file, Locator(Linux).FindExecutable(file));
    }

    [TestMethod]
    public void MissingExplicitPathFailsWithoutSearchingFurther()
    {
        Touch("runtimes", "linux-x64", "native", "nurt");
        string missing = Path.Combine(_root, "missing", "nurt");

        CallException exception = Assert.ThrowsException<CallException>(() => Locator(Linux).FindExecutable(missing));

        Assert.AreEqual(CallStatus.Io, exception.Status);
        StringAssert.Contains(exception.Message, missing);
    }

    [TestMethod]
    public void VariableIsUsedWhenNoPathIsGiven()
    {
        string file = Touch("from-variable", "libnurt.so");
        _environment[ArtifactLocator.LibraryVariable] = file;

        Assert.AreEqual(file, Locator(Linux).FindLibrary(null));
    }

    [TestMethod]
    public void BundledFilesAreFoundForTheRuntimeIdentifier()
    {
        string executable = Touch("runtimes", "linux-x64", "native", "nurt");
        string library = Touch("runtimes", "linux-x64", "native", "libnurt.so.1");

        ArtifactLocator locator = Locator(Linux);

        Assert.AreEqual(executable, locator.FindExecutable(null));
        Assert.AreEqual(library, locator.FindLibrary(null));
    }

    [TestMethod]
    public void FilesNextToTheApplicationAreFound()
    {
        string library = Touch("libnurt.so");

        Assert.AreEqual(library, Locator(Linux).FindLibrary(null));
    }

    [TestMethod]
    public void PathIsSearchedForTheExecutableOnly()
    {
        string executable = Touch("bin", "nurt");
        Touch("bin", "libnurt.so");
        _environment["PATH"] = Path.Combine(_root, "empty") + Path.PathSeparator + Path.Combine(_root, "bin");
        ArtifactLocator locator = Locator(Linux);

        CallException exception = Assert.ThrowsException<CallException>(() => locator.FindLibrary(null));

        Assert.AreEqual(executable, locator.FindExecutable(null));
        Assert.AreEqual(CallStatus.Io, exception.Status);
        StringAssert.Contains(exception.Message, "libnurt library not found");
        Assert.IsFalse(exception.Message.Contains(Path.Combine(_root, "bin"), StringComparison.Ordinal));
    }

    [TestMethod]
    public void WindowsNamesCarryExtensions()
    {
        var windows = new RuntimePlatform(RuntimePlatform.OSFamily.Windows, "win-x64");

        Assert.AreEqual("nurt.exe", windows.ExecutableNames[0]);
        Assert.AreEqual("nurt.dll", windows.LibraryNames[0]);
    }

    [TestMethod]
    public void UnknownPlatformHasNoBundledAssets()
    {
        Touch("runtimes", "linux-x64", "native", "nurt");
        var other = new RuntimePlatform(RuntimePlatform.OSFamily.Linux, null);

        Assert.ThrowsException<CallException>(() => Locator(other).FindExecutable(null));
    }

    [TestMethod]
    public void AutoBackendUsesTheProcessWhenNoLibraryIsFound()
    {
        ClientOptions options = TestSupport.ProcessOptions(BackendKind.Auto);

        using var client = new Client(options, Locator(Linux));

        Assert.AreEqual(BackendKind.Process, client.Backend);
    }

    [TestMethod]
    public void AutoBackendUsesTheProcessWhenTheLibraryCannotBeLoaded()
    {
        ClientOptions options = TestSupport.ProcessOptions(BackendKind.Auto);
        options = new ClientOptions
        {
            Backend = BackendKind.Auto,
            ExecutablePath = options.ExecutablePath,
            ServeArguments = options.ServeArguments,
            LibraryPath = Touch("broken", "libnurt.so"),
        };

        using var client = new Client(options, Locator(Linux));

        Assert.AreEqual(BackendKind.Process, client.Backend);
    }

    [TestMethod]
    public void AutoBackendReportsBothArtifactsWhenNothingIsFound()
    {
        var options = new ClientOptions { Backend = BackendKind.Auto };

        CallException exception = Assert.ThrowsException<CallException>(() => new Client(options, Locator(Linux)));

        Assert.AreEqual(CallStatus.Io, exception.Status);
        StringAssert.Contains(exception.Message, "nurt executable not found");
        StringAssert.Contains(exception.Message, "library backend not used");
    }

    [TestMethod]
    public void LibraryBackendWithoutLibraryIsIoError()
    {
        var options = new ClientOptions { Backend = BackendKind.Library, LibraryPath = Path.Combine(_root, "absent", "libnurt.so") };

        CallException exception = Assert.ThrowsException<CallException>(() => new Client(options, Locator(Linux)));

        Assert.AreEqual(CallStatus.Io, exception.Status);
    }

    [TestMethod]
    public async Task ProcessBackendWithoutExecutableIsIoError()
    {
        var options = new ClientOptions { Backend = BackendKind.Process };

        CallException exception = Assert.ThrowsException<CallException>(() => new Client(options, Locator(Linux)));

        Assert.AreEqual(CallStatus.Io, exception.Status);
        StringAssert.Contains(exception.Message, Path.Combine(_root, "runtimes", "linux-x64", "native", "nurt"));
        await Task.CompletedTask;
    }

    private ArtifactLocator Locator(RuntimePlatform platform) =>
        new(name => _environment.TryGetValue(name, out string? value) ? value : null, _root, platform);

    private string Touch(params string[] parts)
    {
        var segments = new List<string> { _root };
        segments.AddRange(parts);
        string path = Path.Combine(segments.ToArray());
        Directory.CreateDirectory(Path.GetDirectoryName(path)!);
        File.WriteAllText(path, "test file");
        return path;
    }
}
