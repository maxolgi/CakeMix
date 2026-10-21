// gxfx shim: redirect for the bare '#include "output12ax7_table.h"' inside
// vendor/guitarix faust-generated gxtubetremelo.cc / gxtubevibrato.cc (the
// real header lives in DSP/circuit_tables/, which build.rs deliberately
// keeps off the include path). Found via the gxfx/ -I entry ahead of
// everything else; content stays the vendor submodule's single copy.
// Deliberately NO include guard (unlike the w20_*/alembic_* shims): the
// vendor headers are guard-less AND both tube classes pull them, so each
// wrapper namespace must embed its own copy of tables + clip functions —
// a guard here (or #pragma once) would silently neuter the second
// namespace's tables.
#include "../../../vendor/guitarix/trunk/src/LV2/DSP/circuit_tables/output12ax7_table.h"
