package store

import "testing"

func TestGet(t *testing.T) {
	s := New()
	s.Set("a", 1)
	if _, ok := s.Get("a"); !ok {
		t.Fatal("missing key")
	}
}

func BenchmarkSet(b *testing.B) {}
