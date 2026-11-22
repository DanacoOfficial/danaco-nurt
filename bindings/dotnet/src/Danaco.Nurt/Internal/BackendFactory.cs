using System;
using System.Collections.Generic;

namespace Danaco.Nurt;

internal static class BackendFactory
{
    public static IBackend Create(ClientOptions options, ArtifactLocator locator) => options.Backend switch
    {
        BackendKind.Library => CreateLibrary(options, locator.FindLibrary(options.LibraryPath)),
        BackendKind.Process => new ProcessBackend(locator.FindExecutable(options.ExecutablePath), options),
        BackendKind.Auto => CreateAuto(options, locator),
        _ => throw new ArgumentOutOfRangeException(nameof(options), options.Backend, "unknown backend kind"),
    };

    private static LibraryBackend CreateLibrary(ClientOptions options, string path)
    {
        LibraryBackend.Load(path);
        return LibraryBackend.Open(options);
    }

    /// <summary>
    /// Uses the library when it is found, loads and implements a compatible interface
    /// version; otherwise the process. A failure to open the engine is not a reason to fall
    /// back, because the process would meet the same configuration.
    /// </summary>
    private static IBackend CreateAuto(ClientOptions options, ArtifactLocator locator)
    {
        var checkedPaths = new List<string>();
        string? reason = null;
        if (locator.TryFindLibrary(options.LibraryPath, checkedPaths) is { } path)
        {
            try
            {
                LibraryBackend.Load(path);
            }
            catch (CallException exception)
            {
                reason = exception.Message;
            }
        }
        else
        {
            reason = ArtifactLocator.NotFound("libnurt library", checkedPaths).Message;
        }

        if (reason is null)
        {
            return LibraryBackend.Open(options);
        }

        try
        {
            return new ProcessBackend(locator.FindExecutable(options.ExecutablePath), options);
        }
        catch (CallException exception)
        {
            throw new CallException(exception.Status, exception.Message + "; library backend not used: " + reason, exception);
        }
    }
}
