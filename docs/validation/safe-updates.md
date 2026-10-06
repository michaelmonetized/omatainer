# Verified Linux releases and rollback

`scripts/release-package.mjs` builds one committed source revision and packages the executable, updater, notices, license records and complete modified vendor sources. Packages record architecture, required glibc, source revision, application version, document schemas and every file's SHA-256 and size. Native application builds stay on `/home` and force a fresh application compilation for that revision.

Verify the package against an independently obtained manifest checksum before installation. Node.js 22 or newer, `flock` and a matching Linux architecture/glibc are required by the update tool. The native application itself does not acquire a Node dependency. This tool does not claim signed publisher authentication or Windows/macOS qualification.

```sh
node scripts/refresh-source-inventory.mjs generate
# Review and commit the source and inventory before building.
node scripts/release-package.mjs build target/releases/RELEASE
node scripts/release-package.mjs verify target/releases/RELEASE EXPECTED_MANIFEST_SHA256
node scripts/release-package.mjs install target/releases/RELEASE EXPECTED_MANIFEST_SHA256
node scripts/release-package.mjs rollback
```

The default destination is `~/.local/bin/omatainer`; an explicit destination is supported for isolated qualification. Installation refuses a running instance using that destination. Existing processes never have their executing inode overwritten. An OS lock serializes installers and releases after a crash. The complete executable is staged and flushed beside its destination, checked again, then published by atomic rename. Hash-addressed backups and durable receipts support rollback and restoration after a failed publication. Preferences, projects, libraries and plugin settings are left intact. Opening documents can migrate their format; document rollback requires their saved backups, independently of executable rollback.

Six native filesystem/process tests pass: clean install/upgrade/rollback, package corruption and unsupported platforms, publication and rollback fault recovery, unsafe destinations, concurrent installers and lock release after process death, and deferral while an app process is alive. These tests use real host ELF executables. Fresh Omatainer package and installation checks are recorded separately in the receipt. The two checklist generators add five checks for inventory coverage, dependencies, source routes, exclusions and acceptance claims.
