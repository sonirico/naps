export function recv(msg) {
  return { data: new TextDecoder().decode(msg.data).toUpperCase() };
}
