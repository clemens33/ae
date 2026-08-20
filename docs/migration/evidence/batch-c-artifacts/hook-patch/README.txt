unmodified_source=/frozen ae at 72c7293  sha256=b7b8aa9fb77afc0705abdfaadf60cc58911f1cac46fe2ec993578fe5451575fd
hooked_binary_sha256=2235e6b9f2505fdc1f4d1deaedadf05838e597b36f2c5a87971fdff635b2ab9d
patch_sha256=994f73e05334db61b087ddbca616806371207a75d282346cc1861cf2f191ea6a
added_lines=22 removed_lines=0 modified_lines=0
hooks=H_LIST_META_CAPTURED H_REQUEST_SCAN_COMPLETE H_NEXT_SELECTED H_NEXT_RECHECKED
contract=a hook's FIRST statement is the guard and returns 0 before any side effect;
  inactive it writes nothing, prints nothing and cannot change an exit status. Active it
  only records that it was reached and BLOCKS until the controller releases it — the
  controller performs the named writer-shaped mutation, never the hook.
