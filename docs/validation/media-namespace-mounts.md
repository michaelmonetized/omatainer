# Media discovery with namespace mounts

Docker's actual namespace mount on this Linux aarch64 host exposed a parser
failure: mountinfo contains `net:[4026533175]` as the root of an `nsfs` mount.
Rejecting that valid record made the entire inventory unavailable, so local
import, tag reads and media relocation failed even on unrelated filesystems.

The parser accepts bounded relative roots for `nsfs` while requiring absolute
mountpoints and absolute roots for other filesystems. Namespace mounts stay in
the inventory, so searches cannot cross their visible boundaries. They acquire
no block identity. The kernel's [namespace dentry implementation](https://github.com/torvalds/linux/blob/master/fs/nsfs.c)
supplies the symbolic root name.

Local ordinary tests on the corrected source passed: 14 media-location tests,
five relocation-search tests, 13 real media-tag tests and the native relocation
UI test that qualifies the original hash before accepting a search result.
The host inventory test reads the real mountinfo and libudev inventory; the
new boundary regression also checks rejection of non-absolute mountpoints and
non-absolute roots on block filesystems. No mounts or Docker state were changed.

Raw logs are retained under `/home/michael/Projects/omatainer-work/` as
`media-namespace-mounts-tests-v2.log` and
`media-namespace-mounts-dependent-tests.log`.
