// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           bindings/java/nurt/src/main/java/pl/danacogroup/nurt/NurtResponse.java
// Responsibility:   Source file bindings/java/nurt/src/main/java/pl/danacogroup/nurt/NurtResponse.java.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

package pl.danacogroup.nurt;

import java.util.ArrayList;
import java.util.Collections;
import java.util.List;
import java.util.Map;
import java.util.Objects;
import java.util.Optional;
import pl.danacogroup.nurt.internal.Json;
import pl.danacogroup.nurt.internal.JsonException;

/**
 * Response to a request ({@code nurt-response/1}): the effect report
 * {@code nurt-report/1} and, for text formats, the rendered text.
 *
 * <p>A language error is part of a response, not an exception:
 * {@link #ok()} is {@code false} and {@link #error()} returns the error.
 * {@link #orThrow()} turns it into a {@link NurtLanguageException}.
 */
public final class NurtResponse {
    private final Map<String, Object> value;
    private final Map<String, Object> report;

    private NurtResponse(Map<String, Object> value, Map<String, Object> report) {
        this.value = Collections.unmodifiableMap(value);
        this.report = Collections.unmodifiableMap(report);
    }

    /**
     * Response from a parsed JSON value.
     *
     * @param value the value
     * @return the response
     * @throws NurtCallException of kind {@link NurtCallException.Kind#PROTOCOL} when
     *     the value is not an object with an object {@code report}
     */
    public static NurtResponse of(Object value) {
        Map<String, Object> object = Json.object(value);
        Map<String, Object> report = Json.object(Json.member(object, "report"));
        if (object == null || report == null) {
            throw new NurtCallException(NurtCallException.Kind.PROTOCOL, "response has no `report` object");
        }
        return new NurtResponse(object, report);
    }

    /**
     * Response from JSON text.
     *
     * @param json the text
     * @return the response
     * @throws NurtCallException of kind {@link NurtCallException.Kind#PROTOCOL} when
     *     the text is not JSON or not a response
     */
    public static NurtResponse parse(String json) {
        try {
            return of(Json.parse(json));
        } catch (JsonException e) {
            throw new NurtCallException(
                    NurtCallException.Kind.PROTOCOL, "response is not valid JSON: " + e.getMessage(), e);
        }
    }

    /**
     * The effect report.
     *
     * @return the report, read-only
     */
    public Map<String, Object> report() {
        return report;
    }

    /**
     * The reports of the pipelines of a script that ran several operations, in
     * program order. The list is empty for a program with a single operation,
     * whose {@link #report()} holds its records. The reports are not paged and
     * carry no cursor.
     *
     * @return the reports, read-only
     */
    public List<Map<String, Object>> reports() {
        Object items = value.get("reports");
        if (!(items instanceof List<?> list)) {
            return List.of();
        }
        List<Map<String, Object>> reports = new ArrayList<>(list.size());
        for (Object item : list) {
            Map<String, Object> object = Json.object(item);
            if (object != null) {
                reports.add(Collections.unmodifiableMap(object));
            }
        }
        return Collections.unmodifiableList(reports);
    }

    /**
     * Status of the report: {@code ok}, {@code planned}, {@code running},
     * {@code partial}, {@code rolled_back} or {@code failed}.
     *
     * @return the status
     */
    public Optional<String> status() {
        return Optional.ofNullable(Json.string(report, "status"));
    }

    /**
     * Text rendered from the report, when the requested format is a text format.
     *
     * @return the text
     */
    public Optional<String> text() {
        return Optional.ofNullable(Json.string(value.get("rendered"), "text"));
    }

    /**
     * Whether the report carries no language error.
     *
     * @return {@code true} without an error
     */
    public boolean ok() {
        return report.get("error") == null;
    }

    /**
     * Language error of the report.
     *
     * @return the error
     */
    public Optional<LanguageError> error() {
        return Optional.ofNullable(Json.object(report.get("error"))).map(LanguageError::new);
    }

    /**
     * Cursor of the next page, when the result has more pages.
     *
     * @return the cursor
     */
    public Optional<String> nextCursor() {
        return Optional.ofNullable(Json.string(report, "next_cursor"));
    }

    /**
     * The response, or its language error as an exception.
     *
     * @return this response when it has no language error
     * @throws NurtLanguageException when the report carries a language error
     */
    public NurtResponse orThrow() {
        Optional<LanguageError> error = error();
        if (error.isPresent()) {
            throw new NurtLanguageException(error.get());
        }
        return this;
    }

    /**
     * The whole response.
     *
     * @return the object, read-only
     */
    public Map<String, Object> value() {
        return value;
    }

    /**
     * The whole response as JSON text.
     *
     * @return JSON text
     */
    public String json() {
        return Json.write(value);
    }

    @Override
    public boolean equals(Object other) {
        return other instanceof NurtResponse response && value.equals(response.value);
    }

    @Override
    public int hashCode() {
        return Objects.hash(value);
    }

    @Override
    public String toString() {
        return json();
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
