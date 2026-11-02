export function recv(msg) {
  return { headers: { ...msg.headers, "x-extra": ["1", "2"] }, data: msg.data };
}
