# A UDP networking library for me to use for my own games

I needed a networking library to use for my (bevy)[https://bevyengine.org] games.
After trying a few options like (bevy_renet)[https://crates.io/crates/bevy_renet]
and (laminar)[https://crates.io/crates/laminar] I decided to have a go at it myself.

What I ended up with is pretty bare bones and is described below.

I don't really intend for anyone to use this for themselves,
but is here if you wish to learn or use it.

# Architecture

### Connections

Firstly, there isn't a "client" or "server".
There are just *sockets* that make *connections* to other sockets.
If you wan't a client server model that depends on how the library is used.

UDP is connectionless, so a socket considers a connection to exist if heartbeat messages
are being received continuously. A connection is dropped if no messages are received after
a timeout or if a disconnect message is received.

Connections can be created in one of two ways.
The initializing party "opens" a connection and starts sending handshake messages,
or the receiving party receives those handshakes and decides to respond.
After that heartbeats are continuously sent as previously described.

Handshake packets contain a protocol id, and only the correct protocol id will be accepted.
The purpose of this is so that different releases or
versions of your application won't talk to each other.

When a socket receives any message, it first checks if it has a connection from that address or if
it needs to create one.
Then if there is a connection to process the packet it is handed to that connection to process.

### Packets

Packets are layed out like this.
- The first 16 bits describe the length of the next blob
- A blob contains some piece of data
- After that blob it repeats, the next 16 bits describing the length of the next blob
- The only exception to this is if the first 16 bits of the packet are zero.
The packet is a handshake packet and the next 8 bytes contain the protocol id.

### Blobs

One packet can contain any number of blobs.
This is so lots of small pieces of data get grouped together when possible.

Once a packet is received all the blobs get processes separately.

There are five types of blobs.
- A message fragment
- A heartbeat
- A heartbeat response
- A message fragment acknowledgement
- A disconnect message

### Message fragmentation

When a message is too large to be sent in single packet with the configured mtu
it gets fragmented. All messages are sent as fragments,
but most fragments will simply contain the full message.
A fragment contains a fragmentation id, how long the full message is,
what portion of the message the current fragment covers and if the message is reliable.

When a connection receives a fragment it checks to see if it already has a partially constructed
message with that fragmentation id, or it creates one.
The rules of when these partially constructed messages get flushed or forgotten depends on
if the message is reliable.

### Unreliable messages

If a fragment is marked as unreliable then no acknowledgement is sent
and the message is stored until it is fully constructed, or a timeout is reached
and the partially constructed message is forgotten.

### Reliable messages

If a fragment is marked as reliable then an acknowledgement is immediately sent.
The acknowledgement says that that portion of the message has been received.
It is the sending party's responsibility to ensure the reliable message has been received.
If they don't get an acknowldgement for some portion of the message then that part is retransmitted
after a timeout.

Sometimes a reliable message gets completed but the packet with the acknowledgement can get dropped,
meaning that the message on the receiving end gets flushed but then a fragment of it gets
retransmitted and received again. To counter this the receiving party remembers all completed
fragmentation ids, keeping them for some time proportional to the round trip time.

### Heartbeats and heartbeat responses

Heartbeat messages serve two purposes. One is to keep the connection alive and the other is to
calculate round trip time.

Each heartbeat contains a time stamp and each heartbeat gets acknowledged with a heartbeat response
containing that time stamp. The connection can then compare the timestamp of heartbeat repsonses it
receives with when it receives them and estimate a round trip time.

### Disconnect message

Once a party decides to terminate a connection it will stop sending heartbeats.
The other party will continue to send packets until it reaches it's timeout but these
packets will be ignored as they're not heartbeat packets.

To recognise that the other pary has disconnected sooner, a courtesy disconnect message is sent.
It could get dropped but the worst that can happen is that the other party has to wait for timeout.

# Bevy wrapper

I've also included the bevy wrapper I use. Feel free to use your own.

A `NetSocket` component is a wrapper for a socket, and child `Connection` components are created with
methods for sending to and receiving packets from those connections.
`Connection` and `Disconnection` events are also provided.

Again, it is up to you to implement a client server model if you wish to.

### Typed messages

If you wish you can use the `TypedMessagePlugin` to send and receive messages by their types
by accessing the `TypedMessages<T>` resource for that type.
This is done by prefixing each message with an id for that type.

This is used instead of using the send and receive methods on the `Connection` componets
to send and receive raw bytes.

# Examples

You can see how to use the library and it's bevy wrapper in the two provided demo crates.

### regular demo

There are three binaries for the regular demo
- `demo_relay`
- `demo_server`
- `demo_client`

The relay listens on port `3001` and forwards packets to and from `3000` which the server listens on,
adding in packet delay and loss to demonstrate how the library reacts to packet loss.

Run the relay, then the server, then one or many clients.

### Bevy demo

The bevy demo consists of two binaries
- `bevy_demo_server`
- `bevy_demo_client`

This demo shows how to use the api provided by the bevy wrapper.

### Typed demo

The bevy demo consists of two binaries
- `typed_demo_server`
- `typed_demo_client`

This demo shows how to use the bevy wrapper, but with the typed messages api instead.
