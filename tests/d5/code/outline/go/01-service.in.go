package service

type Service struct {
	name string
}

func (s *Service) Start() error {
	return nil
}

func (s *Service) Stop() {}

func New(name string) *Service {
	return &Service{name: name}
}
