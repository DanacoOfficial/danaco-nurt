<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# Security Policy

## Reporting a vulnerability

Suspected vulnerabilities are reported by e-mail to support@danaco-group.pl with
the subject line starting with `[security] nurt`, not in a public issue, pull
request or discussion.

A report contains, as far as available:

- the affected version, commit or release artifact;
- the operating system and architecture;
- a description of the issue and its impact;
- steps to reproduce, a proof of concept or a minimal Nurt program;
- any known mitigations.

Reports may be written in English or Polish.

## Scope

The following are in scope:

- the `nurt` executable and the `libnurt` library built from this repository;
- the Model Context Protocol server and the language server provided by `nurt`;
- the journal of changes, including recovery and undo;
- release artifacts, installers and packages published by Danaco Holding Group
  Sp. z o.o.;
- the continuous integration configuration of this repository.

Examples of relevant issues: execution of effects that were not shown or not
permitted by the program check, escapes from the configured working scope,
loss or corruption of journal data, unsafe extraction of archives, command
injection through argument quoting, memory safety defects in the C interface,
and disclosure of credentials or environment data in reports.

The following are out of scope:

- vulnerabilities in third-party programs started by a Nurt program on the
  user's request, which are reported to their maintainers;
- issues that require an attacker to already control the user account or the
  configuration files of the user;
- denial of service through programs that intentionally consume resources
  beyond configured limits.

Vulnerabilities in third-party Rust crates used by Danaco Nurt are also
reported to the maintainers of the crate and, where applicable, to the RustSec
advisory database.

## Handling process

| Step | Target time |
|---|---|
| Acknowledgement of the report | within 3 business days |
| Initial assessment and severity rating | within 10 business days |
| Fix for a critical vulnerability | within 7 days of confirmation |
| Coordinated public disclosure | within 90 days of the report, or earlier when a fix is released |

The reporter is informed of the progress of the assessment and of the planned
release date. Fixed vulnerabilities are described in a security advisory of the
repository and in [CHANGELOG.md](CHANGELOG.md); a CVE identifier is requested
where appropriate. Reporters are credited in the advisory unless they ask
otherwise.

## Verifying a release

Every release has the file `checksums.txt` with the SHA-256 digests of its
archives and the detached OpenPGP signature `checksums.txt.asc` of that file.
The signature is made with the key

```text
Danaco Nurt (Danaco Holding Group Sp. z o.o.) <support@danaco-group.pl>
5BEC C5F4 067A 8392 8961  8DB7 65E3 E606 1204 E0FC
```

The public key is the file `packaging/release/danaco-nurt.asc`. The release is
verified with:

```sh
gpg --import danaco-nurt.asc
gpg --verify checksums.txt.asc checksums.txt
sha256sum --check checksums.txt
```

The tag of every release is signed with the same key and is verified with
`git tag -v`. The executable `nurt.exe` carries an Authenticode signature. Its
certificate is the file `packaging/release/danaco-nurt-codesign.pem` with the
subject `C=PL, O=Danaco Holding Group Sp. z o.o., CN=Danaco Nurt`. Danaco
Holding Group Sp. z o.o. issued this certificate itself, so Windows does not
chain it to a public certification authority and reports the publisher as
unverified until the certificate is replaced.

### Verifying the history

Every commit of the `main` branch is signed with a separate OpenPGP key that is
used for commits only. The release key above has certified it.

```text
Danaco Nurt Commit Signing (Danaco Holding Group Sp. z o.o.) <support@danaco-group.pl>
62F8 7C0F 4898 35EF 1C94  EF96 0664 7EE7 17FB BC62
```

The public key is the file `packaging/release/danaco-nurt-commit-signing.asc`.
The history is verified with:

```sh
gpg --import danaco-nurt.asc danaco-nurt-commit-signing.asc
git log --show-signature
git verify-commit <commit>
```

## Supported versions

Version 0.1.0 is the first release. Until the first stable release, security
fixes are applied to the `main` branch and published in a new release. The
support period of stable and long-term support releases will be stated in this
file when they are published.
