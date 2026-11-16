# Arcnix

English | [简体中文](README.zh-CN.md)

A friendly desktop workspace for managing servers over SSH, built with Rust, Glory and Sapid. No remote agent required. Windows is the currently validated desktop platform.

- Organize servers in sortable groups, annotate environments and tags, and import SSH configurations. Searches include labels; batch target and configuration comparison forms also offer exact label filters.
- Servers overview combines time-labelled local diagnostics and current-policy backup confirmations, filters findings and records needing verification, and reviews up to 32 selected servers for a new diagnostic run. It preserves the current workspace and makes no background probes.
- Dual-pane SFTP browsing, transfer queues and file conflict handling.
- Context menus for file and folder actions, with keyboard navigation, properties, and confirmed multi-item or recursive deletion.
- Cancellable deletion with live progress, remote POSIX permissions, and per-server file column preferences.
- Interactive SSH terminals, software management, resource metrics and system logs.
- Service management and a reusable library of saved shell commands.
- On-demand name resolution from selected servers, with optional required IP addresses, saved scope and bounded lookups; accepted diagnostic jobs survive desktop closure.
- TCP listener investigation from diagnostic findings or independent server records, showing addresses, visible processes and observed cgroup service names through a separate connection.
- Reviewed script-only bundle import/export, with copy or explicit replacement, editable sharing copies, new-file publication, and the normal multi-server workflow for imported user scripts.
- Typed script arguments, defaults and local presets: only opted-in fields are remembered, changed definitions require review, and bundles exclude preset values.
- Favorite official and personal scripts in one searchable list; favorites survive restarts and keep the normal multi-server review flow.
- On-demand recovery drills restore directory snapshots or PostgreSQL databases into new destinations, verify explicit file/hash or table-count criteria, and keep separate confirmations and server-owned reports.
- Cross-server recovery browses unavailable origins through another trusted server and restores to reviewed executors outside the original backup policy, without changing backup or retention targets.
- Installation preflight records OS, architecture and existing packages; server-owned checks distinguish package state from template-specific application checks, with reports available after restart in recent tasks and server records. Built-in installation preserves existing packages by default; reviewed upgrades require an explicit choice and the actual dpkg transaction must match the review.
- Server task records and saved log inspection across SSH accounts, independent of recent local batches; record queries require neither SFTP nor sudo.
- Disk findings and saved diagnostic records offer a finite deleted-open-file inspection, with deduplicated allocated/logical sizes, observed holders and explicit coverage limits. Reads preserve the current workspace, require Linux/Python, and offer optional administrator access.
- Private configuration recovery records, current-to-original diffs, record storage summaries, and reviewed restoration after local history has been evicted.
- Global task and configuration storage inventory, exact previews with retention/reference protection, and separately reviewed server-owned cleanup.
- Private cleanup reports across servers, with saved phases, live service states and residual file metadata independent of local batch history.
- Audit and residual export storage capacity, cross-page retention previews, and separately reviewed server-owned retirement of eligible cleanup audits; exported copies and cleanup evidence remain protected.
- Separately reviewed server-owned residual exports and quarantine removal, with full data and metadata checksum verification before source deletion.
- Per-server default and remembered directories, connection overrides and transfer conflict policies.
- Remote UTF-8 text editing with change detection, shared size/date formats and rotating client logs.
- Windows x64 release checks and verified automatic updates, with idle restart and startup rollback.
- Reviewed local configuration backup and restore, preserving the previous file and this computer's SSH trust.
- English and Chinese UI, with system (default), dark and light themes.
- Passwords, key passphrases and sudo passwords are never saved to configuration.

## Run

Building requires **Rust 1.99+**, Windows MSVC build tools, native Windows Perl (such as Strawberry Perl) and NASM. Running requires WebView2 Runtime. OpenSSL is linked statically; system SSH and OpenSSL are not required.

```powershell
cargo run --locked -p arcnix --bin arcnix
```

To use a specific configuration, append `-- --config ./my-config.json`.

## Configuration backup

Use **Settings → General → Configuration backup** to export a reviewed JSON snapshot or restore one. It includes saved servers, scripts, favorites, opted-in argument presets, backup policies and confirmations, preferences, and file-path references. Keys, referenced file contents, session credentials, task logs, unsaved publication drafts and server data stay separate. Saved text can contain sensitive values; keep the readable backup private.

Restore previews the replacement, retains this computer's current SSH trust, and preserves the exact previous file beside the active configuration. Imported host keys do not grant trust. Active operations must finish first; a successful restore disconnects the workspace, keeps recent task history and unsaved publication drafts separate, and can repair a corrupt startup configuration without restarting first. Changed source files or settings require a new review. Close other configuration editors during restore.

