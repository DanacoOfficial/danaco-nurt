export interface User {
  id: number;
  name: string;
  greet(): string;
}

interface Internal {
  flag: boolean;
}
