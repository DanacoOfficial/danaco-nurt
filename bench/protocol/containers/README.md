<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
# Container images of nurt-bench

| Image | File | Tasks | Shell |
|---|---|---|---|
| Linux | [`linux/Containerfile`](linux/Containerfile) | `platform` `linux` and `cross` | bash 5 with GNU coreutils, findutils, grep, sed, gawk, tar, zip, Git, Python 3 |
| Windows | [`windows/Containerfile`](windows/Containerfile) | `platform` `windows` and `cross` | PowerShell 7 with Git and Python 3 |

macOS tasks run on a macOS machine of the continuous integration with the
system bash, without a container.

## Build

The build context holds the release executables and the `bench` directory:

```
mkdir context
cp target/x86_64-unknown-linux-musl/release/nurt target/x86_64-unknown-linux-musl/release/nurt-bench context/
cp -r bench context/
docker build -f bench/protocol/containers/linux/Containerfile -t nurt-bench-linux context
```

## Run

```
docker run --rm -e OPENAI_API_KEY -v /home/user/runs:/opt/nurt-bench/runs nurt-bench-linux \
  run --condition bash --model bench/protocol/models/openai.toml --out runs
```

A published series names the digest of the image (`docker inspect --format
'{{.Id}}' nurt-bench-linux`).
