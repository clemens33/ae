# Inherited human-projection WIP — disposition

Preserved from the dead `grok46:txreview` seat without modification:

| file | SHA-256 | disposition |
|---|---|---|
| `inherited/human_project.py` | `7bbbe5eddf52dfdeb52abe86635d8ea940b8037bd5412d0a867e8cdad225183d` | rejected |
| `inherited/human-projection.tsv` | `74e64c92b19ff6710043571d4870025d050b64d7353c3f98f534b0e6faf103c7` | rejected as output of rejected comparator |

The candidate's `compare()` returns `layout-open` after accepting an
`SC-017l` directional branch without reconciling membership. Its final return
also labels any unmatched residual after semantic rows as `layout-open`, despite
the fixed projection reserving only header, separators, padding, ANSI SGR, and
line-ending whitespace for that open choice. Thus a changed semantic row or an
unregistered footer can pass as layout.

`wip-redproof.py` constructs a stopped baseline row and a different successor
unknown row under the candidate's declared SC-017l path. The candidate returns
`layout-open`; a fail-closed comparator must reject it. This is a reject proof,
not an acceptance test of the replacement runner.

Run 2 does not import or execute either inherited file. The candidate stays here
as a byte-pinned handoff artifact.
