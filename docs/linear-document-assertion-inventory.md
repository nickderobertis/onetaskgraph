# Linear live document assertion inventory

The baseline for old locations below is commit `14e32a47`. Paths are relative to the
repository root. `plugin` means `crates/onetaskgraph-linear/tests/live.rs`; `assets`
means `crates/onetaskgraph-linear-e2e/tests/e2e/linear_assets.rs`.

The manager approved moving fresh document creation and document deletion coverage to
one daily fresh pass. Ordinary selected journeys still execute their update, read,
filter, metadata, Markdown and asset assertions. Initialization — explicit, or the cold
rebuild an ordinary CI pass makes of a missing standing slot — is setup, not a
substitute for either pass, and is reported apart from both pass totals. Deleting a document does not restore
Linear's retained-trash capacity.

| Original sender | Ordinary execution | Fresh daily execution | Initialization |
| --- | --- | --- | --- |
| `plugin:1364–1367`, `drive_documents`, filed write | Reset the owned filed slot using its target ID | Original targetless write into the disposable project | Create missing filed slot under retained parent only |
| `plugin:1368–1371`, `drive_documents`, loose write | Reset the owned loose slot using its target ID | Original targetless write without a project | Create missing loose slot only |
| `plugin:1062–1083`, `drive_follow_ups`, metadata probe | Reset the owned follow-up slot using its target ID | Original targetless metadata-bearing write | Create missing follow-up slot only |
| `assets:1191–1203`, live asset document copy | Copy into the owned asset destination | Original first copy creates its destination | Create missing asset destination only |

These are the four successful document-producing operations in an original complete
pass: three plugin documents and one binary destination. `plugin:1153–1174` updates an
existing document. `plugin:1412–1424` deliberately refuses unsupported labels before
transport; it is not another successful create. The asset loop also creates an issue;
issue creation remains in both pass modes and is outside this document inventory.

| Original assertion or asserted operation | New execution mode and location |
| --- | --- |
| `plugin:1364–1371`: both targetless writes succeed | Fresh: `drive_documents`; ordinary instead proves targeted reset succeeds |
| `plugin:1375–1379`: filed document readable by its ID | Ordinary and fresh: `drive_documents` |
| `plugin:1380–1383`: title round trip | Ordinary and fresh: `drive_documents` |
| `plugin:1384–1388`: visible content excludes metadata slot | Ordinary and fresh: `drive_documents` |
| `plugin:1389–1393`: numeric `caller.count` retains JSON type | Ordinary and fresh: `drive_documents` |
| `plugin:1394–1398`: no document labels | Ordinary and fresh: `drive_documents` |
| `plugin:1399–1403`: HTTPS location | Ordinary and fresh: `drive_documents` |
| `plugin:1404–1408`: filed document belongs to expected project | Ordinary and fresh: `drive_documents`; ordinary uses retained parent |
| `plugin:1412–1424`: labeled write refused by name | Ordinary and fresh: `drive_documents` |
| `plugin:1430–1442`: unfiltered listing returns both expected IDs and titles | Ordinary and fresh: `drive_documents`, `settled_documents` |
| `plugin:1443–1455`: project filter returns only filed document | Ordinary and fresh: same helpers |
| `plugin:1456–1468`: orphan filter returns only loose document | Ordinary and fresh: same helpers |
| `plugin:1469–1484`: label filter returns neither document | Ordinary and fresh: same helpers |
| `plugin:1488–1493`: delete both documents successfully and wait for absence | Fresh only: `drive_documents`, `settled_document_absent`; retained slots survive ordinary cleanup |
| `plugin:1062–1083`: initial metadata-bearing document write succeeds | Fresh: `drive_follow_ups`; ordinary uses targeted reset |
| `plugin:1084–1093`: probe readable, metadata and visible content survive | Ordinary and fresh: `drive_follow_ups` |
| `plugin:1094–1105`: raw document HTML-comment normalization matches recorded form | Ordinary and fresh: `drive_follow_ups` |
| `plugin:1129–1140`: raw document Markdown normalization matches recorded form | Ordinary and fresh: `drive_follow_ups` |
| `plugin:1141–1149`: plugin read returns normalized Markdown | Ordinary and fresh: `drive_follow_ups` |
| `plugin:1153–1188`: targeted table write succeeds; content and metadata survive together | Ordinary and fresh: `drive_follow_ups` |
| `assets:1200–1203`: first copy succeeds and reports destination | Ordinary and fresh: live asset copy/recopy driver; fresh exercises destination creation |
| `assets:1204–1210`: destination show succeeds | Ordinary and fresh: same driver |
| `assets:1213–1216`: recoverable ownership title retained | Ordinary and fresh: same driver |
| `assets:1217–1218`: relative asset links replaced | Ordinary and fresh: same driver |
| `assets:1219–1223`: two asset metadata entries; content contains each uploaded URL | Ordinary and fresh: same driver |
| `assets:1224–1232`: HTTPS, uploads.linear.app, port 443, no user info or fragment | Ordinary and fresh: same driver |
| `assets:1233–1240`: authenticated GET succeeds for each image | Ordinary and fresh: same driver; replay serves synthetic bytes |
| `assets:1242–1246`: unchanged recopy succeeds without another fileUpload | Ordinary and fresh: same driver |
| `assets:1252–1269`: journal-driven cleanup confirms successful deletes | Fresh disposable objects and invocation-owned local sets; retained CI destination excluded |
| `assets:1190,1270–1272`: assertion panic is rethrown after cleanup | Both modes: failure cleanup follows ownership rules; every acknowledged create stays counted |

The surrounding follow-up issue assertions remain in both modes: metadata keys and slot
spelling (`plugin:993–1006`), comment creation (`1008–1022`), issue/comment HTML probes
(`1025–1061`), issue Markdown write/read (`1107–1128`), metadata/origin/priority/activity/
case-insensitive content filtering (`1190–1258`), Linear comparator cases (`1260–1299`),
and narrow metadata preservation (`1301–1322`). Existing label, state, issue and project
capability journeys retain their ordinary execution.

Discovery and explicit initialization live in
`crates/onetaskgraph-linear/tests/document_lifecycle/mod.rs`. Its `documentCreate`
sender creates only missing exact owned slots. An ordinary CI pass with an absent or
incomplete standing set reaches that sender for exactly the missing slots, journalled as
a cold rebuild; a non-CI ordinary invocation refuses before it and names the explicit
initialize mode. Uncertain creation acknowledgements require discovery.
The same discovery code supplies both live lanes. Cleanup and orphan sweeping in
`crates/onetaskgraph-linear/tests/cleanup/mod.rs`, asset panic cleanup, and the Linear
janitor delete or preserve according to ownership; none sends `documentCreate`.

The production mutation sender is `graphql::DOCUMENT_CREATE`, selected by the plugin's
`write_document` implementation when the target is absent. The HTTP observers and replay
handlers matching `documentCreate` are receivers, not additional live senders. Other
local fixture document writes exercise offline product behavior and do not reach the
nominated real team. Creation journals count acknowledged HTTP operations at transport,
including creates subsequently deleted or left by a failed pass. Initialization and
failure diagnostics are separate from ordinary and daily pass totals.
