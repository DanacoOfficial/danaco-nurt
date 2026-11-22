export function Select<T>({ options }: { options: T[] }) {
  return <select>{options.length}</select>;
}
