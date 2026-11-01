function recv(msg)
  local headers = {}
  for key, values in msg.headers do
    headers[key] = values
  end
  headers["x-extra"] = { "1", "2" }
  return { headers = headers, data = msg.data }
end
