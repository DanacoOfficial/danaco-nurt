// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
package pl.danacogroup.nurt;

import java.util.Collections;
import java.util.Map;
import java.util.Optional;
import java.util.OptionalLong;
import pl.danacogroup.nurt.internal.Json;

/**
 * Event of the server: progress and output of an execution, background jobs,
 * journal changes, free space, schedules and log messages.
 */
public final class NurtEvent {
    private final Map<String, Object> value;

    /**
     * Event from its JSON object.
     *
     * @param value the object
     */
    public NurtEvent(Map<String, Object> value) {
        this.value = Collections.unmodifiableMap(value);
    }

    /**
     * Type of the event, for example {@code exec.finished} or {@code job.progress}.
     *
     * @return the type, empty when the server sent none
     */
    public String kind() {
        return Optional.ofNullable(Json.string(value, "t")).orElse("");
    }

    /**
     * Sequence number of the event within its execution.
     *
     * @return the number
     */
    public OptionalLong seq() {
        return value.get("seq") instanceof Long number ? OptionalLong.of(number) : OptionalLong.empty();
    }

    /**
     * Identifier of the background job the event belongs to.
     *
     * @return the identifier
     */
    public Optional<String> jobId() {
        return Optional.ofNullable(Json.string(value, "job_id"));
    }

    /**
     * The whole event.
     *
     * @return the object, read-only
     */
    public Map<String, Object> value() {
        return value;
    }

    /**
     * The whole event as JSON text.
     *
     * @return JSON text
     */
    public String json() {
        return Json.write(value);
    }

    @Override
    public String toString() {
        return json();
    }
}
