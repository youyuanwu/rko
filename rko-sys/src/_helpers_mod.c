// SPDX-License-Identifier: GPL-2.0
//
// Dummy module wrapper so that _helpers.ko builds cleanly through modpost.
// The only purpose of this file is to supply MODULE_LICENSE / init / exit
// so kbuild does not error out.  The real payload is helpers.o which gets
// linked into per-sample .ko files via ld -r.

#include <linux/module.h>

static int __init _helpers_init(void) { return 0; }
static void __exit _helpers_exit(void) {}

module_init(_helpers_init);
module_exit(_helpers_exit);
MODULE_LICENSE("GPL");
MODULE_DESCRIPTION("RKO C helper wrappers (build-only module)");
