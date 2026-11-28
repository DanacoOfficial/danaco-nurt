// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           bindings/java/nurt/src/main/java/pl/danacogroup/nurt/LanguageError.java
// Responsibility:   Source file bindings/java/nurt/src/main/java/pl/danacogroup/nurt/LanguageError.java.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

package pl.danacogroup.nurt;

import java.util.ArrayList;
import java.util.Collections;
import java.util.List;
import java.util.Map;
import java.util.Optional;
import pl.danacogroup.nurt.internal.Json;

/**
 * Error of the language reported in a response: a stable {@code E_*} code,
 * the stage, the position in the program and ready fixes.
 */
public final class LanguageError {
    private final Map<String, Object> value;

    LanguageError(Map<String, Object> value) {
        this.value = Collections.unmodifiableMap(value);
    }

    /**
     * Stable code, for example {@code E_SYNTAX}.
     *
     * @return the code, empty when the server sent none
     */
    public String code() {
        return Optional.ofNullable(Json.string(value, "code")).orElse("");
    }

    /**
     * Message in the language of the request.
     *
     * @return the message
     */
    public String message() {
        return Optional.ofNullable(Json.string(value, "message")).orElse("");
    }

    /**
     * Stage in which the error occurred, for example {@code 1}; the member
     * {@code stage} of {@code at}, or a member {@code stage} of the error itself.
     *
     * @return the stage
     */
    public Optional<String> stage() {
        String stage = Json.string(value.get("at"), "stage");
        return Optional.ofNullable(stage != null ? stage : Json.string(value, "stage"));
    }

    /**
     * Position in the program: {@code line}, {@code column} and {@code length}.
     *
     * @return the position
     */
    public Optional<Map<String, Object>> at() {
        return Optional.ofNullable(Json.object(value.get("at")));
    }

    /**
     * Ready replacements of the program text, best first.
     *
     * @return the replacements
     */
    public List<String> fix() {
        List<String> fixes = new ArrayList<>();
        if (value.get("fix") instanceof List<?> list) {
            for (Object item : list) {
                if (item instanceof String text) {
                    fixes.add(text);
                }
            }
        }
        return Collections.unmodifiableList(fixes);
    }

    /**
     * Help topic with the rules that apply.
     *
     * @return the topic
     */
    public Optional<String> help() {
        return Optional.ofNullable(Json.string(value, "help"));
    }

    /**
     * The whole error object.
     *
     * @return the object, read-only
     */
    public Map<String, Object> value() {
        return value;
    }

    @Override
    public String toString() {
        return code() + ": " + message();
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
