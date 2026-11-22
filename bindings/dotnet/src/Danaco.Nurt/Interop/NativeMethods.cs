// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl

using System.Runtime.InteropServices;

[assembly: DefaultDllImportSearchPaths(DllImportSearchPath.SafeDirectories)]

namespace Danaco.Nurt;

/// <summary>
/// Functions of <c>libnurt</c> declared in <c>nurt.h</c>. Every signature is blittable:
/// strings cross the boundary as zero-terminated UTF-8 bytes and results as
/// <see cref="NativeBuffer"/>, which is always released with <see cref="BufferFree"/>.
/// The declarations are generated with <c>LibraryImport</c> on .NET 8 and use
/// <c>DllImport</c> on .NET Standard 2.0.
/// </summary>
internal static unsafe partial class NativeMethods
{
    public const string LibraryName = "nurt";

    /// <summary>Major interface version of the header this binding is written against (<c>NURT_ABI_MAJOR</c>).</summary>
    public const int AbiMajor = 1;

    /// <summary>Lowest minor interface version this binding needs (<c>NURT_ABI_MINOR</c>).</summary>
    public const int AbiMinor = 0;

    /// <summary>Version of the header in the form returned by <c>nurt_abi_version()</c> (<c>NURT_ABI_VERSION</c>).</summary>
    public const uint AbiHeaderVersion = (AbiMajor << AbiMajorShift) | AbiMinor;

    /// <summary>Timeout of <c>nurt_exec_wait</c> that waits without limit (<c>NURT_WAIT_FOREVER</c>).</summary>
    public const uint WaitForever = uint.MaxValue;

    /// <summary>Bits of <c>nurt_abi_version()</c> that hold the minor version.</summary>
    public const uint AbiMinorMask = 0xFFFF;

    /// <summary>Shift of the major version in <c>nurt_abi_version()</c>.</summary>
    public const int AbiMajorShift = 16;
}
