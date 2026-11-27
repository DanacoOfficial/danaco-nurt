<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
<!-- -->
<!-- Module:           docs/integrators/versioning.md -->
<!-- Responsibility:   Documents Versions and compatibility. -->
<!-- Dependencies:     none -->
<!-- Author:           Danaco Official -->
<!-- Version:          0.1.1, 2026-11-27 -->
<!-- License:          see the LICENSE file or https://www.danaco-group.pl -->

# Versions and compatibility

| Item | Version form | Rule |
|---|---|---|
| request, response, report, syntax trees | `nurt: "MAJOR.MINOR"` in each document | a minor version adds optional fields and new values of enumerations marked as extensible; a reader ignores unknown optional fields |
| protocol `nurt-rpc/1` | `initialize` returns the protocol and the features | the host checks the major version and reads the feature list instead of comparing program versions |
| C ABI | `nurt_abi_version()` | the major version must match; the minor version must be at least the one the host was built against |
| tool definition | follows `nurt-request/1` | a new option becomes a new optional argument; tool names and existing arguments do not change within a major version |
| command catalog | `catalog_version` | new commands and parameters are added within a major version; descriptions may change between minor versions |

A host that sends a document with a newer minor version than the console supports receives a language error naming the unsupported field; a newer major version is rejected as unsupported. Hosts pin the major version of the formats they use and test with the conformance suite `nurt-conformance`.

Implementations and integrations that pass the mandatory tests of `nurt-conformance` may use the name "Danaco Nurt Compatible".

The first request of a session is `initialize`; the result lists the protocol, the versions of the formats, the features and the limits:

<!-- schema: rpc -->
```json
{"jsonrpc": "2.0", "id": 0, "method": "initialize", "params": {"protocol": "nurt-rpc/1", "client": {"name": "example-host", "version": "1.0.0"}, "capabilities": {"events": true}}}
```

<!-- schema: rpc#/$defs/initialize_result -->
```json
{
  "protocol": "nurt-rpc/1",
  "version": "1.0.0",
  "abi": 65536,
  "language": "1.0",
  "ast": "1.0",
  "report": "1.0",
  "catalog_rev": "1.0",
  "features": ["events", "preview"],
  "groups": ["core"],
  "limits": {"max_inflight": 4, "max_message_bytes": 16777216}
}
```

<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- Support: support@danaco-group.pl -->
