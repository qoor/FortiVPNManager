// Exported by components/helper-entry/macos (libfvm_helper_entry_macos.a), linked into the Helper.
#pragma once

/// Starts the Helper entry and returns Swift's end of its socketpair, which the caller then owns.
int fvm_helper_entry_start(void);
