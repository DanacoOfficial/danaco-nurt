// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl

//go:build nurt_cgo

package capi

// A file with an export directive may hold only declarations in its
// preamble, so the C side of the event callback is defined in capi.go.

import "C"

import (
	"runtime/cgo"
	"unsafe"
)

// The user pointer of nurt_exec_start carries the integer value of a
// cgo.Handle; it is never dereferenced.
//
//export nurtGoEvent
func nurtGoEvent(user unsafe.Pointer, event *C.char) {
	sink, ok := cgo.Handle(uintptr(user)).Value().(func([]byte))
	if !ok {
		return
	}
	sink([]byte(C.GoString(event)))
}
