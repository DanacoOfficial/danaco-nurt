// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
package pl.danacogroup.nurt.internal;

import static org.junit.jupiter.api.Assertions.assertArrayEquals;
import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertThrows;
import static org.junit.jupiter.api.Assertions.assertTrue;

import java.io.File;
import java.io.IOException;
import java.net.URL;
import java.net.URLClassLoader;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import org.junit.jupiter.api.Test;
import org.junit.jupiter.api.io.TempDir;
import pl.danacogroup.nurt.NurtCallException;

class ArtifactsTest {
    private static final byte[] CONTENT = "artifact".getBytes(StandardCharsets.UTF_8);

    @TempDir
    Path temp;

    private ClassLoader loaderWith(String name, byte[] content, String digest) throws IOException {
        Path directory = temp.resolve("natives").resolve(Artifacts.platform());
        Files.createDirectories(directory);
        Files.write(directory.resolve(name), content);
        if (digest != null) {
            Files.writeString(directory.resolve(name + ".sha256"), digest + "  " + name + "\n");
        }
        return new URLClassLoader(new URL[] {temp.toUri().toURL()}, null);
    }

    private static ClassLoader emptyLoader() {
        return new URLClassLoader(new URL[0], null);
    }

    private Path file(String name) throws IOException {
        Path path = temp.resolve(name);
        Files.write(path, CONTENT);
        return path;
    }

    @Test
    void namesThePlatform() {
        assertEquals("linux-x86_64", Artifacts.platform("Linux", "amd64"));
        assertEquals("linux-aarch64", Artifacts.platform("Linux", "aarch64"));
        assertEquals("macos-aarch64", Artifacts.platform("Mac OS X", "aarch64"));
        assertEquals("windows-x86_64", Artifacts.platform("Windows 11", "amd64"));
        assertEquals("windows-aarch64", Artifacts.platform("Windows 11", "arm64"));
    }

    @Test
    void explicitPathComesFirst() throws IOException {
        Path explicit = file("explicit");
        Path variable = file("variable");
        assertEquals(explicit, Artifacts.locate(explicit, variable.toString(), temp.toString(),
                emptyLoader(), "nurt", true));
    }

    @Test
    void variableComesBeforeBundledAndSearchPath() throws IOException {
        Path variable = file("variable");
        ClassLoader loader = loaderWith("nurt-variable-first", CONTENT, Artifacts.sha256(CONTENT));
        assertEquals(variable, Artifacts.locate(null, variable.toString(), temp.toString(),
                loader, "nurt-variable-first", true));
    }

    @Test
    void missingExplicitPathFailsWithoutFallback() throws IOException {
        file("nurt");
        NurtCallException error = assertThrows(NurtCallException.class, () -> Artifacts.locate(
                temp.resolve("missing"), null, temp.toString(), emptyLoader(), "nurt", true));
        assertEquals(NurtCallException.Kind.NOT_FOUND, error.kind());
        assertTrue(error.getMessage().contains("the configured path"));
    }

    @Test
    void missingVariablePathNamesTheVariable() {
        NurtCallException error = assertThrows(NurtCallException.class, () -> Artifacts.locate(
                null, temp.resolve("missing").toString(), null, emptyLoader(), "nurt", true));
        assertTrue(error.getMessage().contains(Artifacts.EXECUTABLE_VARIABLE));
    }

    @Test
    void searchPathIsScannedInOrder() throws IOException {
        Path first = Files.createDirectories(temp.resolve("first"));
        Path second = Files.createDirectories(temp.resolve("second"));
        Files.write(second.resolve("nurt"), CONTENT);
        String searchPath = temp.resolve("empty") + File.pathSeparator + first + File.pathSeparator + second;
        assertEquals(second.resolve("nurt"),
                Artifacts.locate(null, "", searchPath, emptyLoader(), "nurt", true));
    }

    @Test
    void notFoundListsEveryCheckedLocation() {
        NurtCallException error = assertThrows(NurtCallException.class, () -> Artifacts.locate(
                null, null, temp.toString(), emptyLoader(), "nurt", true));
        assertEquals(NurtCallException.Kind.NOT_FOUND, error.kind());
        assertTrue(error.getMessage().contains(temp.resolve("nurt").toString()));
        assertTrue(error.getMessage().contains("bundled natives/"));
        assertTrue(error.getMessage().contains(Artifacts.EXECUTABLE_VARIABLE));
    }

