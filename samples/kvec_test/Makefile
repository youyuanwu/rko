# Out-of-tree kernel module Makefile for kvec_test
#
# Usage:
#   make                    # build the module
#   make clean              # clean build artifacts
#   make test               # build + test in QEMU
#
# Variables:
#   KSRC  — kernel source directory   (default: ../../linux)
#   KOUT  — kernel build output       (default: ../../linux_bin)
#   MOUT  — module build output       (default: ./build)
#   KVM   — enable KVM acceleration   (default: 1, set 0 to disable)

KSRC ?= $(CURDIR)/../../linux
KOUT ?= $(CURDIR)/../../linux_bin
MOUT ?= $(CURDIR)/build
CARGO ?= cargo
KVM ?= 1

KMAKE = $(MAKE) -C $(KSRC) O=$(KOUT) M=$(CURDIR) MO=$(MOUT) LLVM=1

# Rust source files that the static library depends on
RUST_SRCS := $(CURDIR)/kvec_test.rs $(CURDIR)/Cargo.toml $(CURDIR)/cargo-kernel.toml \
	$(shell find $(CURDIR)/../../rko-core/src $(CURDIR)/../../rko-sys/src -name '*.rs')

.PHONY: all clean test

all: $(MOUT)/kvec_test_rust.o
	$(KMAKE) modules

INITRAMFS = $(MOUT)/initramfs.cpio.gz
SCRIPTS = $(CURDIR)/../../scripts

test: all $(INITRAMFS)
	$(SCRIPTS)/run-qemu-test.sh $(KOUT)/arch/x86/boot/bzImage $(INITRAMFS) $(MOUT)/test.log $(KVM)

$(INITRAMFS): $(MOUT)/kvec_test.ko $(SCRIPTS)/init-kvec-test.sh $(SCRIPTS)/make-initramfs.sh
	$(SCRIPTS)/make-initramfs.sh $< $@ $(SCRIPTS)/init-kvec-test.sh

$(MOUT)/libkvec_test.a: $(RUST_SRCS)
	@mkdir -p $(MOUT)
	RUSTC_BOOTSTRAP=1 $(CARGO) --config $(CURDIR)/cargo-kernel.toml --config 'build.target="$(KOUT)/scripts/target.json"' -Z unstable-options build --release -p kvec_test --manifest-path $(CURDIR)/Cargo.toml --artifact-dir $(MOUT)

$(MOUT)/kvec_test_rust.o: $(MOUT)/libkvec_test.a
	$(LD) -r --whole-archive $< -o $@

clean:
	$(KMAKE) clean
	rm -rf $(MOUT)
	$(CARGO) clean --manifest-path $(CURDIR)/Cargo.toml
