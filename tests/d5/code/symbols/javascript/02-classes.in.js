class Animal {
  constructor(name) {
    this.name = name;
  }

  speak() {}

  static create(name) {
    return new Animal(name);
  }
}

class Dog extends Animal {
  speak() {
    return "woof";
  }
}
