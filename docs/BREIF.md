# Brief

Build a high speed signal acquisition pipeline which is generating signal at 100 Mbps and then transfer it to another machine over UDP and then expose it over the web socket for consumer to subscribe and build apps on top of it. Signals feed is from stock market. Between the UDP packet receiving and web socket exposition there is must be 0% packet loss. Implementation should be non-standard crates dependency free.

## Transmitter

This project is the transmitter part implementation of the whole pipeline.

