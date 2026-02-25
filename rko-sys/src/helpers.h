/* SPDX-License-Identifier: GPL-2.0 */
/*
 * Declarations for C helper wrappers.
 * These wrap kernel macros/static-inlines into real functions that Rust can link.
 * Parsed by bnd-winmd to auto-generate Rust FFI bindings.
 */

#ifndef _RKO_HELPERS_H
#define _RKO_HELPERS_H

#include <linux/fs.h>
#include <linux/pagemap.h>
#include <linux/highmem.h>
#include <linux/slab.h>

/* folio helpers */
void rust_helper_folio_get(struct folio *folio);
void rust_helper_folio_put(struct folio *folio);
long long rust_helper_folio_pos(struct folio *folio);
unsigned long rust_helper_folio_size(struct folio *folio);
void rust_helper_folio_mark_uptodate(struct folio *folio);
void rust_helper_folio_end_read(struct folio *folio, _Bool success);
void rust_helper_flush_dcache_folio(struct folio *folio);
void *rust_helper_kmap_local_folio(struct folio *folio, unsigned long offset);
void rust_helper_kunmap_local(const void *vaddr);

/* inode helpers */
void *rust_helper_alloc_inode_sb(struct super_block *sb,
                                 struct kmem_cache *cache, gfp_t gfp);
void rust_helper_i_uid_write(struct inode *inode, unsigned int uid);
void rust_helper_i_gid_write(struct inode *inode, unsigned int gid);
void rust_helper_mapping_set_large_folios(struct address_space *mapping);
void rust_helper_inode_set_flags(struct inode *inode,
                                 unsigned int flags, unsigned int mask);
void rust_helper_set_nlink(struct inode *inode, unsigned int nlink);

/* dir_emit helper — wraps macro */
_Bool rust_helper_dir_emit(struct dir_context *ctx, const char *name,
                           int namelen, unsigned long long ino,
                           unsigned char type);

/* device number helper */
unsigned int rust_helper_MKDEV(unsigned int major, unsigned int minor);

/* kmem_cache helper — wraps inline on some configs */
struct kmem_cache *rust_helper_kmem_cache_create(const char *name,
                                                  unsigned int size,
                                                  unsigned int align,
                                                  unsigned long flags,
                                                  void (*ctor)(void *));

/* dentry name helpers */
const unsigned char *rust_helper_dentry_name(const struct dentry *dentry);
unsigned int rust_helper_dentry_name_len(const struct dentry *dentry);

/* ERR_PTR / IS_ERR / PTR_ERR helpers */
void *rust_helper_ERR_PTR(long error);
long rust_helper_PTR_ERR(const void *ptr);
_Bool rust_helper_IS_ERR(const void *ptr);

/* file accessor */
struct inode *rust_helper_file_inode(const struct file *f);

/* inode ops setters (i_fop is in anonymous union, not visible to bnd) */
void rust_helper_inode_set_fop(struct inode *inode,
                               const struct file_operations *fop);
void rust_helper_inode_set_aops(struct inode *inode,
                                const struct address_space_operations *aops);

/* generic_file_read_iter wrapper (inline on some configs) */
long rust_helper_generic_file_read_iter(struct kiocb *iocb,
                                        struct iov_iter *to);

/* is_bad_inode wrapper */
_Bool rust_helper_is_bad_inode(struct inode *inode);

#endif /* _RKO_HELPERS_H */
