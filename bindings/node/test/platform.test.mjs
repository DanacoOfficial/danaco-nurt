import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, it } from "node:test";
import { internal, packageDirectory } from "./support.mjs";

const { PLATFORM_PACKAGES, candidatePackages, detectHost, executableName, packageName } = internal("platform");

function names(host) {
  return candidatePackages(host).map(packageName);
}

function readJson(...parts) {
  return JSON.parse(readFileSync(join(packageDirectory, ...parts), "utf8"));
}

/** Replaces `process.platform` and `process.arch` while `body` runs. */
function withProcessPlatform(platform, arch, body) {
  const platformDescriptor = Object.getOwnPropertyDescriptor(process, "platform");
  const archDescriptor = Object.getOwnPropertyDescriptor(process, "arch");
  Object.defineProperty(process, "platform", { value: platform, configurable: true });
  Object.defineProperty(process, "arch", { value: arch, configurable: true });
  try {
    return body();
  } finally {
    Object.defineProperty(process, "platform", platformDescriptor);
    Object.defineProperty(process, "arch", archDescriptor);
  }
}

describe("candidatePackages", () => {
  it("selects the package of the C library of a Linux host first", () => {
    assert.deepEqual(names({ platform: "linux", arch: "x64", libc: "glibc" }), ["nurt-linux-x64-gnu", "nurt-linux-x64-musl"]);
    assert.deepEqual(names({ platform: "linux", arch: "x64", libc: "musl" }), ["nurt-linux-x64-musl", "nurt-linux-x64-gnu"]);
    assert.deepEqual(names({ platform: "linux", arch: "arm64", libc: "musl" }), [
      "nurt-linux-arm64-musl",
      "nurt-linux-arm64-gnu",
    ]);
  });

  it("selects the single package of macOS and Windows hosts", () => {
    assert.deepEqual(names({ platform: "darwin", arch: "arm64" }), ["nurt-darwin-arm64"]);
    assert.deepEqual(names({ platform: "darwin", arch: "x64" }), ["nurt-darwin-x64"]);
    assert.deepEqual(names({ platform: "win32", arch: "x64" }), ["nurt-win32-x64"]);
    assert.deepEqual(names({ platform: "win32", arch: "arm64" }), ["nurt-win32-arm64"]);
  });

  it("selects nothing for an unsupported platform", () => {
    assert.deepEqual(names({ platform: "freebsd", arch: "x64" }), []);
    assert.deepEqual(names({ platform: "linux", arch: "ia32", libc: "glibc" }), []);
  });
});

describe("detectHost", () => {
  it("follows process.platform and process.arch", () => {
    const host = withProcessPlatform("win32", "arm64", () => detectHost());
    assert.deepEqual(host, { platform: "win32", arch: "arm64", libc: undefined });
    assert.deepEqual(candidatePackages(host).map(packageName), ["nurt-win32-arm64"]);
  });

  it("reports a C library only on Linux", () => {
    const darwin = withProcessPlatform("darwin", "x64", () => detectHost());
    assert.equal(darwin.libc, undefined);
    const linux = withProcessPlatform("linux", "x64", () => detectHost());
    assert.ok(linux.libc === "glibc" || linux.libc === "musl");
  });
});

describe("executableName", () => {
  it("adds the extension on Windows only", () => {
    assert.equal(executableName("win32"), "nurt.exe");
    assert.equal(executableName("linux"), "nurt");
    assert.equal(executableName("darwin"), "nurt");
  });
});

describe("package manifests", () => {
  const manifest = readJson("package.json");

  it("lists every platform package as an optional dependency of the same version", () => {
    const expected = Object.fromEntries(PLATFORM_PACKAGES.map((entry) => [packageName(entry), manifest.version]));
    assert.deepEqual(manifest.optionalDependencies, expected);
  });

  it("describes each platform package with matching os, cpu and libc fields", () => {
    for (const entry of PLATFORM_PACKAGES) {
      const platform = readJson("npm", entry.id, "package.json");
      assert.equal(platform.name, packageName(entry));
      assert.equal(platform.version, manifest.version);
      assert.deepEqual(platform.os, [entry.os]);
      assert.deepEqual(platform.cpu, [entry.cpu]);
      if (entry.libc === undefined) {
        assert.equal(platform.libc, undefined);
      } else {
        assert.deepEqual(platform.libc, [entry.libc]);
      }
      assert.equal(platform.license, manifest.license);
      assert.deepEqual(platform.files, ["bin/", "LICENSE", "NOTICE", "README.md"]);
      assert.equal(platform.scripts, undefined);
    }
  });

  it("runs no install scripts", () => {
    for (const script of ["preinstall", "install", "postinstall"]) {
      assert.equal(manifest.scripts[script], undefined);
    }
  });
});
