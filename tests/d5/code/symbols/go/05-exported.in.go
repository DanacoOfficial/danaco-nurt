package api

func Exported() {}

func unexported() {}

type Public struct{}

type private struct{}
