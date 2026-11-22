<?php

abstract class Model {
    abstract protected function table(): string;

    public function save() {}
}
