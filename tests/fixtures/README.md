# Fixture Vaults

Miniature vaults shared by scenario tests, conformance suites, and both frontends.
Each subdirectory is a self-contained vault that `vault-core` can scan and open.

| Fixture | Covers |
| --- | --- |
| `basic/` | Plain notes, folders, subfolder links. |
| `frontmatter/` | Tags (list + string), aliases (list + string), cssclasses, title, invalid YAML, non-mapping YAML, CRLF frontmatter. |
| `unicode-paths/` | Unicode, combining marks, spaces in paths. |
| `duplicate-basenames/` | Same basename in different folders (link ambiguity). |
| `attachments/` | Local images referenced by embeds, missing/remote images, non-image attachments. |
| `embeds/` | Nested embeds and an embed cycle. |
| `crlf-bom/` | CRLF line endings, UTF-8 BOM, mixed line endings. |
| `conflicts/` | Notes intended for external-modification conflict scenarios (dirty reload / keep mine / overwrite). |

Conventions:

- Fixtures are committed as static bytes; tests must copy a fixture to a temporary
  directory before mutating anything.
- The `large-file` scenario is generated at test runtime (a builder writes a note with
  many blocks/headings) rather than committed, to keep the repo small.
- Fixture contents are golden data: changing a fixture requires updating the golden
  expectations in the same commit and stating why.
