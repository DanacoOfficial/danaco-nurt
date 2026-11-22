// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
package pl.danacogroup.nurt.internal;

import java.io.File;
import java.io.IOException;
import java.io.InputStream;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.Paths;
import java.security.MessageDigest;
import java.security.NoSuchAlgorithmException;
import java.util.ArrayList;
import java.util.HexFormat;
import java.util.List;
import java.util.Locale;
import java.util.Map;
import java.util.Optional;
import java.util.concurrent.ConcurrentHashMap;
import pl.danacogroup.nurt.CallException;

/**
 * Location of the {@code nurt} executable and of {@code libnurt}.
 *
 * <p>Artifacts bundled in the JAR lie under {@code natives/<os>-<arch>/}
 * together with a file {@code <name>.sha256} holding the hexadecimal SHA-256
 * digest. They are extracted to a temporary directory once per process and
 * used only when the digest matches.
 */
public final class Artifacts {
    /** Variable that names the executable. */
    public static final String EXECUTABLE_VARIABLE = "NURT_EXECUTABLE";

    /** Variable that names the library. */
    public static final String LIBRARY_VARIABLE = "NURT_LIBRARY";

    private static final String NATIVES = "natives/";
    private static final String DIGEST_SUFFIX = ".sha256";
    private static final Map<String, Path> EXTRACTED = new ConcurrentHashMap<>();

    private Artifacts() {
    }

    /**
     * Platform directory name of the running system, for example
     * {@code linux-x86_64}, {@code macos-aarch64} or {@code windows-x86_64}.
     *
     * @return the name
     */
    public static String platform() {
        return platform(System.getProperty("os.name", ""), System.getProperty("os.arch", ""));
    }

    static String platform(String osName, String osArch) {
        String name = osName.toLowerCase(Locale.ROOT);
        String os = name.startsWith("windows") ? "windows"
                : name.startsWith("mac") || name.startsWith("darwin") ? "macos"
                : name.replace(' ', '-');
        String arch = switch (osArch.toLowerCase(Locale.ROOT)) {
            case "amd64", "x86_64", "x64" -> "x86_64";
            case "aarch64", "arm64" -> "aarch64";
            default -> osArch.toLowerCase(Locale.ROOT);
        };
        return os + "-" + arch;
    }

    /**
     * File name of the executable on the running system.
     *
     * @return {@code nurt.exe} on Windows, {@code nurt} elsewhere
     */
    public static String executableName() {
        return platform().startsWith("windows") ? "nurt.exe" : "nurt";
    }

    /**
     * File name of the library on the running system.
     *
     * @return {@code nurt.dll}, {@code libnurt.dylib} or {@code libnurt.so}
     */
    public static String libraryName() {
        String platform = platform();
        if (platform.startsWith("windows")) {
            return "nurt.dll";
        }
        return platform.startsWith("macos") ? "libnurt.dylib" : "libnurt.so";
    }

    /**
     * Finds the executable: the explicit path, the path in
     * {@value #EXECUTABLE_VARIABLE}, the executable bundled for the platform,
     * then the directories of {@code PATH}. An explicit path or a path in the
     * variable must name an existing file.
     *
     * @param explicit configured path, or {@code null}
     * @return the executable
     * @throws CallException of kind {@link CallException.Kind#NOT_FOUND} with
     *     the checked locations
     */
    public static Path locateExecutable(Path explicit) {
        return locate(explicit, System.getenv(EXECUTABLE_VARIABLE), System.getenv("PATH"),
                Artifacts.class.getClassLoader(), executableName(), true);
    }

    /**
     * Finds the library: the explicit path, the path in
     * {@value #LIBRARY_VARIABLE}, then the library bundled for the platform.
     *
     * @param explicit configured path, or {@code null}
     * @return the library
     * @throws CallException of kind {@link CallException.Kind#NOT_FOUND} with
     *     the checked locations
     */
    public static Path locateLibrary(Path explicit) {
        return locate(explicit, System.getenv(LIBRARY_VARIABLE), null,
                Artifacts.class.getClassLoader(), libraryName(), false);
    }

