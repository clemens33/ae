# Marks-pass queues (machine-derived)

- Snapshot: contract sources read from `git show HEAD` at `ba116fe3d11b6637e9ccf6e6bf1fb4c08cd88a84`; worktree dirty: yes (non-contract changes present).
- Scope: SC rows only. Ownership D records are excluded from all queues: 43 canonical D records (D01–D31 with letter splits); their TBD-field closures remain in evidence batches.
- Canonical SC rows: 425; raw qualifications Q1=148, Q2=35, Q3=122; Q2∩Q3=15.
- Emitted queues after precedence Q2 > Q3 > Q1: Q1=148, Q2=35, Q3=107; already-closed=133; no-queue anomalies=2.
- Reconciliation: 148 Q1 + 35 Q2 + 107 Q3 + 133 already-closed + 2 no-queue = 425 canonical SC rows. The 15 dual Q2/Q3 rows are resolved to Q2 and are not added twice. The canonical delta from the stale 424-row snapshot is SC-1106 (added in the current contract).
- Manifest cross-check (orthogonal criticality labels, not queue criteria): ratification-critical.md at this snapshot reports CRITICAL=324 + DEFERRABLE=103 + OBSERVED=41 = 468 total IDs (425 SC + 43 D).
- Derivation: Q1 requires an explicit docs/ruling anchor in Authority, conflict=none, and no per-row or block classified_by mark. Q2 includes every Bucket 3/4 row and preserves its fix-known-defect issue or DR reference, regardless of mark status. Q3 requires Authority beginning with code-observation, Authority beginning with UNRESOLVED, or an explicit UNCLASSIFIED conflict; each emitted Q3 row uses its CRIT-ASSIGN batch. Rows are read at row-paragraph grain; family/range marks are expanded only to their declared exact IDs.
- Mark states: `block-marked` = S1 preflight, exact S6 frozen set, S9 SC-800..831, or S13 SC-1200..1209; `per-row-marked` = explicit row classified_by (including S1MAP/ratification-day marks and the S3 delivery/routing and helper-signature blocks); otherwise `genuinely-unmarked`.
- Anomalies: no-queue=SC-211k, SC-211m; dual Q2/Q3=SC-920, SC-921, SC-926, SC-927, SC-928, SC-929, SC-958, SC-963, SC-964, SC-965, SC-966, SC-967, SC-968, SC-969, SC-976a; Q3 rows without a CRIT-ASSIGN line=SC-832b, SC-832c, SC-833b, SC-833c, SC-833d, SC-834b, SC-834c.

## Q1 — strong normative authority, conflict=none, genuinely unmarked

