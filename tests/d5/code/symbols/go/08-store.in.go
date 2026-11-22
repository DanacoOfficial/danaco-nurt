package store

type Store struct {
	values map[string]int
}

func New() *Store {
	return &Store{values: map[string]int{}}
}

func (s *Store) Get(key string) (int, bool) {
	value, ok := s.values[key]
	return value, ok
}

func (s *Store) Set(key string, value int) {
	s.values[key] = value
}
