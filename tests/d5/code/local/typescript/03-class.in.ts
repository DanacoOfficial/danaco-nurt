export class Counter {
  private value = 0;

  bump(step: number): number {
    this.value += step;
    return this.double(step);
  }

  double(step: number): number {
    return step * 2;
  }
}