## Operations

After connecting, open **Operations**. **Services** lists loaded systemd service units, including stopped and failed units. Select a service to inspect its state or open its journal. Start, stop and restart open the shared server-task form with the current server selected. Add other targets if needed, review each preflight plan, then submit. Refresh the service list to inspect the resulting state.

**Commands** lets you create, edit, delete and run saved multiline shell commands. Commands are stored in local configuration and shared across server profiles; do not embed passwords or tokens. Plain and parameterized commands use the same reviewed server-task flow. **Tasks** retains logs, exit results and recent batch history across desktop restarts. Commands use a noninteractive POSIX shell; interactive programs belong in Terminal.

Software installation and package-index updates also enter this shared flow. Submission requires server systemd, root or sudo, and flock; ordinary command workloads retain the SSH UID. Independent task connections may require entering credentials again. Optional sudo passwords pass through stdin and are never stored in configuration. Disconnecting the workspace, switching servers or closing Arcnix does not stop accepted jobs. Query unknown results before retrying. Prepare the required server tools before using saved tasks on systems without them; Terminal remains available for manual commands.

## Preferences and updates

**Settings** contains display formats, connection timeout/keepalive, separate upload/download conflict defaults, client log export/rotation and updates. Edit a server to set its own directories and connection overrides. File actions expose per-server transfer defaults. Right-click a regular remote text file and choose **Edit file**; drafts stay in memory, preserve UTF-8 BOM/CRLF and are checked against the remote contents before saving. The editor supports files up to 1 MiB.

Version information and updates live in **Settings → About**. **General** contains language and theme; **Files** owns browsing and size/date formats. **Logging** separates the operation log from optional file logging.

