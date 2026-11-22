<?php

class User {
    public function __construct(private string $name) {}

    public function name(): string {
        return $this->name;
    }
}
