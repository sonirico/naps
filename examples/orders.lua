function recv(msg)
  if not string.find(msg.data, '"status":"confirmed"', 1, true) then
    return nil
  end
  return { subject = "myapp.orders.confirmed", data = msg.data }
end
