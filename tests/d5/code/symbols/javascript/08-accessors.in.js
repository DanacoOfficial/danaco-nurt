class Box {
  #value = 0;

  get value() {
    return this.#value;
  }

  set value(next) {
    this.#value = next;
  }
}
