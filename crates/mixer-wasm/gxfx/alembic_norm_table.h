// gxfx shim: redirect for the bare '#include "alembic_norm_table.h"' inside
// vendor/guitarix faust-generated gx_studiopre_st.cc / gx_w20.cc (the real
// header lives in DSP/circuit_tables/, which build.rs deliberately keeps
// off the include path). Found via the gxfx/ -I entry ahead of everything
// else; content stays the vendor submodule's single copy.
#pragma once
#include "../../../vendor/guitarix/trunk/src/LV2/DSP/circuit_tables/alembic_norm_table.h"
