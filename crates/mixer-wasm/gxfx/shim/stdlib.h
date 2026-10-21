// Freestanding <stdlib.h> shim: mirrors <cstdlib> — the allocation pair
// the gxfx global operator new/delete overloads route to (the zita
// sources include it but call no libc function from it; their allocations
// are C++ new/delete, resolved by gxfx_dsp_eleven.cpp at archive link)
// and getenv (livelooper wave paths; NULL under the wasm c_floor).
#pragma once
#define NULL __null
typedef __SIZE_TYPE__ size_t;
extern "C" {
void* malloc(size_t size);
void free(void* ptr);
char* getenv(const char* name);
}
