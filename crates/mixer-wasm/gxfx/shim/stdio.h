// Freestanding <stdio.h> shim: printf only, used by zita's
// Resampler_table::print_list (a debug dump nothing in the gxfx chain
// calls — an uncalled declaration emits no undefined symbol, like the
// <cstring> shim's unused entries).
#pragma once
typedef __SIZE_TYPE__ size_t;
extern "C" {
int printf(const char*, ...);
}
