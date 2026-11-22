export const handler = async (event: Event): Promise<void> => {
  console.log(event.type);
};

let count: number = 0;

const increment = (): number => ++count;
