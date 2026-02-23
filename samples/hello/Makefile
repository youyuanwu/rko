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

# Rust source files that the static library depends on
RUST_SRCS := $(CURDIR)/hello.rs $(CURDIR)/Cargo.toml $(CURDIR)/cargo-kernel.toml \
	$(shell find $(CURDIR)/../../rko-sys/src -name '*.rs')

.PHONY: all clean

all: $(MOUT)/hello_rust.o
	$(KMAKE) modules

$(MOUT)/libhello.a: $(RUST_SRCS)
	@mkdir -p $(MOUT)
	RUSTC_BOOTSTRAP=1 $(CARGO) --config $(CURDIR)/cargo-kernel.toml --config 'build.target="$(KOUT)/scripts/target.json"' -Z unstable-options build --release -p hello --manifest-path $(CURDIR)/Cargo.toml --artifact-dir $(MOUT)

$(MOUT)/hello_rust.o: $(MOUT)/libhello.a
	$(LD) -r --whole-archive $< -o $@

clean:
	$(KMAKE) clean
	rm -rf $(MOUT)
	$(CARGO) clean --manifest-path $(CURDIR)/Cargo.toml
