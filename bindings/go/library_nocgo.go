// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl

//go:build !nurt_cgo

package nurt

func newLibraryBackend(ClientOptions) (backend, error) {
	return nil, newError(StatusArg, "the library backend is not built in; build with -tags nurt_cgo and libnurt registered with pkg-config", nil)
}
