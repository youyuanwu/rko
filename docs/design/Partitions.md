# Binding Header Selection

The former bnd-winmd partitions and manually sized injected types have
been replaced by bindgen generation in `rko-sys/build.rs`.

Headers are included through `rko-sys/src/bindings.h` and selected through
the build script's header allowlist. Referenced types are generated
recursively into one flat `rko_sys` namespace, without partition features.

See [Bindings](Bindings.md) for the current design and
[Adding Kernel Bindings](../guides/adding-bindings.md) for extension steps.
