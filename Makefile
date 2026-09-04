DATA_DIR := data

.PHONY: clean-data

## clean-data: remove generated `data/` directory
clean-data:
	rm -rf $(DATA_DIR)
