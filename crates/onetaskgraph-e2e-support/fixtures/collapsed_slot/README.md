# Collapsed-slot editor states

Captured from team `TES2` by spike `spike-linear-collapsed-slot`, on its branch
`nick/linear-metadata-collapsed-block/spike-linear-collapsed-slot`, and copied here rather than
measured again. Each file is one record's long form as Linear stored it (`markdown`), the base64
Yjs editor state Linear answered the create with (`state`) and that state decoded into the
ProseMirror node tree (`node_tree`): an issue, a project and a document, each ending in the legacy
HTML-comment slot (`*-legacy`) and in the `Fields` section followed by the collapsed metadata block
that opens with its sentinel paragraph (`*-fields-sentinel`).

`src/linear_editor.rs` holds its model of Linear's editor to these files: each `markdown` derives
the recorded `node_tree`, and each `state` decodes to it.
