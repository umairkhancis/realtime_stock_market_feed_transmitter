.PHONY: start clean-data

## transmit: send data/feed.csv over UDP, one message per datagram
start:
	cargo run -- transmit

## clean-data: remove generated `data/` directory
DATA_DIR := data
clean-data:
	rm -rf $(DATA_DIR)
