// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl

using System;

namespace Danaco.Nurt;

internal static class Guard
{
    public static void NotNull(object? value, string name)
    {
#if NET8_0_OR_GREATER
        ArgumentNullException.ThrowIfNull(value, name);
#else
        if (value is null)
        {
            throw new ArgumentNullException(name);
        }
#endif
    }

    public static void NotDisposed(bool disposed, object instance)
    {
#if NET8_0_OR_GREATER
        ObjectDisposedException.ThrowIf(disposed, instance);
#else
        if (disposed)
        {
            throw new ObjectDisposedException(instance.GetType().FullName);
        }
#endif
    }
}
