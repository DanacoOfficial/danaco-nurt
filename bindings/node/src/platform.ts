// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
import { readdirSync } from "node:fs";

/** C library of a Linux system. */
export type Libc = "glibc" | "musl";

/** Operating system, processor architecture and C library of a host. */
export interface HostPlatform {
  /** Value of `process.platform`. */
  readonly platform: string;
  /** Value of `process.arch`. */
  readonly arch: string;
  /** C library on Linux; undefined elsewhere or when unknown. */
  readonly libc?: Libc | undefined;
}

/** Package that carries the `nurt` executable for one platform. */
export interface PlatformPackage {
  /** Platform identifier, the suffix of the package name. */
  readonly id: string;
  /** Value of `process.platform` and of the `os` field of the package. */
  readonly os: string;
  /** Value of `process.arch` and of the `cpu` field of the package. */
  readonly cpu: string;
  /** C library and value of the `libc` field of the package (Linux only). */
  readonly libc?: Libc;
  /** Rust target triple of the executable. */
  readonly target: string;
}

/** Prefix of the names of the platform packages. */
export const PLATFORM_PACKAGE_PREFIX = "nurt-";

/** Platform packages, one per supported platform. */
export const PLATFORM_PACKAGES: readonly PlatformPackage[] = [
  { id: "linux-x64-gnu", os: "linux", cpu: "x64", libc: "glibc", target: "x86_64-unknown-linux-gnu" },
  { id: "linux-x64-musl", os: "linux", cpu: "x64", libc: "musl", target: "x86_64-unknown-linux-musl" },
  { id: "linux-arm64-gnu", os: "linux", cpu: "arm64", libc: "glibc", target: "aarch64-unknown-linux-gnu" },
  { id: "linux-arm64-musl", os: "linux", cpu: "arm64", libc: "musl", target: "aarch64-unknown-linux-musl" },
  { id: "darwin-x64", os: "darwin", cpu: "x64", target: "x86_64-apple-darwin" },
  { id: "darwin-arm64", os: "darwin", cpu: "arm64", target: "aarch64-apple-darwin" },
  { id: "win32-x64", os: "win32", cpu: "x64", target: "x86_64-pc-windows-msvc" },
  { id: "win32-arm64", os: "win32", cpu: "arm64", target: "aarch64-pc-windows-msvc" },
];

/** Returns the npm package name of a platform package. */
export function packageName(entry: PlatformPackage): string {
  return `${PLATFORM_PACKAGE_PREFIX}${entry.id}`;
}

/** Returns the file name of the `nurt` executable on a platform. */
export function executableName(platform: string): string {
  return platform === "win32" ? "nurt.exe" : "nurt";
}

/**
 * Returns the platform packages that can run on a host, best match first.
 *
 * On Linux the package for the detected C library comes first and the package
 * for the other C library second, because a package manager that ignores the
 * `libc` field may install either.
 */
export function candidatePackages(host: HostPlatform): PlatformPackage[] {
  const matching = PLATFORM_PACKAGES.filter((entry) => entry.os === host.platform && entry.cpu === host.arch);
  const rank = (entry: PlatformPackage): number => (entry.libc === undefined || entry.libc === host.libc ? 0 : 1);
  return matching.sort((left, right) => rank(left) - rank(right));
}

/** Describes the current host. */
export function detectHost(): HostPlatform {
  return {
    platform: process.platform,
    arch: process.arch,
    libc: process.platform === "linux" ? detectLibc() : undefined,
  };
}

/** Directory that holds the dynamic loader on Linux. */
const LOADER_DIRECTORY = "/lib";
/** File name prefix of the dynamic loader of musl. */
const MUSL_LOADER_PREFIX = "ld-musl-";

function detectLibc(): Libc {
  try {
    return readdirSync(LOADER_DIRECTORY).some((name) => name.startsWith(MUSL_LOADER_PREFIX)) ? "musl" : "glibc";
  } catch {
    return "glibc";
  }
}
