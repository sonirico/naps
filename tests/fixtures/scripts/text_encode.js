export function recv(msg) {
  return { data: new TextEncoder().encode("ñ") };
}