id | bucket | exact claim-bearing authority anchor | conflict enum | batch (Q3 only) | annotation
SC-100 | 1 | AGENTS.md | none | — | genuinely-unmarked
SC-011 | 2 | commands.md | none | — | genuinely-unmarked
SC-012 | 2 | commands.md | none | — | genuinely-unmarked
SC-018 | 2 | commands.md:5 | none | — | genuinely-unmarked
SC-019 | 2 | commands.md:10-11 | none | — | genuinely-unmarked
SC-211o | 2 | commands.md:711-723 + DR-005 | none | — | genuinely-unmarked
SC-211p | 2 | AGENTS.md | none | — | genuinely-unmarked
SC-300a | 2 | AGENTS.md | none | — | genuinely-unmarked
SC-300b | 2 | config.md + telegram.md | none | — | genuinely-unmarked
SC-301 | 2 | config.md | none | — | genuinely-unmarked
SC-302 | 2 | config.md | none | — | genuinely-unmarked
SC-303 | 2 | #32 | none | — | genuinely-unmarked
SC-304 | 2 | config.md:3 | none | — | genuinely-unmarked
SC-305 | 2 | config.md | none | — | genuinely-unmarked
SC-306 | 2 | config.md | none | — | genuinely-unmarked
SC-400a | 2 | architecture.md + AGENTS.md | none | — | genuinely-unmarked
SC-401a | 2 | architecture.md:77-83 | none | — | genuinely-unmarked
SC-402 | 1 | AGENTS.md | none | — | genuinely-unmarked
SC-403 | 1 | AGENTS.md | none | — | genuinely-unmarked
SC-600 | 1 | AGENTS.md | none | — | genuinely-unmarked
SC-601 | 1 | AGENTS.md | none | — | genuinely-unmarked
SC-602 | 2 | helpers.md | none | — | genuinely-unmarked
SC-700 | 1 | AGENTS.md | none | — | genuinely-unmarked
SC-701 | 1 | AGENTS.md | none | — | genuinely-unmarked
SC-702 | 1 | AGENTS.md | none | — | genuinely-unmarked
SC-703 | 2 | AGENTS.md | none | — | genuinely-unmarked
SC-704 | 1 | #81 + #79 | none | — | genuinely-unmarked
SC-704a | 1 | S8 joint adapter ruling (SC-704 frame) | none | — | genuinely-unmarked
SC-704e | 1 | S8 joint adapter ruling (SC-704 frame) + SC-811 pins context | none | — | genuinely-unmarked
SC-804a | 1 | architecture.md:99-104 | none | — | genuinely-unmarked
SC-804b | 1 | architecture.md:100 | none | — | genuinely-unmarked
SC-804c | 1 | architecture.md:100-101 | none | — | genuinely-unmarked
SC-804f | 1 | architecture.md:100-101 | none | — | genuinely-unmarked
SC-804d | 1 | architecture.md:101-103 | none | — | genuinely-unmarked
SC-804e | 1 | architecture.md:103-104 | none | — | genuinely-unmarked
SC-806a | 1 | architecture.md:81-83 | none | — | genuinely-unmarked
SC-806b | 2 | architecture.md:81-82 | none | — | genuinely-unmarked
SC-810a | 2 | AGENTS.md + architecture.md:131-133 | none | — | genuinely-unmarked
SC-810b | 2 | architecture.md:131-133 | none | — | genuinely-unmarked
SC-811a | 2 | AGENTS.md | none | — | genuinely-unmarked
SC-811b | 2 | AGENTS.md | none | — | genuinely-unmarked
SC-815a | 1 | commands.md:382-386 | none | — | genuinely-unmarked
SC-815b | 1 | commands.md:386-389 | none | — | genuinely-unmarked
SC-815c | 1 | commands.md:389-390 | none | — | genuinely-unmarked
SC-815d | 2 | commands.md:389-390 | none | — | genuinely-unmarked
SC-818a | 1 | commands.md:534-535 + architecture.md:134-137 | none | — | genuinely-unmarked
SC-818b | 1 | commands.md:534-536 | none | — | genuinely-unmarked
SC-818c | 1 | commands.md:536-542 | none | — | genuinely-unmarked
SC-818d | 1 | commands.md:537-540 + architecture.md:134-137 | none | — | genuinely-unmarked
SC-818e | 1 | architecture.md:137-138 | none | — | genuinely-unmarked
SC-820a | 1 | commands.md:526-532 + architecture.md:146-149 | none | — | genuinely-unmarked
SC-820b | 2 | commands.md:526-532 | none | — | genuinely-unmarked
SC-821a | 1 | architecture.md:150-155 | none | — | genuinely-unmarked
SC-821b | 1 | architecture.md:150-155 | none | — | genuinely-unmarked
SC-824a | 1 | commands.md:589-592 | none | — | genuinely-unmarked
SC-824b | 1 | commands.md:589-592 | none | — | genuinely-unmarked
SC-825a | 2 | commands.md:594-598 | none | — | genuinely-unmarked
SC-825b | 2 | commands.md:594-598 | none | — | genuinely-unmarked
SC-825c | 2 | commands.md:594-598 | none | — | genuinely-unmarked
SC-829a | 1 | architecture.md:193-199 | none | — | genuinely-unmarked
SC-829b | 1 | architecture.md:198-201 | none | — | genuinely-unmarked
SC-835a | 1 | commands.md:297-305 | none | — | genuinely-unmarked
SC-835b | 1 | commands.md:297-300 | none | — | genuinely-unmarked
SC-835c | 1 | commands.md:297-302 | none | — | genuinely-unmarked
SC-835d | 1 | commands.md:301-302 | none | — | genuinely-unmarked
SC-835e | 1 | commands.md:311-321 | none | — | genuinely-unmarked
SC-835g | 1 | commands.md:311-325 | none | — | genuinely-unmarked
SC-835h | 1 | commands.md:325-330 | none | — | genuinely-unmarked
SC-835f | 2 | commands.md:333-334 | none | — | genuinely-unmarked
SC-838a | 2 | commands.md:459-465 | none | — | genuinely-unmarked
SC-838b | 2 | commands.md:453-458 | none | — | genuinely-unmarked
SC-839a | 1 | commands.md:417-421 | none | — | genuinely-unmarked
SC-839b | 1 | commands.md:423-430 | none | — | genuinely-unmarked
SC-839c | 1 | commands.md:431-434 | none | — | genuinely-unmarked
SC-839d | 1 | commands.md:430-434 | none | — | genuinely-unmarked
SC-839e | 1 | commands.md:408-414 | none | — | genuinely-unmarked
SC-836 | 1 | commands.md:651-652 | none | — | genuinely-unmarked
SC-837 | 2 | commands.md:698 | none | — | genuinely-unmarked
SC-832a | 2 | commands.md:287-290 | none | — | genuinely-unmarked
SC-833a | 2 | commands.md:24 | none | — | genuinely-unmarked
SC-834a | 2 | commands.md:713-715 | none | — | genuinely-unmarked
SC-902 | 2 | watchdog.md:5-10 + commands.md:177-185 | none | — | genuinely-unmarked
SC-903 | 2 | watchdog.md:7-10 | none | — | genuinely-unmarked
SC-904 | 2 | commands.md:177-185 + watchdog.md:7-10 | none | — | genuinely-unmarked
SC-905 | 1 | watchdog.md:48-70 | none | — | genuinely-unmarked
SC-906 | 1 | watchdog.md:70-73 | none | — | genuinely-unmarked
SC-907 | 1 | watchdog.md:73 | none | — | genuinely-unmarked
SC-908 | 1 | watchdog.md:73 | none | — | genuinely-unmarked
SC-909 | 1 | watchdog.md:91-98 | none | — | genuinely-unmarked
SC-910 | 1 | watchdog.md:74-75 | none | — | genuinely-unmarked
SC-911 | 1 | watchdog.md:75-76 | none | — | genuinely-unmarked
SC-912 | 1 | watchdog.md:76-77 | none | — | genuinely-unmarked
SC-914 | 1 | watchdog.md:77-78 | none | — | genuinely-unmarked
SC-915 | 1 | watchdog.md:104-121 | none | — | genuinely-unmarked
SC-916 | 1 | watchdog.md:116-121 | none | — | genuinely-unmarked
SC-917 | 1 | watchdog.md:116-136 | none | — | genuinely-unmarked
SC-918 | 1 | watchdog.md:116-136 | none | — | genuinely-unmarked
SC-919 | 1 | watchdog.md:80-83 | none | — | genuinely-unmarked
SC-922 | 2 | monitor.md:1-40 | none | — | genuinely-unmarked
SC-923 | 1 | monitor.md:5-12 | none | — | genuinely-unmarked
SC-924 | 2 | monitor.md:34-40 + DR-002 | none | — | genuinely-unmarked
SC-925 | 1 | watchdog.md:138-143 | none | — | genuinely-unmarked
SC-930 | 2 | commands.md:219-249 | none | — | genuinely-unmarked
SC-931 | 2 | commands.md:233-249 | none | — | genuinely-unmarked
SC-932 | 1 | commands.md:233-238 | none | — | genuinely-unmarked
SC-933 | 1 | commands.md:240-246 | none | — | genuinely-unmarked
SC-934 | 1 | commands.md:221-231 | none | — | genuinely-unmarked
SC-935 | 1 | commands.md:187-195 | none | — | genuinely-unmarked
SC-936 | 1 | commands.md:197-200 | none | — | genuinely-unmarked
SC-937 | 1 | commands.md:197-203 | none | — | genuinely-unmarked
SC-938 | 1 | commands.md:201-203 | none | — | genuinely-unmarked
SC-939a | 1 | commands.md:203-206 | none | — | genuinely-unmarked
SC-939b | 1 | commands.md:208-215 | none | — | genuinely-unmarked
SC-939c | 2 | commands.md:215-217 + telegram.md:138-140 | none | — | genuinely-unmarked
SC-939d | 2 | telegram.md:69-77 | none | — | genuinely-unmarked
SC-939e | 2 | telegram.md:73-77 | none | — | genuinely-unmarked
SC-939f | 2 | commands.md:264-272 + #52 | none | — | genuinely-unmarked
SC-940 | 1 | telegram.md@72c7293:19-24 | none | — | genuinely-unmarked
SC-1000 | 2 | install.md | none | — | genuinely-unmarked
SC-1001 | 2 | install.md | none | — | genuinely-unmarked
SC-1002 | 2 | install.md + commands.md:168 | none | — | genuinely-unmarked
SC-1003 | 1 | AGENTS.md | none | — | genuinely-unmarked
SC-1004 | 1 | AGENTS.md | none | — | genuinely-unmarked
SC-1100 | 1 | AGENTS.md | none | — | genuinely-unmarked
SC-1101b | 2 | AGENTS.md | none | — | genuinely-unmarked
SC-1102 | 2 | AGENTS.md | none | — | genuinely-unmarked
SC-1103 | 1 | S12 seat ruling 2026-08-20 (semantic limit-or-loud) | none | — | genuinely-unmarked
SC-1104 | 1 | AGENTS.md | none | — | genuinely-unmarked
SC-1105 | 2 | AGENTS.md | none | — | genuinely-unmarked
SC-1205a | 1 | #59 | none | — | genuinely-unmarked
SC-1205b | 2 | #59 | none | — | genuinely-unmarked
SC-1207a | 1 | #59 | none | — | genuinely-unmarked
SC-1207b | 2 | #59 | none | — | genuinely-unmarked
SC-1300 | 1 | events.md + bridge-protocol.md | none | — | genuinely-unmarked
SC-1400 | 2 | config.md + watchdog.md:35 | none | — | genuinely-unmarked
SC-1401 | 2 | config.md + watchdog.md:36 | none | — | genuinely-unmarked
SC-1402 | 2 | config.md + watchdog.md:37 | none | — | genuinely-unmarked
SC-1403 | 2 | config.md + watchdog.md:38 | none | — | genuinely-unmarked
SC-1404a | 2 | config.md + watchdog.md:41 | none | — | genuinely-unmarked
SC-1404b | 2 | config.md + watchdog.md:41 | none | — | genuinely-unmarked
SC-1405a | 2 | config.md + watchdog.md:42 | none | — | genuinely-unmarked
SC-1405b | 2 | config.md + watchdog.md:42 | none | — | genuinely-unmarked
SC-1406a | 2 | config.md + watchdog.md:43 | none | — | genuinely-unmarked
SC-1406b | 2 | config.md + watchdog.md:43 | none | — | genuinely-unmarked
SC-1407a | 2 | config.md + watchdog.md:44 | none | — | genuinely-unmarked
SC-1407b | 2 | config.md | none | — | genuinely-unmarked
SC-1408a | 2 | config.md | none | — | genuinely-unmarked
SC-1408b | 2 | config.md | none | — | genuinely-unmarked

