// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           examples/c/hello.c
// Responsibility:   Source file examples/c/hello.c.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

/*
 * Smallest program that uses the C library nurt-lib: it checks the interface
 * version of the header against the library and prints the version of the
 * library. The release checks build it against the archive nurt-lib:
 *
 *   cc hello.c $(pkg-config --cflags --libs nurt) -o hello
 *   cc hello.c $(pkg-config --static --cflags --libs nurt) -o hello   (musl)
 *
 * The program exits with status 0 and prints one line "nurt <version>".
 */

#include "nurt.h"

#include <stdio.h>

int main(void) {
    if (nurt_abi_check(NURT_ABI_VERSION) != NURT_OK) {
        fprintf(stderr, "the library does not implement the interface of nurt.h\n");
        return 1;
    }
    const char *version = nurt_version();
    if (version == NULL) {
        fprintf(stderr, "nurt_version returned no string\n");
        return 1;
    }
    printf("nurt %s\n", version);
    return 0;
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
