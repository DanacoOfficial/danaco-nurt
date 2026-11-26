// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl

using System;
using System.IO;
using System.Runtime.InteropServices;
using System.Threading;
#if NET8_0_OR_GREATER
using System.Reflection;
#endif

namespace Danaco.Nurt;

/// <summary>
/// Loads <c>libnurt</c> from a resolved path. A process holds one library: the first
/// client that uses the library backend selects its file.
/// </summary>
/// <remarks>
/// On .NET 8 the declarations of <see cref="NativeMethods"/> are bound to the loaded file
/// through <c>NativeLibrary.SetDllImportResolver</c>. On .NET Standard 2.0 the file is
/// preloaded on Windows, where a module that is already loaded satisfies later imports of
/// the same name; on Linux and macOS the same <c>NativeLibrary</c> calls are made by reflection
/// (<see cref="PortableLibrary"/>), which needs .NET Core 3.0 or later. On a runtime without
/// <c>NativeLibrary</c> the path is not used and the runtime searches its default locations for
/// <c>libnurt</c>; a missing file is then reported by the failed load of the first import.
/// </remarks>
internal static class LibraryLoader
{
    private static readonly object Gate = new();
    private static string? s_path;
#if NET8_0_OR_GREATER
    private static IntPtr s_handle;
#endif

    /// <summary>Path of the loaded library; <see langword="null"/> before the first load.</summary>
    public static string? LoadedPath => Volatile.Read(ref s_path);

    public static void Load(string path)
    {
        string full = Path.GetFullPath(path);
        lock (Gate)
        {
            if (s_path is not null)
            {
                if (!string.Equals(s_path, full, StringComparison.Ordinal))
                {
                    throw new NurtCallException(
                        CallStatus.InvalidArgument,
                        "libnurt is already loaded from " + s_path + "; a process uses one library file");
                }

                return;
            }

#if NET8_0_OR_GREATER
            IntPtr handle;
            try
            {
                handle = NativeLibrary.Load(full);
            }
            catch (DllNotFoundException exception)
            {
                throw new NurtCallException(CallStatus.Io, "cannot load libnurt from " + full + ": " + exception.Message, exception);
            }
            catch (BadImageFormatException exception)
            {
                throw new NurtCallException(CallStatus.Io, "libnurt at " + full + " is built for another platform", exception);
            }

            Volatile.Write(ref s_handle, handle);
            NativeLibrary.SetDllImportResolver(typeof(LibraryLoader).Assembly, Resolve);
#else
            if (RuntimeInformation.IsOSPlatform(OSPlatform.Windows))
            {
                if (NativeMethods.LoadLibraryEx(full, IntPtr.Zero, NativeMethods.LoadWithAlteredSearchPath) == IntPtr.Zero)
                {
                    throw new NurtCallException(
                        CallStatus.Io,
                        "cannot load libnurt from " + full + ": system error " + Marshal.GetLastWin32Error().ToString(System.Globalization.CultureInfo.InvariantCulture));
                }
            }
            else if (PortableLibrary.IsAvailable)
            {
                PortableLibrary.BindImports(typeof(LibraryLoader).Assembly, NativeMethods.LibraryName, PortableLibrary.Load(full));
            }
#endif
            Volatile.Write(ref s_path, full);
        }
    }

#if NET8_0_OR_GREATER
    private static IntPtr Resolve(string libraryName, Assembly assembly, DllImportSearchPath? searchPath) =>
        libraryName == NativeMethods.LibraryName ? Volatile.Read(ref s_handle) : IntPtr.Zero;
#endif
}
