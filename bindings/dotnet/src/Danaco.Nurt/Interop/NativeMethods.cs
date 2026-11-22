using System.Runtime.InteropServices;

namespace Danaco.Nurt;

/// <summary>
/// Functions of <c>libnurt</c> declared in <c>nurt.h</c>. Every signature is blittable:
/// strings cross the boundary as zero-terminated UTF-8 bytes and results as
/// <see cref="NativeBuffer"/>, which is always released with <see cref="BufferFree"/>.
/// The declarations are generated with <c>LibraryImport</c> on .NET 8 and use
/// <c>DllImport</c> on .NET Standard 2.0.
/// </summary>
[DefaultDllImportSearchPaths(DllImportSearchPath.SafeDirectories)]
internal static unsafe partial class NativeMethods
{
    public const string LibraryName = "nurt";

    /// <summary>Major interface version of the header this binding is written against (<c>NURT_ABI_MAJOR</c>).</summary>
    public const int AbiMajor = 1;

    /// <summary>Lowest minor interface version this binding needs (<c>NURT_ABI_MINOR</c>).</summary>
    public const int AbiMinor = 0;

    /// <summary>Timeout of <c>nurt_exec_wait</c> that waits without limit (<c>NURT_WAIT_FOREVER</c>).</summary>
    public const uint WaitForever = uint.MaxValue;

    /// <summary>Bits of <c>nurt_abi_version()</c> that hold the minor version.</summary>
    public const uint AbiMinorMask = 0xFFFF;

    /// <summary>Shift of the major version in <c>nurt_abi_version()</c>.</summary>
    public const int AbiMajorShift = 16;
}
