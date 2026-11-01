export function recv(msg: { data: Uint8Array }): { data: Uint8Array }[] {
  return [{ data: msg.data }, { data: msg.data }];
}
