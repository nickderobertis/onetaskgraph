# The `workspace` project

Nx fans targets out **by name**, so a cross-cutting check needs a project to belong to.
This is that project: it owns the checks that are about the repository rather than about
any one deliverable — that the Nx graph still matches the Cargo graph, that no plugin
crate has reached for the engine, that the three affected selections still hold, and that
every project still declares the same uniform target names.

It declares an `implicitDependency` on every other project on purpose. Those checks are
only worth having if they run whenever anything they check can change, and the thing they
check is the shape of the other projects — so a change to any project selects this one.
