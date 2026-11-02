export function recv(msg) {
  try {
    new TextDecoder("latin1");
  } catch (e) {
    throw new Error(`${e.name}: ${e.message}`);
  }
  return null;
}
