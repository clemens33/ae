# Vendored front-end files of `ae web`

Three files from two npm packages, embedded with `include_bytes!` and served from `/assets/`. No build step, no Node.
A version bump is reviewed like a new crate: new tarball, new SHA-256 values here and in `tests/it/web_assets_spec.rs`.

| Served file | Package | Tarball | Tarball SHA-256 | Path in tarball | Licence |
|---|---|---|---|---|---|
| preact.mjs | preact 11.2.0 | https://registry.npmjs.org/preact/-/preact-11.2.0.tgz | 3c4456766bd32b6b534d52bf4e9c2a29b33e7eedf6e96ed479300ba1a7888ebf | package/dist/preact.mjs | MIT |
| hooks.mjs | preact 11.2.0 | same tarball | same | package/hooks/dist/hooks.mjs | MIT |
| htm.mjs | htm 3.1.1 | https://registry.npmjs.org/htm/-/htm-3.1.1.tgz | 2425b9bee11409177bcabc7f32e319926fc6690c1701c0b257c88bdff2d5ba90 | package/dist/htm.mjs | Apache-2.0 |

## File SHA-256

| Served file | Bytes | Upstream SHA-256 | Served SHA-256 |
|---|---|---|---|
| preact.mjs | 11970 | bdcbd9abdf2c77c5c45326eab42191e949222d479d0d5f69467387a77040cb55 | identical |
| hooks.mjs | 3634 (upstream 3628) | e862ab2973aa6455eaa0f77c5d832d2157d6be59a17c8ba546550a575b58c6b4 | 427a330c992595bfd3f6cfe43eb6ba7a238ea1a7fa99c0e4a8f380a30dde473b |
| htm.mjs | 1207 | ab33dd3f38059b9be4d5f5350128eefb2356639c4e0bbe9d9e8b3ba75847e9e4 | identical |

## The one rewrite

Upstream `hooks.mjs` imports the bare specifier `from"preact"`. A browser resolves a bare specifier only through an
import map, and the page's CSP (`script-src 'self'`) refuses an inline one. The served file therefore differs from
upstream in exactly one place: `from"preact"` became `from"./preact.mjs"`, which resolves to `/assets/preact.mjs`, the
same module instance the page imports. Reversing that one replacement gives the upstream SHA-256 above; the asset spec
checks both directions.

## Licences

Preact: MIT, Copyright (c) 2015-present Jason Miller. htm: Apache License 2.0, author Jason Miller (its package.json; the LICENSE names no holder). Neither
tarball carries a NOTICE file. Both licence texts are vendored byte for byte from the tarballs above, embedded with
`include_bytes!` and served together at `/assets/licenses.txt`, each under a heading naming its package, version and
tarball.

| Vendored file | Path in tarball | Bytes | SHA-256 |
|---|---|---|---|
| LICENSE-preact.txt | preact 11.2.0 package/LICENSE | 1087 | 1fe6958409c8c257a70c587a18b6f7f412b179b456630790d30b2ec9a8e4b7d4 |
| LICENSE-htm.txt | htm 3.1.1 package/LICENSE | 11341 | 740725f7252e750af735d0028cc534970772f513331e9f68150fede8fb3ce00f |
