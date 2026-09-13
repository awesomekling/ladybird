/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <LibSandbox/Sandbox.h>
#include <LibSandbox/Seccomp.h>
#include <LibTest/TestCase.h>
#include <errno.h>
#include <fcntl.h>
#include <sys/stat.h>
#include <sys/syscall.h>
#include <sys/wait.h>
#include <unistd.h>

#if defined(__NR_newfstatat) && defined(__NR_fstat)
TEST_CASE(fstatat_queries_descriptors_without_allowing_path_queries)
{
    int pipe_fds[2];
    VERIFY(pipe(pipe_fds) == 0);
    struct stat expected_metadata {};
    VERIFY(syscall(__NR_fstat, pipe_fds[0], &expected_metadata) == 0);

    auto child = fork();
    VERIFY(child >= 0);
    if (child == 0) {
        MUST(Sandbox::install_no_new_privileges());
        Sandbox::SeccompPolicy policy;
        policy.deny_readonly_filesystem_probes();
        policy.allow_file_descriptor_operations();
        policy.allow_common_runtime();
        MUST(policy.install());

        struct stat metadata {};
        for (u64 i = 0; i < 2; ++i) {
            errno = EDOM;
            VERIFY(syscall(__NR_newfstatat, pipe_fds[0], "", &metadata, AT_EMPTY_PATH) == 0);
            VERIFY(errno == EDOM);
            VERIFY(metadata.st_ino == expected_metadata.st_ino);
            VERIFY(metadata.st_dev == expected_metadata.st_dev);
            VERIFY(metadata.st_mode == expected_metadata.st_mode);
        }

        VERIFY(syscall(__NR_newfstatat, pipe_fds[0], nullptr, &metadata, AT_EMPTY_PATH) == 0);
        VERIFY(metadata.st_ino == expected_metadata.st_ino);
        VERIFY(syscall(__NR_newfstatat, AT_FDCWD, "", &metadata, AT_EMPTY_PATH) == -1);
        VERIFY(errno == EBADF);
        VERIFY(syscall(__NR_newfstatat, -1, "", &metadata, AT_EMPTY_PATH) == -1);
        VERIFY(errno == EBADF);
        VERIFY(syscall(__NR_newfstatat, pipe_fds[0], "", nullptr, AT_EMPTY_PATH) == -1);
        VERIFY(errno == EFAULT);
        VERIFY(syscall(__NR_newfstatat, AT_FDCWD, "/", &metadata, AT_EMPTY_PATH) == -1);
        VERIFY(errno == EACCES);
        VERIFY(syscall(__NR_newfstatat, AT_FDCWD, ".", &metadata, AT_EMPTY_PATH) == -1);
        VERIFY(errno == EACCES);
        VERIFY(syscall(__NR_newfstatat, AT_FDCWD, "/", &metadata, 0) == -1);
        VERIFY(errno == EACCES);
        VERIFY(syscall(__NR_newfstatat, pipe_fds[0], "", &metadata, AT_EMPTY_PATH | AT_SYMLINK_NOFOLLOW) == -1);
        VERIFY(errno == EACCES);
        VERIFY(close(pipe_fds[0]) == 0);
        VERIFY(close(pipe_fds[1]) == 0);
        _exit(0);
    }

    VERIFY(close(pipe_fds[0]) == 0);
    VERIFY(close(pipe_fds[1]) == 0);
    int status = 0;
    VERIFY(waitpid(child, &status, 0) == child);
    EXPECT(WIFEXITED(status));
    if (WIFEXITED(status))
        EXPECT_EQ(WEXITSTATUS(status), 0);
}
#endif
