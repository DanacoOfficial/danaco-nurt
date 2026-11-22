<?php

class Registry {
    private static $items = [];

    public static function add($item) {
        self::$items[] = $item;
    }
}
