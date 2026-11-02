interface Msg {
  subject: string;
  headers: Record<string, string[]>;
  data: Uint8Array;
}

export function recv(msg: Msg): Msg {
  return msg;
}
