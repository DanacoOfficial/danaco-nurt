export class Service {
  private items: string[] = [];

  constructor(private readonly name: string) {}

  public add(item: string): void {
    this.items.push(item);
  }

  protected static create(): Service {
    return new Service("default");
  }
}
