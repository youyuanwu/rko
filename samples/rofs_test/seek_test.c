/* seek_test.c — SEEK_DATA/SEEK_HOLE test for rofs_test filesystem */
#define _GNU_SOURCE
#include <fcntl.h>
#include <unistd.h>
#include <stdio.h>

int main(int argc, char **argv) {
    if (argc < 2) { fprintf(stderr, "usage: seek_test <file>\n"); return 1; }
    int fd = open(argv[1], O_RDONLY);
    if (fd < 0) { perror("open"); return 1; }

    off_t sz = lseek(fd, 0, SEEK_END);
    lseek(fd, 0, SEEK_SET);

    /* SEEK_DATA from 0: should return 0 (data starts at beginning) */
    off_t data = lseek(fd, 0, SEEK_DATA);
    /* SEEK_HOLE from 0: should return file size (hole at EOF) */
    off_t hole = lseek(fd, 0, SEEK_HOLE);

    printf("size=%ld data=%ld hole=%ld\n", (long)sz, (long)data, (long)hole);
    if (data == 0 && hole == sz)
        printf("seek_test: PASS\n");
    else
        printf("seek_test: FAIL (expected data=0 hole=%ld)\n", (long)sz);
    close(fd);
    return 0;
}
