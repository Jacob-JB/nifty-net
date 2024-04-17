A networking library for me to use for my own games

If you want to see a demo, run these binaries
1. `demo_relay`
2. `demo_server`
3. one or more `demo_client`s

The relay listens on port `3001` and drops some packets and adds a delay, then retransmits to the server on port `3000`.
The server listens on port `3000` and prints out messages from clients.
The client sends reliable messages to the server at an interval, and has an mtu of 20 bytes to force fragmentation.

You will see that for however many clients you run each of their messages gets received

Currently there is no functionality for dropping connections
