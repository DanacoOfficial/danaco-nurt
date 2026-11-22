<?php

function main() {
    return add(1, 2);
}

function add(int $a, int $b): int {
    return $a + $b;
}
