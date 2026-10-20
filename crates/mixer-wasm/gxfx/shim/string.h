// Freestanding <string.h> shim: mirrors <cstring> — the byte/str family
// at C-name scope for the zita sources (resampler.cc's inp_count
// bookkeeping calls memcpy; the rest are uncalled declarations).
#pragma once
typedef __SIZE_TYPE__ size_t;
extern "C" {
void* memcpy(void* dest, const void* src, size_t n);
void* memmove(void* dest, const void* src, size_t n);
void* memset(void* dest, int value, size_t n);
int memcmp(const void* a, const void* b, size_t n);
size_t strlen(const char* s);
}
