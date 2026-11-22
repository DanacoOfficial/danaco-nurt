<?php

enum Suit: string {
    case Hearts = 'H';
    case Spades = 'S';

    public function color(): string {
        return 'Red';
    }
}
