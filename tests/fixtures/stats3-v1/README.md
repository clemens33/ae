# Frozen format-1 Statistics store

Captured on the hp from `ea6def36245d44b97627a3a3bc1cfa00645243cf`
before slice 3 implementation, through the real `Maintainer` and `Files` APIs.
The temporary exporter is not part of the product or these tests.
The worker-fact records on journal lines 2, 3, 5, and 6 are hand-authored
to exercise the existing reader before writers existed.

The journal has two certificates. Birth 1 has an attempted start followed by
an exec failure. Birth 2 has an attempted start and an undelivered brief.
Hand totals are two births, one brief failure, one start failure, and one
unproved start. The format-1 view has two births but no outcome columns.

These bytes are historical input. Never regenerate them from the current
serializer. Device, inode, and witness hashes describe the original fixture.
The upgrade test installs the same journal and lets the version change force
the ordinary one-time rebuild.
