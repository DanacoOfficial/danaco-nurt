// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           bindings/go/backends_nocgo_test.go
// Responsibility:   Source file bindings/go/backends_nocgo_test.go.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//go:build !nurt_cgo

package nurt

import (
	"context"
	"testing"
)

var conformanceBackends = []Backend{BackendProcess}

func TestLibraryBackendNeedsBuildTag(t *testing.T) {
	_, err := New(context.Background(), ClientOptions{Backend: BackendLibrary})
	wantStatus(t, err, StatusArg)
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
