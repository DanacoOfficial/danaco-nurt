export function List({ items }: { items: string[] }) {
  const renderItem = (item: string) => <li key={item}>{item}</li>;
  return <ul>{items.map(renderItem)}</ul>;
}
