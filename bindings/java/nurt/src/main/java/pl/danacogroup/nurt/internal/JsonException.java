// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
package pl.danacogroup.nurt.internal;

/** JSON text that does not follow RFC 8259. */
public final class JsonException extends IllegalArgumentException {
    private static final long serialVersionUID = 1L;

    /**
     * Creates the exception.
     *
     * @param message what is wrong and where
     */
    public JsonException(String message) {
        super(message);
    }
}