    static Path locate(Path explicit, String variable, String searchPath, ClassLoader loader,
            String fileName, boolean executable) {
        String what = executable ? "nurt executable" : "nurt library";
        String variableName = executable ? EXECUTABLE_VARIABLE : LIBRARY_VARIABLE;
        if (explicit != null) {
            return existing(explicit, what, "the configured path");
        }
        if (variable != null && !variable.isEmpty()) {
            return existing(Paths.get(variable), what, variableName);
        }
        List<String> checked = new ArrayList<>();
        checked.add(variableName + " (not set)");
        String resource = NATIVES + platform() + "/" + fileName;
        Optional<Path> bundled = extract(loader, resource, executable);
        if (bundled.isPresent()) {
            return bundled.get();
        }
        checked.add("bundled " + resource);
        if (searchPath != null) {
            for (String directory : searchPath.split(File.pathSeparator)) {
                if (directory.isEmpty()) {
                    continue;
                }
                Path candidate = Paths.get(directory, fileName);
                if (Files.isRegularFile(candidate)) {
                    return candidate;
                }
                checked.add(candidate.toString());
            }
        }
        throw new CallException(CallException.Kind.NOT_FOUND,
                what + " not found; checked " + String.join(", ", checked)
                        + "; set " + variableName + " or pass the path");
    }

    private static Path existing(Path path, String what, String source) {
        if (Files.isRegularFile(path)) {
            return path;
        }
        throw new CallException(CallException.Kind.NOT_FOUND,
                what + " not found at " + path + " given by " + source + "; correct the path");
    }

    /**
     * Extracts a bundled artifact after checking its digest.
     *
     * @param loader class loader holding the artifact
     * @param resource resource name under {@code natives/}
     * @param executable whether to mark the extracted file executable
     * @return the extracted file, or empty when the artifact is not bundled
     * @throws CallException of kind {@link CallException.Kind#IO} when the
     *     digest is missing or does not match, or the file cannot be written
     */
    static Optional<Path> extract(ClassLoader loader, String resource, boolean executable) {
        Path cached = EXTRACTED.get(resource);
        if (cached != null && Files.isRegularFile(cached)) {
            return Optional.of(cached);
        }
        byte[] content = read(loader, resource);
        if (content == null) {
            return Optional.empty();
        }
        byte[] digestFile = read(loader, resource + DIGEST_SUFFIX);
        if (digestFile == null) {
            throw new CallException(CallException.Kind.IO,
                    "bundled " + resource + " has no " + DIGEST_SUFFIX + " file; rebuild the package");
        }
        String expected = new String(digestFile, StandardCharsets.US_ASCII).trim().split("\\s+")[0];
        String actual = sha256(content);
        if (!actual.equalsIgnoreCase(expected)) {
            throw new CallException(CallException.Kind.IO,
                    "bundled " + resource + " has SHA-256 " + actual + ", expected " + expected
                            + "; the package is damaged");
        }
        try {
            Path directory = Files.createTempDirectory("nurt-");
            Path target = directory.resolve(resource.substring(resource.lastIndexOf('/') + 1));
            Files.write(target, content);
            if (executable && !target.toFile().setExecutable(true, true)) {
                throw new IOException("cannot mark " + target + " executable");
            }
            target.toFile().deleteOnExit();
            directory.toFile().deleteOnExit();
            EXTRACTED.put(resource, target);
            return Optional.of(target);
        } catch (IOException e) {
            throw new CallException(CallException.Kind.IO,
                    "cannot extract bundled " + resource + ": " + e.getMessage(), e);
        }
    }

    private static byte[] read(ClassLoader loader, String resource) {
        try (InputStream in = loader.getResourceAsStream(resource)) {
            return in == null ? null : in.readAllBytes();
        } catch (IOException e) {
            throw new CallException(CallException.Kind.IO,
                    "cannot read bundled " + resource + ": " + e.getMessage(), e);
        }
    }

    /**
     * Hexadecimal SHA-256 digest.
     *
     * @param content the bytes
     * @return the digest in lower case
     */
    public static String sha256(byte[] content) {
        try {
            return HexFormat.of().formatHex(MessageDigest.getInstance("SHA-256").digest(content));
        } catch (NoSuchAlgorithmException e) {
            throw new IllegalStateException("SHA-256 is required of every Java platform", e);
        }
    }
}
