// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           bindings/dotnet/src/Danaco.Nurt/Interop/PortableLibrary.cs
// Responsibility:   Source file bindings/dotnet/src/Danaco.Nurt/Interop/PortableLibrary.cs.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

using System;
using System.Reflection;
using System.Runtime.InteropServices;

namespace Danaco.Nurt;

/// <summary>
/// Loads a library from an explicit path through <c>System.Runtime.InteropServices.NativeLibrary</c>
/// by reflection, so that a build for .NET Standard 2.0 can honour the path on Linux and macOS.
/// </summary>
/// <remarks>
/// .NET Standard 2.0 has no <c>NativeLibrary</c>; the type exists at run time on .NET Core 3.0 and
/// later (also .NET 5 and later). On a runtime without it <see cref="IsAvailable"/> is
/// <see langword="false"/> and the runtime searches its default locations for <c>libnurt</c>.
/// </remarks>
#pragma warning disable IL2026, IL2057, IL2075, IL2080 // Reflection on a framework type that is present on every runtime that has it.
internal static class PortableLibrary
{
    private const string TypeName = "System.Runtime.InteropServices.NativeLibrary";
    private const string ResolverTypeName = "System.Runtime.InteropServices.DllImportResolver";

    private static readonly Type[] LoadSignature = { typeof(string) };
    private static readonly Type? NativeLibraryType = typeof(object).Assembly.GetType(TypeName);

    /// <summary>Whether the running .NET has <c>NativeLibrary</c>.</summary>
    public static bool IsAvailable => NativeLibraryType?.GetMethod("Load", LoadSignature) is not null
        && NativeLibraryType.GetMethod("SetDllImportResolver") is not null;

    /// <summary>Loads the file at <paramref name="path"/> and returns its handle.</summary>
    /// <exception cref="NurtCallException">The file cannot be loaded; the message names the path and the cause.</exception>
    public static IntPtr Load(string path)
    {
        MethodInfo load = NativeLibraryType?.GetMethod("Load", LoadSignature)
            ?? throw new NurtCallException(CallStatus.Io, "cannot load libnurt from " + path + ": this runtime cannot load a library from a path");
        try
        {
            return (IntPtr)load.Invoke(null, new object[] { path })!;
        }
        catch (TargetInvocationException exception) when (exception.InnerException is DllNotFoundException or BadImageFormatException)
        {
            Exception cause = exception.InnerException!;
            throw new NurtCallException(CallStatus.Io, "cannot load libnurt from " + path + ": " + cause.Message, cause);
        }
    }

    /// <summary>Makes the imports of <paramref name="libraryName"/> in <paramref name="assembly"/> use <paramref name="handle"/>.</summary>
    public static void BindImports(Assembly assembly, string libraryName, IntPtr handle)
    {
        MethodInfo set = NativeLibraryType?.GetMethod("SetDllImportResolver")
            ?? throw new NurtCallException(CallStatus.Io, "this runtime cannot bind libnurt to a path");
        Type resolverType = NativeLibraryType!.Assembly.GetType(ResolverTypeName)
            ?? throw new NurtCallException(CallStatus.Io, "this runtime cannot bind libnurt to a path");
        var resolver = new Resolver(libraryName, handle);
        Delegate callback = Delegate.CreateDelegate(resolverType, resolver, nameof(Resolver.Resolve));
        try
        {
            set.Invoke(null, new object[] { assembly, callback });
        }
        catch (TargetInvocationException exception) when (exception.InnerException is InvalidOperationException)
        {
            throw new NurtCallException(CallStatus.Io, "libnurt is already bound to a path in this process", exception.InnerException);
        }
    }

    /// <summary>Answers the import of one library name with a loaded handle.</summary>
    internal sealed class Resolver
    {
        private readonly string _name;
        private readonly IntPtr _handle;

        public Resolver(string name, IntPtr handle)
        {
            _name = name;
            _handle = handle;
        }

        public IntPtr Resolve(string libraryName, Assembly assembly, DllImportSearchPath? searchPath) =>
            string.Equals(libraryName, _name, StringComparison.Ordinal) ? _handle : IntPtr.Zero;
    }
}
#pragma warning restore IL2026, IL2057, IL2075

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
