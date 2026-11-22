// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl

using Microsoft.VisualStudio.TestTools.UnitTesting;

// Tests start child processes and read process-wide state (the loaded library and the
// count of live execution handles), so they run one at a time.
[assembly: DoNotParallelize]
