export function identity<T>(value: T): T {
  return value;
}

export class Box<T> {
  constructor(public value: T) {}

  map<U>(f: (v: T) => U): Box<U> {
    return new Box(f(this.value));
  }
}
