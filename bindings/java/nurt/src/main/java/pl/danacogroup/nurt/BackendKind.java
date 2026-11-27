// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           bindings/java/nurt/src/main/java/pl/danacogroup/nurt/BackendKind.java
// Responsibility:   Source file bindings/java/nurt/src/main/java/pl/danacogroup/nurt/BackendKind.java.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

package pl.danacogroup.nurt;

/** How the client reaches Danaco Nurt. */
public enum BackendKind {
    /**
     * The library backend when it is installed, the library is found and its
     * interface version is compatible; otherwise the process backend.
     */
    AUTO,
    /** Child process {@code nurt serve --stdio}; needs Java 17 or later. */
    PROCESS,
    /**
     * The library {@code libnurt} called in the process; needs the artifact
     * {@code pl.danaco-group:nurt-ffm} and Java 22 or later.
     */
    LIBRARY
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
