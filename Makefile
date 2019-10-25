build:
	cargo build --release

fmt:
	cargo fmt -v

clean:
	cargo clean

.PHONY: build clean fmt