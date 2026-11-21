// Exported by components/helper/macos (libfvm_helper_macos.a), linked into the Helper.
#pragma once

#include <stddef.h>
#include <stdint.h>

/// Starts the Helper entry and returns Swift's end of its socketpair, which the caller then owns.
int fvm_helper_entry_start(void (*_Nonnull fault)(const uint8_t *_Nonnull text, size_t length));