    @Test
    void bundledArtifactIsExtractedWhenTheDigestMatches() throws IOException {
        ClassLoader loader = loaderWith("nurt-good", CONTENT, Artifacts.sha256(CONTENT).toUpperCase());
        Path extracted = Artifacts.locate(null, null, null, loader, "nurt-good", true);
        assertArrayEquals(CONTENT, Files.readAllBytes(extracted));
        assertTrue(Files.isExecutable(extracted) || Artifacts.platform().startsWith("windows"));
        assertEquals(extracted, Artifacts.locate(null, null, null, loader, "nurt-good", true));
    }

    @Test
    void bundledArtifactWithWrongDigestIsRejected() throws IOException {
        ClassLoader loader = loaderWith("nurt-damaged", CONTENT, Artifacts.sha256(new byte[] {1}));
        NurtCallException error = assertThrows(NurtCallException.class,
                () -> Artifacts.locate(null, null, null, loader, "nurt-damaged", true));
        assertEquals(NurtCallException.Kind.IO, error.kind());
        assertTrue(error.getMessage().contains("damaged"));
    }

    @Test
    void bundledArtifactWithoutDigestIsRejected() throws IOException {
        ClassLoader loader = loaderWith("nurt-unsigned", CONTENT, null);
        NurtCallException error = assertThrows(NurtCallException.class,
                () -> Artifacts.locate(null, null, null, loader, "nurt-unsigned", true));
        assertEquals(NurtCallException.Kind.IO, error.kind());
    }

    @Test
    void libraryIsKnownByTheNameOfThePackageAndByTheNameOfTheBuild() {
        assertEquals(java.util.List.of("libnurt.so", "libnurt.so.1", "libnurt_ffi.so"), Artifacts.libraryNames("linux-x86_64"));
        assertEquals(java.util.List.of("libnurt.dylib", "libnurt_ffi.dylib"), Artifacts.libraryNames("macos-aarch64"));
        assertEquals(java.util.List.of("nurt.dll", "nurt_ffi.dll"), Artifacts.libraryNames("windows-x86_64"));
    }

    @Test
    void libraryBuiltAsNurtFfiIsFoundInTheSearchPath() throws IOException {
        Path directory = Files.createDirectory(temp.resolve("lib"));
        Path built = Files.write(directory.resolve("libnurt_ffi.so"), CONTENT);
        Path found = Artifacts.locate(null, null, temp.resolve("none") + File.pathSeparator + directory, emptyLoader(),
                Artifacts.libraryNames("linux-x86_64"), false);
        assertEquals(built, found);
        NurtCallException error = assertThrows(NurtCallException.class, () -> Artifacts.locate(
                null, null, directory.resolve("empty").toString(), emptyLoader(), Artifacts.libraryNames("linux-x86_64"), false));
        assertTrue(error.getMessage().contains("libnurt_ffi.so"), error.getMessage());
        assertTrue(error.getMessage().contains(Artifacts.LIBRARY_VARIABLE), error.getMessage());
    }

    @Test
    void discardRemovesTheExtractedFileAndItsDirectory() throws IOException {
        Path directory = Files.createDirectory(temp.resolve("nurt-extracted"));
        Path target = Files.write(directory.resolve("nurt"), CONTENT);
        Artifacts.discard(target);
        assertFalse(Files.exists(target));
        assertFalse(Files.exists(directory));
        Artifacts.discard(target);
    }

    @Test
    void discardKeepsADirectoryThatHoldsOtherFiles() throws IOException {
        Path directory = Files.createDirectory(temp.resolve("nurt-shared"));
        Path target = Files.write(directory.resolve("nurt"), CONTENT);
        Path other = Files.write(directory.resolve("other"), CONTENT);
        Artifacts.discard(target);
        assertFalse(Files.exists(target));
        assertTrue(Files.exists(other));
    }

    @Test
    void sha256MatchesTheKnownVector() {
        assertEquals("ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
                Artifacts.sha256("abc".getBytes(StandardCharsets.US_ASCII)));
    }
}
