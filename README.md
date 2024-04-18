A networking library for me to use for my own games

If you want to see a demo, run these binaries
1. `demo_relay`
2. `demo_server`
3. one or more `demo_client`s

The relay listens on port `3001` and drops some packets and adds a delay, then retransmits to the server on port `3000`.
The server listens on port `3000` and prints out messages from clients.
The client sends several reliable messages to the server, with a small mtu to force fragmentation.
Once they are all received it disconnects.

All the messages will be received reliably,
and if the courtesy disconnect packet doesn't reach the server it will just time out the client
