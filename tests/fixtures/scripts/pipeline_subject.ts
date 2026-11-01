export function recv(msg: { data: Uint8Array }): { subject: string; data: Uint8Array } {
  return { subject: "out.c", data: msg.data };
}
