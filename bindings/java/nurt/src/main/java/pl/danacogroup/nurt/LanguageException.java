// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
package pl.danacogroup.nurt;

/** Language error of a response, raised by {@link Response#orThrow()}. */
public final class LanguageException extends RuntimeException {
    private static final long serialVersionUID = 1L;

    private final transient LanguageError error;

    /**
     * Creates the exception.
     *
     * @param error the language error
     */
    public LanguageException(LanguageError error) {
        super(error.toString());
        this.error = error;
    }

    /**
     * The language error.
     *
     * @return the error
     */
    public LanguageError error() {
        return error;
    }
}
