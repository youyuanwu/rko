/* SPDX-License-Identifier: GPL-2.0 */
/*
 * Declarations for C helper wrappers.
 * These wrap kernel macros/static-inlines into real functions that Rust can link.
 * Parsed by bnd-winmd to auto-generate Rust FFI bindings.
 */

#ifndef _RKO_HELPERS_H
#define _RKO_HELPERS_H

/* No helpers needed yet — krealloc_node_align_noprof and kfree are real
 * exported symbols available via the rko.slab partition.
 */

#endif /* _RKO_HELPERS_H */
