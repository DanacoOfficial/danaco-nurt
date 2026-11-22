function* ids() {
  yield 1;
}

async function* stream() {
  yield await Promise.resolve(1);
}
