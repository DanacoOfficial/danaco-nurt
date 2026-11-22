<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
# Event log fixtures

Event XML in the form that `EvtRender` with `EvtRenderEventXml` produces.
The providers, computer names and payloads are invented for the tests; no
file contains data of a real system.

| File | Content |
|---|---|
| `service_state.xml` | classic event with named `Data`, qualifiers, correlation, execution and security elements |
| `unnamed_data.xml` | error with unnamed `Data`, `Binary` and entity references, written on one line |
| `user_data.xml` | `LogAlways` event of the `Security` channel with nested `UserData` |
