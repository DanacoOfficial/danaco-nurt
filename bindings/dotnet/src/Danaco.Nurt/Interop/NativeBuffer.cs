// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           bindings/dotnet/src/Danaco.Nurt/Interop/NativeBuffer.cs
// Responsibility:   Source file bindings/dotnet/src/Danaco.Nurt/Interop/NativeBuffer.cs.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

using System.Runtime.InteropServices;
using System.Text.Json;

namespace Danaco.Nurt;

/// <summary>
/// Layout of <c>nurt_buf</c>. <see cref="Pointer"/> is null exactly when <see cref="Length"/>
/// is zero; a zero byte follows the last byte; <see cref="Capacity"/> belongs to the library.
/// </summary>
[StructLayout(LayoutKind.Sequential)]
internal unsafe struct NativeBuffer
{
    public byte* Pointer;
    public nuint Length;
    public nuint Capacity;

    public readonly bool IsEmpty => Length == 0;

    public readonly string ReadString() => Utf8.FromNative(Pointer, Length);

    public readonly JsonElement? ReadJson(string what) => IsEmpty ? null : JsonRead.Parse(ReadString(), what);
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
