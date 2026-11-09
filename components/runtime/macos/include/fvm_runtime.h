// Exported by components/runtime (libfvm_runtime.a), linked into the app.
#pragma once

/// Starts the runtime and returns Swift's end of its socketpair, which the caller then owns.
int fvm_runtime_start(void);
