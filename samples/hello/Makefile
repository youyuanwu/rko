# Out-of-tree kernel module Makefile
#
# Usage:
#   make                    # build the module
#   make clean              # clean build artifacts
#
# Requires:
#   Rust 1.93.0+ with rust-src component
#
# Variables:
#   KSRC  — kernel source directory   (default: ../../linux)
#   KOUT  — kernel build output       (default: ../../linux_bin)
#   MOUT  — module build output       (default: ./build)

KSRC ?= $(CURDIR)/../../linux
KOUT ?= $(CURDIR)/../../linux_bin
MOUT ?= $(CURDIR)/build
CARGO ?= cargo

KMAKE = $(MAKE) -C $(KSRC) O=$(KOUT) M=$(CURDIR) MO=$(MOUT) LLVM=1

.PHONY: all clean cargo

all: cargo
	$(KMAKE) modules

cargo:
	RUSTC_BOOTSTRAP=1 $(CARGO) build --release -p hello --manifest-path $(CURDIR)/Cargo.toml
	@mkdir -p $(MOUT)
	$(LD) -r --whole-archive $(CURDIR)/target/target/release/libhello.a -o $(MOUT)/hello_rust.o

clean:
	$(KMAKE) clean
	rm -rf $(MOUT)
	$(CARGO) clean --manifest-path $(CURDIR)/Cargo.toml
