let calls = 0;

export function recv(msg: { data: Uint8Array }): { data: Uint8Array } {
  calls += 1;
  if (calls === 1) {
    throw new Error("first delivery fails");
  }
  return { data: msg.data };
}
