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

/* Constants exposed to Rust (bnd extracts enum values) */
#include <linux/kdev_t.h>
enum {
    RKO_MINORMASK = MINORMASK,
};

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

/* symlink inode operations — extern statics not visible to bnd */
const struct inode_operations *rust_helper_page_symlink_inode_operations(void);
const struct inode_operations *rust_helper_simple_symlink_inode_operations(void);

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

/* ── Mutex helpers ─────────────────────────────────────────────────── */
#include <linux/mutex.h>
void rust_helper___mutex_init(struct mutex *lock, const char *name,
                              struct lock_class_key *key);
void rust_helper_mutex_lock(struct mutex *lock);
void rust_helper_mutex_unlock(struct mutex *lock);
int rust_helper_mutex_trylock(struct mutex *lock);
_Bool rust_helper_mutex_is_locked(struct mutex *lock);

/* ── Spinlock helpers ──────────────────────────────────────────────── */
#include <linux/spinlock.h>
void rust_helper___spin_lock_init(spinlock_t *lock, const char *name,
                                  struct lock_class_key *key);
void rust_helper_spin_lock(spinlock_t *lock);
void rust_helper_spin_unlock(spinlock_t *lock);
int rust_helper_spin_trylock(spinlock_t *lock);
int rust_helper_spin_is_locked(spinlock_t *lock);

/* ── RCU helpers ───────────────────────────────────────────────────── */
#include <linux/rcupdate.h>
void rust_helper_rcu_read_lock(void);
void rust_helper_rcu_read_unlock(void);

/* ── Lockdep helpers ───────────────────────────────────────────────── */
#include <linux/lockdep.h>
void rust_helper_lockdep_register_key(struct lock_class_key *key);
void rust_helper_lockdep_unregister_key(struct lock_class_key *key);

/* ── Waitqueue helpers ─────────────────────────────────────────────── */
void rust_helper___init_waitqueue_head(struct wait_queue_head *wq_head,
                                       const char *name,
                                       struct lock_class_key *key);
void rust_helper___wake_up(struct wait_queue_head *wq_head,
                           unsigned int mode, int nr_exclusive, void *key);

/* ── Task helpers ──────────────────────────────────────────────────── */
#include <linux/sched.h>
#include <linux/sched/task.h>
#include <linux/kthread.h>
struct task_struct *rust_helper_get_current(void);
void rust_helper_get_task_struct(struct task_struct *t);
void rust_helper_put_task_struct(struct task_struct *t);
int rust_helper_kthread_should_stop(void);

/* ── Network helpers ───────────────────────────────────────────────── */
#include <linux/net.h>
#include <net/net_namespace.h>
void *rust_helper_get_net(void *net);
void rust_helper_put_net(void *net);
void rust_helper_set_wq_entry_private(struct wait_queue_entry *wq, void *p);
void *rust_helper_get_wq_entry_private(struct wait_queue_entry *wq);

/* ── Workqueue helpers ─────────────────────────────────────────────── */
#include <linux/workqueue.h>
void rust_helper_init_work_with_key(struct work_struct *work,
                                    work_func_t func, _Bool onstack,
                                    const char *name,
                                    struct lock_class_key *key);

/* schedule() — not inline but not in any traversed partition */
void rust_helper_schedule(void);

/* init_waitqueue_func_entry — inline in wait.h */
void rust_helper_init_waitqueue_func_entry(struct wait_queue_entry *wq_entry,
                                           wait_queue_func_t func);

/* ── Block device helpers ──────────────────────────────────────────── */
#include <linux/blkdev.h>
unsigned long long rust_helper_bdev_nr_sectors(void *bdev);
int rust_helper_sb_min_blocksize(struct super_block *sb, int size);
int rust_helper_sb_set_blocksize(struct super_block *sb, int size);
struct address_space *rust_helper_sb_bdev_mapping(struct super_block *sb);

/* ── iomap helpers ─────────────────────────────────────────────────── */
#include <linux/iomap.h>
void rust_helper_iomap_bio_read_folio(struct folio *folio,
                                      const struct iomap_ops *ops);
void rust_helper_iomap_bio_readahead(struct readahead_control *rac,
                                     const struct iomap_ops *ops);

/* ── memalloc helpers ──────────────────────────────────────────────── */
#include <linux/sched/mm.h>
unsigned int rust_helper_memalloc_nofs_save(void);
void rust_helper_memalloc_nofs_restore(unsigned int flags);

/* ── userspace copy helpers ────────────────────────────────────────── */
#include <linux/uaccess.h>
unsigned long rust_helper_copy_to_user(void *to, const void *from,
                                       unsigned long n);
unsigned long rust_helper_copy_from_user(void *to, const void *from,
                                          unsigned long n);

/* ── delayed_call helper ──────────────────────────────────────────── */
void rust_helper_set_delayed_call(struct delayed_call *call,
                                  void (*fn)(void *), void *arg);

/* ── KUnit helpers ─────────────────────────────────────────────────── */
struct kunit;
struct kunit *rust_helper_kunit_get_current_test(void);
void rust_helper_kunit_mark_failed(void *test);

/* ── Completion helpers ────────────────────────────────────────────── */
#include <linux/completion.h>
void rust_helper_init_completion(struct completion *x);
void rust_helper_complete(struct completion *x);
void rust_helper_wait_for_completion(struct completion *x);
unsigned long rust_helper_wait_for_completion_timeout(
    struct completion *x, unsigned long timeout);

#endif /* _RKO_HELPERS_H */
