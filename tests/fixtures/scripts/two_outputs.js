export function recv(msg) {
  return [
    { subject: "a", data: "first" },
    { data: msg.data },
  ];
}