## Q2 — bucket 3/4 baseline audit

id | bucket | exact claim-bearing authority anchor | conflict enum | batch (Q3 only) | annotation
SC-200 | 4 | DR-004 | DR-004 | — | genuinely-unmarked
SC-204 | 4 | helpers.md + DR-004 | DR-004 | — | genuinely-unmarked
SC-210 | 4 | helpers.md + DR-004 | DR-004 | — | genuinely-unmarked
SC-400b | 4 | DR-001 | DR-001 | — | genuinely-unmarked
SC-400c | 4 | DR-006 + #79 + #76 | DR-006 | — | genuinely-unmarked
SC-401b | 4 | DR-001 | DR-001 | — | genuinely-unmarked
SC-704b | 3 | DR-005 | fix-known-defect(#56) | — | genuinely-unmarked
SC-704c | 4 | DR-005 + #50 | DR-005 | — | genuinely-unmarked
SC-704d | 4 | DR-005 | DR-005 | — | genuinely-unmarked
SC-705 | 3 | S8 joint seat ruling (2026-08-20) grounded in the #46/#30 transported-fact rulings | fix-known-defect(#94) | — | per-row-marked
SC-706 | 3 | #30-family ruling (commit 32719f5) + AGENTS.md | fix-known-defect(#94) | — | per-row-marked
SC-900 | 4 | DR-001 | DR-001 | — | genuinely-unmarked
SC-901 | 4 | DR-002 | DR-002 | — | genuinely-unmarked
SC-913 | 3 | watchdog.md:77-78 + #44 | fix-known-defect(#45) | — | genuinely-unmarked
SC-920 | 3 | UNRESOLVED(memo s10-watchdog gives no normative authority citation) | fix-known-defect(#51) | — | genuinely-unmarked; dual-Q3 batch=T-WD
SC-921 | 3 | UNRESOLVED(memo s10-watchdog gives no normative authority citation) | fix-known-defect(#73) | — | genuinely-unmarked; dual-Q3 batch=T-WD
SC-926 | 3 | UNRESOLVED(memo supplies ownership D26a/census-2 evidence but no normative authority citation) | fix-known-defect(#88-A) | — | genuinely-unmarked; dual-Q3 batch=T-WD
SC-927 | 3 | UNRESOLVED(memo supplies ownership D26b/census-2 evidence but no normative authority citation) | fix-known-defect(#88-B) | — | genuinely-unmarked; dual-Q3 batch=T-WD
SC-928 | 3 | UNRESOLVED(memo supplies census-3 I2 evidence but no normative authority citation) | fix-known-defect(#88-C) | — | genuinely-unmarked; dual-Q3 batch=T-WD
SC-929 | 4 | UNRESOLVED(no SC-929 authority citation in requested S10 memos) | DR-002 | — | genuinely-unmarked; dual-Q3 batch=T-WD
SC-958 | 4 | UNRESOLVED(memo gives line ranges 9-12,167-169,181-185 and census3 I8 but no normative source citation) | DR-003 | — | genuinely-unmarked; dual-Q3 batch=T-STORE
SC-963 | 3 | UNRESOLVED(memo gives issue-evidence line range 181-198 without a frozen normative source citation) | fix-known-defect(#83) | — | genuinely-unmarked; dual-Q3 batch=T-STORE
SC-964 | 3 | DR-002 | fix-known-defect(#84) | — | genuinely-unmarked; dual-Q3 batch=T-STORE
SC-965 | 3 | #85 | fix-known-defect(#85) | — | genuinely-unmarked; dual-Q3 batch=T-STORE
SC-966 | 3 | UNRESOLVED(memo s10-telegram gives no normative authority citation) | fix-known-defect(#86-E) | — | genuinely-unmarked; dual-Q3 batch=T-STORE
SC-967 | 3 | UNRESOLVED(memo s10-telegram gives no normative authority citation) | fix-known-defect(#87) | — | genuinely-unmarked; dual-Q3 batch=T-STORE
SC-968 | 3 | UNRESOLVED(memo s10-telegram gives no normative authority citation) | fix-known-defect(#88-G) | — | genuinely-unmarked; dual-Q3 batch=T-STORE
SC-969 | 3 | UNRESOLVED(memo s10-telegram gives no normative authority citation) | fix-known-defect(#87-H) | — | genuinely-unmarked; dual-Q3 batch=T-STORE
SC-976a | 4 | UNRESOLVED(no SC-976a authority citation in requested S10 memos) | DR-001 | — | genuinely-unmarked; dual-Q3 batch=T-STORE
SC-1006 | 3 | install.md + #57 | fix-known-defect(#57) | — | genuinely-unmarked
SC-1101a | 3 | AGENTS.md + #75 | fix-known-defect(#75) | — | genuinely-unmarked
SC-1106 | 3 | AGENTS.md TSV-framing + interpreted-sinks direction (ruling) | fix-known-defect(#95) | — | per-row-marked
SC-1202 | 3 | AGENTS.md + #59 | fix-known-defect(#61) | — | block-marked
SC-1301 | 3 | architecture.md:158-166 | fix-known-defect(#88-I) | — | genuinely-unmarked
SC-1302 | 3 | architecture.md | fix-known-defect(#75) | — | genuinely-unmarked

## Q3 — code-observation / UNRESOLVED / UNCLASSIFIED

id | bucket | exact claim-bearing authority anchor | conflict enum | batch (Q3 only) | annotation
SC-101 | — | code-observation | UNCLASSIFIED | C | genuinely-unmarked
SC-102a | — | code-observation | UNCLASSIFIED | C | genuinely-unmarked
SC-102b | — | code-observation | UNCLASSIFIED | C | genuinely-unmarked
SC-013 | — | code-observation | UNCLASSIFIED | H-HELPER | genuinely-unmarked
SC-018b | — | code-observation | UNCLASSIFIED | C | genuinely-unmarked
SC-014 | — | code-observation | UNCLASSIFIED | H-HELPER | genuinely-unmarked
SC-211a | — | code-observation | UNCLASSIFIED | H-HELPER | genuinely-unmarked
SC-211b | — | code-observation | UNCLASSIFIED | H-HELPER | genuinely-unmarked
SC-211c | — | code-observation | UNCLASSIFIED | H-HELPER | genuinely-unmarked
SC-211d | — | code-observation | UNCLASSIFIED | H-HELPER | genuinely-unmarked
SC-211e | — | code-observation | UNCLASSIFIED | H-HELPER | genuinely-unmarked
SC-211f | — | code-observation | UNCLASSIFIED | H-HELPER | genuinely-unmarked
SC-211g | — | code-observation | UNCLASSIFIED | H-HELPER | genuinely-unmarked
SC-211h | — | code-observation | UNCLASSIFIED | H-HELPER | genuinely-unmarked
SC-211i | — | code-observation | UNCLASSIFIED | H-HELPER | genuinely-unmarked
SC-211j | — | code-observation | UNCLASSIFIED | H-HELPER | genuinely-unmarked
SC-211l | — | code-observation | UNCLASSIFIED | H-HELPER | genuinely-unmarked
SC-211n | — | code-observation | UNCLASSIFIED | H-HELPER | genuinely-unmarked
SC-300c | — | code-observation | UNCLASSIFIED | F-CONFIG | genuinely-unmarked
SC-307 | — | code-observation | UNCLASSIFIED | F-CONFIG | genuinely-unmarked
SC-405e | — | code-observation | UNCLASSIFIED | C | genuinely-unmarked
SC-508 | — | code-observation | UNCLASSIFIED | L-COMPACT | genuinely-unmarked
SC-603 | — | code-observation | UNCLASSIFIED | F-TMUX | genuinely-unmarked
SC-604 | — | code-observation | UNCLASSIFIED | F-TMUX | genuinely-unmarked
SC-707 | — | code-observation | UNCLASSIFIED | F-ADAPTER | genuinely-unmarked
SC-832b | — | code-observation | UNCLASSIFIED | MISSING-CRIT-ASSIGN | genuinely-unmarked; anomaly: no CRIT-ASSIGN line
SC-832c | — | code-observation | UNCLASSIFIED | MISSING-CRIT-ASSIGN | genuinely-unmarked; anomaly: no CRIT-ASSIGN line
SC-833b | — | code-observation | — | MISSING-CRIT-ASSIGN | genuinely-unmarked; anomaly: no CRIT-ASSIGN line
SC-833c | — | code-observation | — | MISSING-CRIT-ASSIGN | genuinely-unmarked; anomaly: no CRIT-ASSIGN line
SC-833d | — | code-observation | — | MISSING-CRIT-ASSIGN | genuinely-unmarked; anomaly: no CRIT-ASSIGN line
SC-834b | — | code-observation | — | MISSING-CRIT-ASSIGN | genuinely-unmarked; anomaly: no CRIT-ASSIGN line
SC-834c | — | code-observation | — | MISSING-CRIT-ASSIGN | genuinely-unmarked; anomaly: no CRIT-ASSIGN line
SC-941 | 2 | UNRESOLVED(memo citation is only the unqualified line range 47-53) | none | T-CTRL | genuinely-unmarked
SC-942 | 2 | UNRESOLVED(memo citation is only the unqualified line range 9-15) | none | T-CTRL | genuinely-unmarked
SC-943 | 1 | UNRESOLVED(memo citation is only the unqualified line range 51,55-57) | none | T-AUTH | genuinely-unmarked
SC-944a | 1 | UNRESOLVED(memo citation is only the unqualified line range 59-65) | none | T-AUTH | genuinely-unmarked
SC-944b | 1 | UNRESOLVED(memo citation is only the unqualified line range 59-65) | none | T-AUTH | genuinely-unmarked
SC-944c | 1 | UNRESOLVED(memo citation is only the unqualified line range 59-65) | none | T-AUTH | genuinely-unmarked
SC-945 | 2 | UNRESOLVED(memo citation is only the unqualified line range 67-77) | none | T-AUTH | genuinely-unmarked
SC-946 | 1 | UNRESOLVED(memo citation is only the unqualified line references 69,77) | none | T-AUTH | genuinely-unmarked
SC-947 | 1 | UNRESOLVED(memo citation is only the unqualified line reference 91) | none | T-AUTH | genuinely-unmarked
SC-948 | 2 | UNRESOLVED(memo citation is only the unqualified line reference 91) | none | T-AUTH | genuinely-unmarked
SC-949 | 1 | UNRESOLVED(memo citation is only the unqualified line reference 92) | none | T-AUTH | genuinely-unmarked
SC-950 | 2 | UNRESOLVED(memo citation is only the unqualified line reference 93) | none | T-AUTH | genuinely-unmarked
SC-951 | 1 | UNRESOLVED(memo citation is only the unqualified line reference 97) | none | T-AUTH | genuinely-unmarked
SC-952 | 2 | UNRESOLVED(memo citation is only the unqualified line reference 95) | none | T-AUTH | genuinely-unmarked
SC-953 | 2 | UNRESOLVED(memo citation is only the unqualified line reference 155) | none | T-STORE | genuinely-unmarked
SC-954 | 2 | UNRESOLVED(memo citation is only the unqualified line reference 155) | none | T-STORE | genuinely-unmarked
SC-955 | 2 | UNRESOLVED(memo citation is only the unqualified line range 148-155) | none | T-STORE | genuinely-unmarked
SC-956 | 1 | UNRESOLVED(memo citation is only the unqualified line range 161-167) | none | T-STORE | genuinely-unmarked
SC-957 | 1 | UNRESOLVED(memo citation is only the unqualified line range 163-171) | none | T-STORE | genuinely-unmarked
SC-959 | 2 | UNRESOLVED(memo citation is only the unqualified line range 167-169) | none | T-STORE | genuinely-unmarked
SC-960 | 1 | UNRESOLVED(memo gives unqualified line references 97,169) | none | T-AUTH | genuinely-unmarked
SC-961 | 1 | UNRESOLVED(memo gives only unqualified line references 35,210,216-220) | none | T-AUTH | genuinely-unmarked
SC-962 | 1 | UNRESOLVED(memo citation is only the unqualified line reference 212) | none | T-AUTH | genuinely-unmarked
SC-970 | 2 | UNRESOLVED(memo citation is only the unqualified line range 27-53) | none | T-STORE | genuinely-unmarked
SC-971 | 2 | UNRESOLVED(memo citation is only the unqualified line range 148-165) | none | T-STORE | genuinely-unmarked
SC-972 | 2 | UNRESOLVED(no SC-972 authority citation in requested S10 memos) | none | H-DELIVERY | genuinely-unmarked
SC-973a | 1 | UNRESOLVED(no SC-973a authority citation in requested S10 memos) | none | H-DELIVERY | genuinely-unmarked
SC-973b | 1 | UNRESOLVED(no SC-973b authority citation in requested S10 memos) | none | H-DELIVERY | genuinely-unmarked
SC-974a | 2 | UNRESOLVED(no SC-974a authority citation in requested S10 memos) | none | H-DELIVERY | genuinely-unmarked
SC-974b | 2 | UNRESOLVED(no SC-974b authority citation in requested S10 memos) | none | H-DELIVERY | genuinely-unmarked
SC-975a | 1 | UNRESOLVED(no SC-975a authority citation in requested S10 memos) | none | T-STORE | genuinely-unmarked
SC-975b | 1 | UNRESOLVED(no SC-975b authority citation in requested S10 memos) | none | T-STORE | genuinely-unmarked
SC-976b | 2 | UNRESOLVED(no SC-976b authority citation in requested S10 memos) | none | T-STORE | genuinely-unmarked
SC-977 | 1 | UNRESOLVED(no SC-977 authority citation in requested S10 memos) | none | T-STORE | genuinely-unmarked
SC-978a | 2 | UNRESOLVED(no SC-978a authority citation in requested S10 memos) | none | T-STORE | genuinely-unmarked
SC-978b | 2 | UNRESOLVED(no SC-978b authority citation in requested S10 memos) | none | T-STORE | genuinely-unmarked
SC-979a | 1 | UNRESOLVED(no SC-979a authority citation in requested S10 memos) | none | T-STORE | genuinely-unmarked
SC-979b | 1 | UNRESOLVED(no SC-979b authority citation in requested S10 memos) | none | T-STORE | genuinely-unmarked
SC-1005 | — | code-observation | UNCLASSIFIED | F-INSTALL | genuinely-unmarked
SC-1303 | — | code-observation | UNCLASSIFIED | L-RENTRANS | genuinely-unmarked
SC-1304a | — | code-observation | UNCLASSIFIED | L-RENTRANS | genuinely-unmarked
SC-1304b | — | code-observation | UNCLASSIFIED | L-RENTRANS | genuinely-unmarked
SC-1304c | — | code-observation | UNCLASSIFIED | L-RENTRANS | genuinely-unmarked
SC-1304d | — | code-observation | UNCLASSIFIED | L-RENTRANS | genuinely-unmarked
SC-1305 | — | code-observation | UNCLASSIFIED | L-COMPACT | genuinely-unmarked
SC-1306a | — | code-observation | UNCLASSIFIED | C | genuinely-unmarked
SC-1306b | — | code-observation | UNCLASSIFIED | C | genuinely-unmarked
SC-1306c | — | code-observation | UNCLASSIFIED | C | genuinely-unmarked
SC-1306d | — | code-observation | UNCLASSIFIED | C | genuinely-unmarked
SC-1306e | — | code-observation | UNCLASSIFIED | C | genuinely-unmarked
SC-1409a | — | code-observation | UNCLASSIFIED | H-ENV | genuinely-unmarked
SC-1409b | — | code-observation | UNCLASSIFIED | H-ENV | genuinely-unmarked
SC-1409c | — | code-observation | — | H-ENV | genuinely-unmarked
SC-1410a | — | code-observation | UNCLASSIFIED | H-ENV | genuinely-unmarked
SC-1410b | — | code-observation | UNCLASSIFIED | H-ENV | genuinely-unmarked
SC-1410c | — | code-observation | UNCLASSIFIED | H-ENV | genuinely-unmarked
SC-1410d | — | code-observation | UNCLASSIFIED | H-ENV | genuinely-unmarked
SC-1410e | — | code-observation | UNCLASSIFIED | H-ENV | genuinely-unmarked
SC-1410f | — | code-observation | UNCLASSIFIED | H-ENV | genuinely-unmarked
SC-1410g | — | code-observation | UNCLASSIFIED | H-ENV | genuinely-unmarked
SC-1410h | — | code-observation | UNCLASSIFIED | H-ENV | genuinely-unmarked
SC-1410i | — | code-observation | UNCLASSIFIED | H-ENV | genuinely-unmarked
SC-1410j | — | code-observation | UNCLASSIFIED | H-ENV | genuinely-unmarked
SC-1410k | — | code-observation | UNCLASSIFIED | H-ENV | genuinely-unmarked
SC-1410l | — | code-observation | UNCLASSIFIED | H-ENV | genuinely-unmarked
SC-1411a | — | code-observation | UNCLASSIFIED | H-ENV | genuinely-unmarked
SC-1411b | — | code-observation | UNCLASSIFIED | H-ENV | genuinely-unmarked
SC-1411c | — | code-observation | UNCLASSIFIED | H-ENV | genuinely-unmarked
SC-1412a | — | code-observation | UNCLASSIFIED | H-ENV | genuinely-unmarked
SC-1412b | — | code-observation | UNCLASSIFIED | H-ENV | genuinely-unmarked
SC-1412c | — | code-observation | UNCLASSIFIED | H-ENV | genuinely-unmarked
SC-1412d | — | code-observation | UNCLASSIFIED | H-ENV | genuinely-unmarked
SC-1412e | — | code-observation | UNCLASSIFIED | H-ENV | genuinely-unmarked
SC-1412f | — | code-observation | UNCLASSIFIED | H-ENV | genuinely-unmarked
SC-1412g | — | code-observation | — | H-ENV | genuinely-unmarked

## Already closed — classified_by provenance

source mark block | ruling | ids
S1 preflight | ae-20260820T115449Z-1b7ef041 | SC-016a, SC-016b, SC-016c, SC-016d, SC-017a, SC-017b, SC-017c, SC-017d, SC-017e, SC-017f, SC-017g, SC-017h, SC-017i, SC-020a, SC-020b, SC-020c
S1MAP/ratification-day | 2026-08-20 | SC-021, SC-022
S5 per-row marks | 2026-08-20 | SC-404, SC-405a, SC-405b, SC-405c, SC-405d, SC-405f, SC-405g, SC-405i, SC-405j, SC-405k
S6 exact frozen set | 76722eb/f4e93ef | SC-500, SC-501, SC-502, SC-503a, SC-503b, SC-504a, SC-504b, SC-505a, SC-505b, SC-506, SC-507a, SC-507b, SC-507c, SC-507d, SC-509, SC-510a, SC-510b, SC-510c, SC-510d, SC-511a, SC-511b, SC-511c, SC-512, SC-513a, SC-513b, SC-513c, SC-514, SC-515a, SC-515b, SC-515c, SC-516, SC-517a, SC-517b, SC-517c
S6 per-row marks | 2026-08-20 | SC-509b, SC-510e, SC-510f, SC-518, SC-519, SC-520, SC-521a, SC-521b, SC-522, SC-523a, SC-523b, SC-524
S9 exact frozen set | 7398f6de | SC-800, SC-801, SC-802, SC-803, SC-805, SC-807, SC-808, SC-809, SC-812, SC-813, SC-814, SC-816, SC-817, SC-819, SC-822, SC-823, SC-826, SC-827, SC-828, SC-830, SC-831
S12 per-row mark | 2026-08-20 | SC-980
S13 exact frozen set | 07e2770 | SC-1200, SC-1201, SC-1203, SC-1204, SC-1206, SC-1208, SC-1209
S3 delivery/routing MARK batch 1 | ae-20260820T163523Z-e935697d | SC-201, SC-202, SC-203, SC-205, SC-206, SC-207, SC-208, SC-209a, SC-209b, SC-209c, SC-209d
S3 helper-signature MARK batch 2A | ae-20260820T164044Z-f958a368 | SC-212a, SC-212b, SC-212c, SC-212d, SC-212e, SC-212f, SC-212g, SC-212h, SC-212i, SC-212j, SC-212k, SC-212l, SC-212m, SC-212n, SC-212o, SC-212p, SC-212q, SC-212r, SC-212s
