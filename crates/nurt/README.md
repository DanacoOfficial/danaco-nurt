<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
# nurt

Danaco Nurt is a command shell and language where every change can be undone. The crate is the Rust interface.

The crate runs programs of the Nurt command language and returns their effect
reports.

## Installation

```toml
[dependencies]
nurt = "0.1"
```

Two backends are available as Cargo features. `embedded` (default) runs the
engine inside the calling process through `Embedded`. `process` (default)
talks to a `nurt serve` process through `Nurt` and depends only on
`serde_json`; an application that needs only that client uses
`default-features = false, features = ["process"]`.

The process client starts the `nurt` executable named by `Builder::executable`, by the
`NURT_EXECUTABLE` environment variable or found in `PATH`.

## Usage

```rust,no_run
use nurt::{Nurt, Request};

let nurt = Nurt::builder().spawn()?;
let response = nurt.run(&Request::program(r#"find "/home/user/src" glob: "*.go""#))?;
match response.error() {
    None => println!("{}", response.text().unwrap_or_default()),
    Some(error) => eprintln!("{error}; fix: {:?}", error.fix()),
}
nurt.close()?;
# Ok::<(), nurt::Error>(())
```

Language errors (syntax, binding, assertions) are part of the response and
carry a stable `E_*` code with ready fixes. Call errors (`nurt::Error`) mean
that the call itself failed.

## Features

| Feature | Default | Content |
|---|---|---|
| `process` | yes | Client of `nurt serve --stdio` and of connections opened by the application |

## Documentation

- API reference: <https://docs.rs/nurt>
- Language and commands: <https://nurt.danaco-group.pl>

## License

Apache-2.0. See `LICENSE` and `NOTICE` in the repository
<https://github.com/DanacoOfficial/nurt>.
