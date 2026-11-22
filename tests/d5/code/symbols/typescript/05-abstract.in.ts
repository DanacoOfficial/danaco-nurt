export abstract class Shape {
  abstract area(): number;

  describe(): string {
    return `area ${this.area()}`;
  }
}

class Square extends Shape {
  area(): number {
    return 1;
  }
}
