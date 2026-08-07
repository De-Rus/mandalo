### Echo socket
{{wsUrl}}/ws/echo
x-trace: mandalo

>> hola
hola desde Mándalo

>> json
{"op": "sub", "channel": "prices"}

### Negotiated subprotocol
# The mock offers `v2` and `chat`; it answers the handshake with the first one it
# also speaks, and says so in the connected event.
{{wsUrl}}/ws/protocol
subprotocol: v2
subprotocol: chat

>> hola
hola

### Echo socket, resumed
# Reconnects on its own and pings every 20 seconds, which is what a long-lived
# socket behind a proxy needs.
{{wsUrl}}/ws/echo
auto-reconnect: true
ping-interval: 20

>> ping
ping
