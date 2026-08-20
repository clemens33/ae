unmodified_source_sha256=b7b8aa9fb77afc0705abdfaadf60cc58911f1cac46fe2ec993578fe5451575fd
hooked_binary_sha256=7d5a9bda713dc28cd01e7d99dd39eb524a9e6994d7bf7714a551648e980c29f0
patch_sha256=a252a7497280da2c1670f7d08b3eaa65d15b5a4c9369f04fa92f0a3d317b43e0
added_lines=23 removed_lines=1 (the removed line is the _lib emission list, re-added with _ae_hook appended)
hooks=H_LIST_META_CAPTURED H_REQUEST_SCAN_COMPLETE H_NEXT_SELECTED H_NEXT_RECHECKED
contract=a hook's FIRST statement is the guard and returns 0 before any side effect;
  inactive it writes nothing, prints nothing and cannot change an exit status. Active it
  only records that it was reached and BLOCKS until the controller releases it — the
  controller performs the named writer-shaped mutation, never the hook.
lib_emission=_ae_hook is added to the _lib declare -f list because _ar_request_states is
  emitted into the generated requests helper and must find its hook defined there.
