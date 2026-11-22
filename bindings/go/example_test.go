// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl

package nurt_test

import (
	"context"
	"fmt"
	"log"

	nurt "github.com/DanacoOfficial/nurt/bindings/go"
)

func Example() {
	ctx := context.Background()
	c, err := nurt.New(ctx, nurt.ClientOptions{})
	if err != nil {
		log.Fatal(err)
	}
	defer c.Close()

	resp, err := c.Run(ctx, nurt.Request{
		Program: `find "/home/user/project" glob: "**/*.go" |> first 20`,
	})
	if err != nil {
		log.Fatal(err)
	}
	if !resp.OK() {
		log.Fatal(resp.Err())
	}
	fmt.Println(resp.Text())
}

func ExampleExecution_Events() {
	ctx := context.Background()
	c, err := nurt.New(ctx, nurt.ClientOptions{})
	if err != nil {
		log.Fatal(err)
	}
	defer c.Close()

	x, err := c.Start(ctx, nurt.Request{Program: `find "/home/user/project" glob: "**/*.go" |> first 20`})
	if err != nil {
		log.Fatal(err)
	}
	defer x.Close()
	for ev := range x.Events() {
		fmt.Println(ev.Seq, ev.Type)
	}
	if _, err := x.Wait(ctx); err != nil {
		log.Fatal(err)
	}
}
