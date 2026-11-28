// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           bindings/java/nurt/src/main/java/pl/danacogroup/nurt/CallStatus.java
// Responsibility:   Source file bindings/java/nurt/src/main/java/pl/danacogroup/nurt/CallStatus.java.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

package pl.danacogroup.nurt;

import java.util.Optional;

/** Status code of a failed call, as defined by the C interface ({@code nurt.h}). */
public enum CallStatus {
    /** The operation did not finish within the time limit. */
    TIMEOUT(1, "NURT_E_TIMEOUT"),
    /** The operation ended because it was cancelled. */
    CANCELLED(2, "NURT_E_CANCELLED"),
    /** An argument is not valid. */
    ARG(3, "NURT_E_ARG"),
    /** The interface versions are not compatible. */
    ABI(4, "NURT_E_ABI"),
    /** A file, the state directory or the executable cannot be used. */
    IO(5, "NURT_E_IO"),
    /** Internal failure. */
    INTERNAL(6, "NURT_E_INTERNAL");

    private final int code;
    private final String cName;

    CallStatus(int code, String cName) {
        this.code = code;
        this.cName = cName;
    }

    /**
     * Numeric code in {@code nurt.h}.
     *
     * @return the code, from 1 to 6
     */
    public int code() {
        return code;
    }

    /**
     * Name in {@code nurt.h}, for example {@code NURT_E_ARG}.
     *
     * @return the name
     */
    public String cName() {
        return cName;
    }

    /**
     * Status with the given numeric code.
     *
     * @param code numeric code
     * @return the status; empty for {@code 0} ({@code NURT_OK}) and unknown codes
     */
    public static Optional<CallStatus> fromCode(int code) {
        for (CallStatus status : values()) {
            if (status.code == code) {
                return Optional.of(status);
            }
        }
        return Optional.empty();
    }

    /**
     * Status with the given name in {@code nurt.h}.
     *
     * @param cName name such as {@code NURT_E_IO}
     * @return the status, or empty for an unknown name
     */
    public static Optional<CallStatus> fromCName(String cName) {
        for (CallStatus status : values()) {
            if (status.cName.equals(cName)) {
                return Optional.of(status);
            }
        }
        return Optional.empty();
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
