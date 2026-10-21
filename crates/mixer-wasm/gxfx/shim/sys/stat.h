// Freestanding <sys/stat.h> shim: the stat/mkdir surface of the
// livelooper's save_array() (its "$HOME/gxloops" existence check and
// mkdir attempt — the Rust c_floor defines both to fail with -1 under
// the empty wasm environ, which pairs with the always-fail sndfile.hh
// stub: nothing about loop persistence can succeed, everything degrades
// harmlessly). Natively the real header/libc serve unchanged.
#pragma once

struct stat {
    unsigned int st_mode;
};

extern "C" {
int stat(const char* path, struct stat* buf);
int mkdir(const char* path, unsigned int mode);
}

#define S_ISDIR(m) ((((unsigned int)(m)) & 0170000u) == 0040000u)
#define S_IRWXU 0700
#define S_IRWXG 0070
#define S_IRWXO 0007
#define S_IROTH 0004
#define S_IWOTH 0002
#define S_IXOTH 0001
