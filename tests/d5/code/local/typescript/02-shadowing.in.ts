const value = 1;

function shadow(value: number): number {
  return value + 1;
}

function closure(): () => number {
  let count = 0;
  return () => {
    count += 1;
    return count;
  };
}
