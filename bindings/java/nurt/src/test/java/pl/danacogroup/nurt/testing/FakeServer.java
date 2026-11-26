// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
package pl.danacogroup.nurt.testing;

import java.io.BufferedReader;
import java.io.IOException;
import java.io.InputStreamReader;
import java.io.OutputStreamWriter;
import java.io.PrintWriter;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.StandardOpenOption;
import java.util.ArrayList;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import pl.danacogroup.nurt.internal.Json;

/**
 * Server for tests that speaks {@code nurt-rpc/1} on its standard streams.
 *
 * <p>The program text of {@code run} selects the behavior:
 * {@code crash} ends the process, {@code garble} sends invalid JSON,
 * {@code slow} answers only after {@code cancel}, {@code events} sends
 * events before the answer, {@code error:<code>} answers with a JSON-RPC
 * error, {@code language} answers with a language error and
 * {@code hold:<n>} keeps answers until {@code n} requests are held and then
 * answers them in reverse order. Other programs succeed with the program as
 * the rendered text. Other methods return their name and parameters; text
 * methods return {@code <method>:<params>}.
 *
 * <p>Environment: {@code FAKE_PROTOCOL} replaces the protocol named in
 * {@code initialize}; {@code FAKE_LOG} names a file that receives the name
 * of every received method, one per line.
 */
public final class FakeServer {
    private final PrintWriter out;
    private final Path log;
    private final Map<Long, Map<String, Object>> slow = new LinkedHashMap<>();
    private final List<Map<String, Object>> held = new ArrayList<>();

    private FakeServer(PrintWriter out, Path log) {
        this.out = out;
        this.log = log;
    }

    /**
     * Serves standard input until it ends.
     *
     * @param args not used
     * @throws IOException when a stream fails
     */
    public static void main(String[] args) throws IOException {
        String logName = System.getenv("FAKE_LOG");
        PrintWriter out = new PrintWriter(new OutputStreamWriter(System.out, StandardCharsets.UTF_8), true);
        FakeServer server = new FakeServer(out, logName == null ? null : Path.of(logName));
        try (BufferedReader in = new BufferedReader(new InputStreamReader(System.in, StandardCharsets.UTF_8))) {
            String line;
            while ((line = in.readLine()) != null) {
                server.handle(Json.object(Json.parse(line)));
            }
        }
    }

    private void handle(Map<String, Object> request) throws IOException {
        String method = (String) request.get("method");
        if (log != null) {
            Files.writeString(log, method + "\n", StandardOpenOption.CREATE, StandardOpenOption.APPEND);
        }
        Map<String, Object> params = Json.object(request.get("params"));
        switch (method) {
            case "initialize" -> {
                String protocol = System.getenv("FAKE_PROTOCOL");
                reply(request, map("protocol", protocol == null ? "nurt-rpc/1" : protocol, "version", "0.1.0"));
            }
            case "shutdown" -> {
                reply(request, map());
                out.flush();
                Runtime.getRuntime().halt(0);
            }
            case "run" -> run(request, Json.string(params, "program"));
            case "cancel" -> cancel(request, params);
            case "page" -> reply(request, envelope(report("ok")));
            case "print", "help" -> reply(request, map("text", method + ":" + Json.write(params)));
            case "describe" -> reply(request, map("description", method + ":" + Json.write(params)));
            default -> reply(request, map("method", method, "params", params));
        }
    }

    private void run(Map<String, Object> request, String program) {
        String text = program == null ? "" : program;
        if (text.equals("crash")) {
            out.flush();
            Runtime.getRuntime().halt(3);
        } else if (text.equals("garble")) {
            out.print("{not json\n");
            out.flush();
        } else if (text.equals("slow")) {
            slow.put((Long) request.get("id"), request);
        } else if (text.equals("events")) {
            Object id = request.get("id");
            send(map("jsonrpc", "2.0", "method", "nurt/event", "params", map("t", "job.output", "seq", 0L, "id", "j-1")));
            for (long seq = 1; seq <= 3; seq++) {
                send(map("jsonrpc", "2.0", "method", "nurt/event",
                        "params", map("t", "progress", "seq", seq, "request", String.valueOf(id))));
            }
            send(map("jsonrpc", "2.0", "method", "nurt/event",
                    "params", map("t", "exec.finished", "seq", 4L, "request", String.valueOf(id))));
            reply(request, envelope(report("ok")));
        } else if (text.startsWith("error:")) {
            send(map("jsonrpc", "2.0", "id", request.get("id"),
                    "error", map("code", Long.parseLong(text.substring(6)), "message", "rejected")));
        } else if (text.equals("language")) {
            Map<String, Object> report = report("failed");
            report.put("error", map("code", "E_SYNTAX", "message", "unclosed list",
                    "fix", List.of("find \"/srv/app\" glob: [\"*.go\"]"), "help", "lists"));
            reply(request, envelope(report));
        } else if (text.startsWith("hold:")) {
            held.add(request);
            if (held.size() == Integer.parseInt(text.substring(5).split("/")[0])) {
                for (int i = held.size() - 1; i >= 0; i--) {
                    Map<String, Object> waiting = held.get(i);
                    Map<String, Object> report = report("ok");
                    report.put("kind", Json.string(waiting.get("params"), "program"));
                    reply(waiting, envelope(report));
                }
                held.clear();
            }
        } else {
            Map<String, Object> response = envelope(report("ok"));
            response.put("rendered", map("format", "text", "text", text));
            reply(request, response);
        }
    }

    private void cancel(Map<String, Object> request, Map<String, Object> params) {
        reply(request, map());
        Object id = params.get("request_id");
        if (id instanceof Long requestId && slow.containsKey(requestId)) {
            slow.remove(requestId);
            send(map("jsonrpc", "2.0", "id", requestId,
                    "error", map("code", -32002L, "message", "the request was cancelled",
                            "data", map("status", "NURT_E_CANCELLED", "response", envelope(report("rolled_back"))))));
        }
    }

    private static Map<String, Object> report(String status) {
        return map("nurt", "1.0", "status", status, "kind", "find");
    }

    private static Map<String, Object> envelope(Map<String, Object> report) {
        return map("nurt", "1.0", "report", report);
    }

    private void reply(Map<String, Object> request, Object result) {
        send(map("jsonrpc", "2.0", "id", request.get("id"), "result", result));
    }

    private void send(Map<String, Object> message) {
        out.print(Json.write(message) + "\n");
        out.flush();
    }

    private static Map<String, Object> map(Object... namesAndValues) {
        Map<String, Object> map = new LinkedHashMap<>();
        for (int i = 0; i + 1 < namesAndValues.length; i += 2) {
            map.put((String) namesAndValues[i], namesAndValues[i + 1]);
        }
        return map;
    }
}
