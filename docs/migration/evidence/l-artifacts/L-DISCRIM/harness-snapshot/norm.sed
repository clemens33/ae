s|/private/tmp/aelx/EQUIV/[A-Za-z0-9._-]*|<ROOT>|g
s|/tmp/aelx/EQUIV/[A-Za-z0-9._-]*|<ROOT>|g
s|/private/tmp/aelx/[A-Za-z0-9._-]*/[A-Za-z0-9._-]*|<ROOT>|g
s|/tmp/aelx/[A-Za-z0-9._-]*/[A-Za-z0-9._-]*|<ROOT>|g
s/[0-9a-fA-F]\{8\}-[0-9a-fA-F]\{4\}-[0-9a-fA-F]\{4\}-[0-9a-fA-F]\{4\}-[0-9a-fA-F]\{12\}/<UUID>/g
s/[0-9]\{4\}-[0-9]\{2\}-[0-9]\{2\}T[0-9]\{2\}:[0-9]\{2\}:[0-9]\{2\}Z/<TS>/g
s/[0-9]\{4\}-[0-9]\{2\}-[0-9]\{2\} [0-9]\{2\}:[0-9]\{2\}:[0-9]\{2\} UTC/<TSU>/g
s/[0-9]\{10\}/<EPOCH>/g
s/"session_id":"[0-9a-f]\{8\}"/"session_id":"<SID8>"/g
s/^\([^|]*|@[0-9]*|%[0-9]*\)|[0-9]*|/\1|<PID>|/
s/[0-9a-f]\{40\}/<SHA1>/g
s/[0-9a-f]\{64\}/<SHA256>/g
s/\[detached HEAD [0-9a-f]\{7,\}\]/[detached HEAD <ABBREV>]/g
s/HEAD is now at [0-9a-f]\{7,\}/HEAD is now at <ABBREV>/g
s/\(\.watchdog\.pid[[:space:]][^[:space:]]*[[:space:]][^[:space:]]*[[:space:]][^[:space:]]*[[:space:]]\)[0-9]*/\1<PIDLEN>/
