// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           bindings/dotnet/src/Danaco.Nurt/Privilege.cs
// Responsibility:   Source file bindings/dotnet/src/Danaco.Nurt/Privilege.cs.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

namespace Danaco.Nurt;

/// <summary>Rights with which an operation runs.</summary>
public enum Privilege
{
    /// <summary>The rights of the account (<c>account</c>); the default.</summary>
    Account = 0,

    /// <summary>The rights of the administrator (<c>elevated</c>).</summary>
    Elevated = 1,

    /// <summary>The rights of the account, with one elevated repetition after <c>E_PERMISSION</c> or <c>E_PRIVILEGE</c> (<c>auto</c>).</summary>
    Auto = 2,
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
