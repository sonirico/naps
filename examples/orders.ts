interface Msg {
  subject: string;
  headers: Record<string, string[]>;
  data: Uint8Array;
}

export function recv(msg: Msg) {
  const data = new TextDecoder().decode(msg.data);
  const order = JSON.parse(data);
  if (order.status !== "confirmed") {
    return null;
  }
  return { subject: "myapp.orders.confirmed", data };
}
