// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
package pl.danacogroup.nurt;

/**
 * The call itself failed: the executable or the library was not found, the
 * process ended, the protocol was broken or an argument was rejected.
 *
 * <p>Errors of the language are not call errors; they are part of a
 * successful {@link NurtResponse}.
 */
public class NurtCallException extends RuntimeException {
    private static final long serialVersionUID = 1L;

    /** Kind of a call error. */
    public enum Kind {
        /** No response arrived within the time limit. */
        TIMEOUT(CallStatus.TIMEOUT),
        /** The operation was cancelled. */
        CANCELLED(CallStatus.CANCELLED),
        /** An argument or a request was rejected. */
        INVALID_ARGUMENT(CallStatus.ARG),
        /** The server or the library implements another major version. */
        INCOMPATIBLE(CallStatus.ABI),
        /** A file or the state directory cannot be used. */
        IO(CallStatus.IO),
        /** The server or the library reported an internal failure. */
        INTERNAL(CallStatus.INTERNAL),
        /** The executable or the library was not found. */
        NOT_FOUND(CallStatus.IO),
        /** The connection is closed or the process has ended. */
        DISCONNECTED(CallStatus.IO),
        /** The server sent a message that breaks the protocol. */
        PROTOCOL(CallStatus.INTERNAL);

        private final CallStatus status;

        Kind(CallStatus status) {
            this.status = status;
        }

        /**
         * Status code of the C interface that corresponds to the kind.
         *
         * @return the status
         */
        public CallStatus status() {
            return status;
        }

        /**
         * Kind that corresponds to a status code of the C interface.
         *
         * @param status the status
         * @return the kind
         */
        public static Kind of(CallStatus status) {
            return switch (status) {
                case TIMEOUT -> TIMEOUT;
                case CANCELLED -> CANCELLED;
                case ARG -> INVALID_ARGUMENT;
                case ABI -> INCOMPATIBLE;
                case IO -> IO;
                case INTERNAL -> INTERNAL;
            };
        }
    }

    private final Kind kind;

    /**
     * Creates the exception.
     *
     * @param kind kind of the error
     * @param message what failed and how to correct it
     */
    public NurtCallException(Kind kind, String message) {
        super(message);
        this.kind = kind;
    }

    /**
     * Creates the exception with a cause.
     *
     * @param kind kind of the error
     * @param message what failed and how to correct it
     * @param cause the underlying failure
     */
    public NurtCallException(Kind kind, String message, Throwable cause) {
        super(message, cause);
        this.kind = kind;
    }

    /**
     * Kind of the error.
     *
     * @return the kind
     */
    public Kind kind() {
        return kind;
    }

    /**
     * Status code of the C interface that corresponds to the error.
     *
     * @return the status
     */
    public CallStatus status() {
        return kind.status();
    }
}