Packaged Windows x64 builds check stable releases daily by default. Verified updates download in the background, then offer a 30-second restart countdown only after SSH disconnects, tasks finish and dialogs close; **Later** defers installation for the session. Installation retains the previous executable and restores it if the new process fails to acknowledge startup. GitHub HTTPS release metadata and its SHA-256 asset digest authorize the download; this is not publisher code signing. Missing assets/digests are never installed. Development builds pause background updates; manual checks remain available. See the [update channel requirements](docs/releasing.md#desktop-update-channel) before publishing.

## Development and releases

CI checks workflows, Rustfmt, typos, Clippy, tests, documentation and crate packaging. Matching `v*` tags additionally build GitHub Release archives for macOS ARM64, Windows AMD64, Linux ARM64 and AMD64, with SHA-256 checksums. crates.io publication is currently disabled; see the release guide to enable it.

[Development guide](docs/development.md) · [Release setup](docs/releasing.md) · [Changelog](CHANGELOG.md)

Licensed under Apache-2.0. Bundled third-party assets retain their own licenses.


### Global scripts and server-owned tasks

Use **Scripts** without selecting or connecting to a server. Browse the built-in APT installation and diagnostic templates, create your own shell scripts, or copy an official template. Standalone unquoted `{{parameter}}` placeholders become literal shell arguments in the run form. Select up to 32 trusted servers and review each preflight plan before submitting; preflight and submission parallelism are bounded to 1–8 connections. An accepted server job releases its submission slot immediately, so later targets do not wait for its runtime to finish. Running jobs can exceed the submission limit.

The shared task form searches names, addresses, accounts, ports and groups, filters groups, and adds visible trusted targets without discarding earlier selections. Counts expose hidden selections; **Review selected servers** must reveal them before preflight. Filtering retains in-memory credentials, while deselection and cancellation discard them. Bulk additions are atomic and reject more than 32 targets or multiple accounts at the same endpoint. Backup policies and reviewed cleanup/recovery sources keep their original target scope; filtering never grants trust or broadens that scope.

Jobs run under the remote systemd manager and require root or sudo to submit. Non-administrator scripts run as the original SSH UID. Logs and exit results remain in the SSH user's `$HOME/.local/state/arcnix/tasks/<task-id>/` after Arcnix closes. **Tasks** keeps the latest 10 batches beside the configuration file and supports reconnecting to query original server results without resubmitting work. Desktop-dependent scheduling and continuous monitoring are outside this release.

Expand a recent batch and use **Query batch results…** to read several original targets in one action, with 1–8 parallel queries. Each target needs its original address, account and currently trusted host fingerprint; unsubmitted or already-querying targets are unavailable. Query credentials stay in memory, and a failed target does not prevent other results from loading. These are single reads: they neither rerun tasks nor start continuous monitoring.

**Submitting** means the server has not yet acknowledged receipt; closing Arcnix may interrupt that submission. **Running** appears after a successful server launch acknowledgement. Unconfirmed submissions become unknown after a desktop restart and must be queried by their original task ID before retrying.

Global jobs that may change server state also require `flock`. A server lock excludes overlapping jobs across desktop restarts, SSH accounts and clients, including jobs entered through Software and Operations; preflight checks availability, and actual execution acquires the lock again. A conflicting job exits without running its payload. On-demand Diagnostics can run alongside it. The service holds the lock independently of Arcnix; lock availability does not confirm the outcome of an interrupted task. This guard does not cover older jobs submitted before the feature.


### Backups and recovery

**Backups** manages on-demand directory backup policies across servers. Install restic on each server, prepare a repository password file with mode 400 or 600, then initialize the repository. Policy settings reference server paths rather than storing repository passwords. Local paths, SFTP locations and HTTPS S3 locations are accepted; remote backend credentials must already be configured on the server.

Backup, full repository verification, retention and restore use server-owned tasks. Snapshots are filtered by policy and source-server tags. Restores pin a full snapshot ID, require a new destination outside the source and repository (including resolved symlinks), and verify restored contents. Retention is previewed before execution and does not prune repository data. Regular file backups do not establish consistency for running databases. Scheduling does not depend on the desktop application and is outside this increment.

**Browse snapshots → Browse files** lists directory metadata in pages of 100, with parent/path navigation and selection across directories. Select regular files or directories, then review a new restore task on one or more servers. Selected paths are literal, including wildcard characters; directory selection includes descendants. Preflight and execution both verify snapshot/source tags and selected-path existence before creating a destination. Successful confirmations distinguish a full snapshot from selected paths and retain the source-server ID. Browsing and selected restores require server-side Python 3.8+; the page links to its installer. Reads require no SFTP, retain access only in browser memory, and clear it on close, navigation or explicit forgetting. Failed reads retain the old page but disable restoration until a successful query. No scheduler or resident desktop is added.


Database policies use native logical exports instead of copying live data directories. PostgreSQL exports use custom archives and support restoring to a new database with a separate restore role. MySQL/MariaDB exports require InnoDB and a direct schema-wide or global SELECT grant; SQL exports are recovered as files for a controlled import. Database secrets remain in server-side pgpass/client option files. These are single-database exports, not cluster/PITR backups.

Backup, integrity-check, file-restore and database-restore confirmations are stored separately from the rolling task list. They retain repository/snapshot/destination and original server identity, distinguish the latest attempt from the last success, and mark evidence from older settings. Failed attempts and late queries do not erase newer successful observations.

### Connected resource trends

The server workspace **Resources** page shows CPU, memory and network trends for the last 1, 5 or 15 minutes of reads. Inspect samples by pointer or keyboard, pause automatic sampling, or clear the local history. Missing or failed reads remain gaps. History stays in this connection’s memory, up to 15 minutes and 1,024 reads; leaving Resources stops new sampling, and connection changes or closing Arcnix discard it. Details are available through `?`; no background monitoring is added.

### On-demand diagnostics

Add **TLS endpoints** to check the certificate actually served to each selected server. Separate the connect host/port from an optional verification name (DNS names also set SNI). OpenSSL verifies the leaf, identity, chain and validity against that server's CA store, then checks the expiry threshold. Network failures stay inconclusive; this is a direct TLS snapshot, separate from HTTP health or certificate renewal.

**Diagnostics** compares selected checks across trusted servers: filesystem space and inodes, failed systemd services, expected local TCP listeners, first-certificate PEM expiry, and Nginx/OpenSSH configuration validation. Add multiple certificate and configuration paths (up to 20 checks in total), choose thresholds, review each target, then submit server-owned tasks. Configuration validation does not reload services. Results retain the selected checks and parameters, show each file separately, distinguish findings from inconclusive checks, and link to directory inspection and the server workspace. Reopen **Tasks** to query an unfinished run after closing Arcnix. This is an on-demand snapshot; local listeners do not establish network reachability or application health.

### Compare current configurations

**Configurations → Compare current files** reads the same absolute path from 2–32 trusted servers over independent SSH connections. Choose a baseline to inspect content differences and ownership/mode differences. Reads need Linux, Python 3.8+ and access to `/proc/self/fd`, without SFTP or systemd; administrator access is optional and explicit. Files are limited to 128 KiB of UTF-8 text. Failed, changed, missing and stopped reads cannot become matching results; an older snapshot stays labelled and excluded from comparison. Details are available through `?`.

Snapshots stay in page memory and are cleared on navigation. Known authentication secrets are hidden, but other file secrets may remain visible. A complete, unhidden baseline can fill an empty publication draft; validation, reload, health checks and per-server review remain required. Reading or copying a draft never publishes it, and no desktop background service is added.

### Configuration publication

**Configurations** prepares changes across trusted servers. Upload a private UTF-8 candidate, review each server's diff and metadata, then publish a full Nginx configuration or use explicit validation, reload and health commands for another application. SFTP, Python 3.8+, root/sudo and systemd are required. Content and diffs stay out of local task history; command settings, hashes and results are retained. Server records under `/var/lib/arcnix/configurations/<task-id>` hold the original, candidate, private logs and a versioned manifest.

Server tasks recheck the reviewed file version, validate again, publish a synced sibling atomically, reload and check health. Failures attempt to restore the previous file and separately confirm service recovery. Explicit restore creates a fresh reviewed change from a recorded original, including recovery of a deleted destination. Cooperating publications use a per-path server lock; observed intervening edits are refused. This is one-file recovery, not a transaction over arbitrary command side effects or referenced configuration files. Accepted jobs continue after the client closes; a failure stops submissions that have not started, while already accepted jobs manage their own recovery.

Enable **Use configuration parameters** in a publication draft to insert `{{name}}` in candidate contents. Set defaults, select servers, then expand **Configuration values…** only where an override is needed. Each candidate and digest is frozen before preflight; review each server’s diff before execution. Values are literal single-line text, expanded once, without escaping application syntax. Destination and validation/reload/health settings remain shared. Parameters stay in session memory; history keeps each target’s recovery settings and digest without storing parameter values. A draft blocks manual and automatic restart installation until you review or explicitly discard it; discarding the draft retains server jobs and versions. Cancelling review returns to the draft, and deselection discards that server’s overrides.

**Configurations → Server recovery records** reads private records across trusted servers without relying on local batches. Inspect saved phases, bounded private logs, current-to-original diffs and record-directory allocation. Reads require Python 3.8+ and root/sudo, without SFTP, and never execute saved validation or health commands. Restoration opens a new authenticated task review for the original server and repeats the server checks; active or unverified sources and unavailable originals are refused. Reading does not create local history or a health confirmation. Credentials are cleared when leaving Configurations or explicitly forgotten; in-flight reads may finish later.

### Record storage

**Tasks → Record storage** inspects multiple server accounts independently of the workspace, with an entry from configuration recovery records. Compare allocated and logical bytes, retain recent records, and select up to 100 entries across pages for exact file/directory previews. Defaults keep seven days, the newest ten task records, and three verified configuration originals per destination path. Active, uncertain, referenced, incomplete and unsafe records remain protected; unavailable configuration references protect all candidates. Python 3.8+ and root/sudo are required, without SFTP. Queries and previews never delete records, truncate logs or stop services. Credentials and previews stay in page memory; leaving Tasks, changing views or forgetting credentials prevents new queries.

**Review cleanup on servers** takes verified candidates into a separate authentication, preflight and exact-file review. Explicitly submit the reviewed plans as server-owned systemd jobs. Execution rechecks file identities, service states and recovery references under operation, registry and configuration-path locks. Records move into a private quarantine and their directories are frozen before individual verified files are unlinked; no recursive deletion runs. Deletion has no undo. Interrupted or partially failed jobs may retain private quarantines and durable reports for manual inspection, without automatic replay. Accepted jobs survive desktop closure. Execution additionally requires Linux `renameat2`; query the original task when its result is unknown.

**Tasks → Cleanup reports** reads private server reports even after local batches are gone. Browse multiple servers independently, inspect saved removal intent alongside current metadata at original and quarantined locations, and keep active or conflicting service states visible. Missing files do not imply confirmed deletion. New cleanup jobs retain directory identity before moving; older reports without this evidence remain unproven. Queries read metadata on demand without running frozen scripts, reading configuration contents or resuming jobs.

Stopped, proven quarantines can enter **Review residual exports** with fresh authentication, per-server preflight and explicit submission. Server jobs copy remaining bytes into a new private `/var/lib/arcnix/record-recoveries/<task>/files` directory and save original ownership, permissions, timestamps and extended attributes as separate metadata, retaining the source. Select a completed export for a separate **Review quarantine removal**: execution verifies every exported data and metadata SHA-256 before removing exactly the reviewed source files and empty directories. Interruptions retain remaining source files and exported copies; tasks cannot replay, but a fresh inspected plan can handle the remainder. Previously deleted files cannot be recovered. Export does not restore live paths or apply original permissions. A completed export can be separately verified and downloaded as a ZIP containing data and metadata; checksums and the completion receipt are checked before saving a new local file, and the server copy is retained. Accepted operations are systemd-owned without SFTP or a resident desktop. Audit/export storage supports capacity inspection and exact retention previews; proven, unreferenced cleanup audits can be separately reviewed for removal. Export copies and audit-cleanup evidence remain protected.
