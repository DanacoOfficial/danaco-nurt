<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
<!-- -->
<!-- Module:           spec/glossary.md -->
<!-- Responsibility:   Documents Glossary. -->
<!-- Dependencies:     none -->
<!-- Author:           Danaco Official -->
<!-- Version:          0.1.1, 2026-11-27 -->
<!-- License:          see the LICENSE file or https://www.danaco-group.pl -->

# Glossary

Terms with a fixed meaning in the specification and the documentation. Each
term is defined here only; other documents refer to this page.

## Change

A change is a modification of files, provider values or repository state
for which Nurt keeps the previous state in its journal. A change can be
undone until its journal entry expires. Modifications Nurt cannot preserve
and effects on external systems are recorded as irreversible and reported
separately.

The journal protocol, the irreversible markers and the expiry of entries are
specified in [journal.md](journal.md).

<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- Support: support@danaco-group.pl -->
