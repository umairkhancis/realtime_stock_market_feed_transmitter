## Slice 1

A small test where transmitter emits an ITCH message as a UDP packet and receiver on other machine with rust dependency free implementation and recover the same data.

## Slice 2

1:1 correspondence between a message and a UDP packet. 
Messages per second = Packets per second

On a 1 Gbps Ethernet Link:
1,000,000,000 / 36 = 27.7M packets per second. 

Due to 1:1 correspondence between message and packet, every 3600 microseconds, a new UDP packet is received. 

Switching between kernel space and user space 27.7M times per second is not feasible.

So, payload data rate is always lower than the link rate.

For feasibility reasons:
Transmitter emits data at 10,000 messages per second i.e. 10,000 packets per second at the uniform rate.
Transmitter emits messages that simulates a real market behavior.

Note: Generate 100,000 synthetic messages as CSV so that we can ask meanigful questions about the market behavior modulating in time at the receiver end.

